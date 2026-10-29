use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use ringbuf::{
    HeapRb,
    traits::{Consumer, Producer, Split},
};

use crate::audio::resampler::LinearResampler;
use crate::demux::wav_stream::WavStreamReader;
use std::{
    error::Error,
    io,
    path::Path,
    sync::{
        Arc,
        atomic::{AtomicU8, AtomicU64, Ordering},
    },
    thread::{self, JoinHandle},
    time::Duration,
};

const SOURCE_CHUNK_FRAMES: usize = 4096;

const MAX_CHANNELS: usize = 32;
const BUFFER_FRAMES: usize = 2048;

const PAUSED: u8 = 0;
const PLAYING: u8 = 1;
const STOPPED: u8 = 2;
const ENDED: u8 = 3;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AudioEngineState {
    Paused,
    Playing,
    Stopped,
    Ended,
}

impl AudioEngineState {
    fn from_raw(value: u8) -> Self {
        match value {
            PLAYING => Self::Playing,
            STOPPED => Self::Stopped,
            ENDED => Self::Ended,
            _ => Self::Paused,
        }
    }
}

fn pop_current_frame(
    consumer: &mut ringbuf::HeapCons<OutputFrame>,
    generation: &AtomicU64,
) -> (Option<OutputFrame>, u64) {
    loop {
        let current_generation = generation.load(Ordering::Acquire);
        match consumer.try_pop() {
            Some(frame)
                if frame.generation == current_generation
                    && generation.load(Ordering::Acquire) == current_generation =>
            {
                return (Some(frame), current_generation);
            }
            Some(_) => continue,
            None => return (None, current_generation),
        }
    }
}

fn start_playback(state: &AtomicU8) {
    if state.load(Ordering::Acquire) == PAUSED {
        state.store(PLAYING, Ordering::Release);
    }
}

fn update_state_after_seek(state: &AtomicU8, target: u64, duration: u64) {
    if target >= duration {
        state.store(ENDED, Ordering::Release);
    } else if state.load(Ordering::Acquire) == ENDED {
        state.store(PAUSED, Ordering::Release);
    }
}

#[derive(Clone, Copy)]
struct OutputFrame {
    samples: [f32; MAX_CHANNELS],
    generation: u64,
    output_index: u64,
}

struct ResampledOutputQueue<'a> {
    producer: ringbuf::HeapProd<OutputFrame>,
    source_channels: usize,
    output_channels: usize,
    generation: u64,
    duration_frames: u64,
    state: &'a AtomicU8,
    seek_generation: &'a AtomicU64,
}

impl ResampledOutputQueue<'_> {
    fn enqueue(&mut self, samples: &[f32], output_index: &mut u64) -> bool {
        if self.source_channels == 0 || self.output_channels == 0 {
            return false;
        }

        for source_frame in samples.chunks_exact(self.source_channels) {
            if *output_index >= self.duration_frames {
                break;
            }

            let mut frame = OutputFrame {
                samples: [0.0; MAX_CHANNELS],
                generation: self.generation,
                output_index: *output_index,
            };

            if self.output_channels == 1 && self.source_channels > 1 {
                let sum: f32 = source_frame.iter().sum();
                frame.samples[0] = sum / self.source_channels as f32;
            } else {
                for out_channel in 0..self.output_channels {
                    let source_channel = if self.source_channels == 1 {
                        0
                    } else {
                        out_channel.min(self.source_channels - 1)
                    };

                    frame.samples[out_channel] = source_frame[source_channel];
                }
            }

            loop {
                if self.state.load(Ordering::Acquire) == STOPPED {
                    return false;
                }

                if self.seek_generation.load(Ordering::Acquire) != self.generation {
                    return false;
                }

                match self.producer.try_push(frame) {
                    Ok(()) => break,
                    Err(_) => thread::sleep(Duration::from_millis(1)),
                }
            }

            *output_index += 1;
        }

        true
    }
}

pub struct AudioEngine {
    stream: Option<cpal::Stream>,
    state: Arc<AtomicU8>,
    position_frames: Arc<AtomicU64>,
    seek_generation: Arc<AtomicU64>,
    seek_target_frame: Arc<AtomicU64>,
    finished_generation: Arc<AtomicU64>,
    duration_frames: u64,
    output_rate: u32,
    worker: Option<JoinHandle<()>>,
}

impl AudioEngine {
    pub fn open(path: impl AsRef<Path>) -> Result<Self, Box<dyn Error>> {
        let reader = WavStreamReader::open(path)?;
        let source_info = *reader.info();

        let source_rate = source_info.sample_rate;
        let source_channels = source_info.channels as usize;
        let source_frames = reader.total_frames();
        if source_frames == 0 {
            return Err(io::Error::other("WAV file contains no PCM frames").into());
        }
        let host = cpal::default_host();
        let device = host
            .default_output_device()
            .ok_or_else(|| io::Error::other("No audio output device found"))?;

        let supported = device.default_output_config()?;

        if supported.sample_format() != cpal::SampleFormat::F32 {
            return Err(io::Error::other("This engine currently requires F32 audio output").into());
        }

        let output_rate = supported.sample_rate().0;
        let output_channels = supported.channels() as usize;

        if output_rate == 0 || output_channels == 0 {
            return Err(io::Error::other("Invalid audio output configuration").into());
        }

        if output_channels > MAX_CHANNELS {
            return Err(io::Error::other("Output device has too many audio channels").into());
        }

        let config: cpal::StreamConfig = supported.into();

        let state = Arc::new(AtomicU8::new(PAUSED));
        let position_frames = Arc::new(AtomicU64::new(0));
        let seek_generation = Arc::new(AtomicU64::new(0));
        let seek_target_frame = Arc::new(AtomicU64::new(0));
        let finished_generation = Arc::new(AtomicU64::new(u64::MAX));
        let duration_frames = if source_frames == 0 {
            0
        } else {
            ((source_frames as u128 * output_rate as u128) / source_rate as u128).max(1) as u64
        };

        let ring = HeapRb::<OutputFrame>::new(BUFFER_FRAMES);
        let (producer, mut consumer) = ring.split();

        let callback_state = Arc::clone(&state);
        let callback_position = Arc::clone(&position_frames);
        let callback_generation = Arc::clone(&seek_generation);
        let callback_finished = Arc::clone(&finished_generation);

        let stream = device.build_output_stream(
            &config,
            move |output: &mut [f32], _| {
                if callback_state.load(Ordering::Acquire) != PLAYING {
                    output.fill(0.0);
                    return;
                }

                for output_frame in output.chunks_mut(output_channels) {
                    if output_frame.len() != output_channels {
                        output_frame.fill(0.0);
                        continue;
                    }

                    let (next_frame, current_generation) =
                        pop_current_frame(&mut consumer, &callback_generation);

                    match next_frame {
                        Some(frame) => {
                            output_frame.copy_from_slice(&frame.samples[..output_channels]);

                            if callback_generation.load(Ordering::Acquire) == current_generation {
                                callback_position.store(frame.output_index + 1, Ordering::Relaxed);
                            } else {
                                output_frame.fill(0.0);
                            }
                        }
                        None => {
                            output_frame.fill(0.0);
                            if callback_generation.load(Ordering::Acquire) == current_generation
                                && callback_finished.load(Ordering::Acquire) == current_generation
                            {
                                let _ = callback_state.compare_exchange(
                                    PLAYING,
                                    ENDED,
                                    Ordering::AcqRel,
                                    Ordering::Acquire,
                                );
                                break;
                            }
                        }
                    }
                }
            },
            |err| eprintln!("OpenWave audio output error: {err}"),
            None,
        )?;

        stream.play()?;

        let worker_state = Arc::clone(&state);
        let worker_generation = Arc::clone(&seek_generation);
        let worker_target = Arc::clone(&seek_target_frame);
        let worker_finished = Arc::clone(&finished_generation);

        let worker = thread::spawn(move || {
            let mut reader = reader;
            let mut current_generation = u64::MAX;
            let mut output_index = 0_u64;
            let mut resampler: Option<LinearResampler> = None;
            let mut output_queue = ResampledOutputQueue {
                producer,
                source_channels,
                output_channels,
                generation: current_generation,
                duration_frames,
                state: &worker_state,
                seek_generation: &worker_generation,
            };

            loop {
                if worker_state.load(Ordering::Acquire) == STOPPED {
                    return;
                }

                let requested_generation = worker_generation.load(Ordering::Acquire);

                // A seek invalidates both the previous resampler and queued output.
                if requested_generation != current_generation {
                    let target_output = worker_target.load(Ordering::Relaxed).min(duration_frames);

                    let target_source =
                        (target_output as u128 * source_rate as u128 / output_rate as u128) as u64;

                    if let Err(error) = reader.seek_frame(target_source) {
                        eprintln!("OpenWave: seek failed: {error}");
                        worker_finished.store(requested_generation, Ordering::Release);
                        worker_state.store(ENDED, Ordering::Release);
                        current_generation = requested_generation;
                        resampler = None;
                        continue;
                    }

                    match LinearResampler::new_at(
                        source_rate,
                        output_rate,
                        source_channels,
                        target_source,
                        target_output,
                    ) {
                        Ok(value) => resampler = Some(value),
                        Err(error) => {
                            eprintln!("OpenWave: resampler initialization failed: {error}");
                            worker_finished.store(requested_generation, Ordering::Release);
                            worker_state.store(ENDED, Ordering::Release);
                            current_generation = requested_generation;
                            continue;
                        }
                    }

                    current_generation = requested_generation;
                    output_index = target_output;
                    output_queue.generation = current_generation;

                    if target_output >= duration_frames {
                        worker_finished.store(current_generation, Ordering::Release);
                        continue;
                    }
                }

                // Wait for a new seek after EOF instead of repeatedly reading EOF.
                if worker_finished.load(Ordering::Acquire) == current_generation {
                    thread::sleep(Duration::from_millis(2));
                    continue;
                }

                if output_index >= duration_frames {
                    worker_finished.store(current_generation, Ordering::Release);
                    continue;
                }

                let input = match reader.read_frames(SOURCE_CHUNK_FRAMES) {
                    Ok(data) => data,
                    Err(error) => {
                        eprintln!("OpenWave: PCM read failed: {error}");
                        worker_finished.store(current_generation, Ordering::Release);
                        worker_state.store(ENDED, Ordering::Release);
                        continue;
                    }
                };

                // EOF: flush the interpolation tail before signalling completion.
                if input.is_empty() {
                    let tail = match resampler.as_mut() {
                        Some(value) => value.finish(),
                        None => Vec::new(),
                    };

                    let queued = output_queue.enqueue(&tail, &mut output_index);

                    if !queued {
                        continue;
                    }

                    worker_finished.store(current_generation, Ordering::Release);
                    continue;
                }

                let output = match resampler.as_mut() {
                    Some(value) => match value.push(&input) {
                        Ok(samples) => samples,
                        Err(error) => {
                            eprintln!("OpenWave: resampling failed: {error}");
                            worker_finished.store(current_generation, Ordering::Release);
                            worker_state.store(ENDED, Ordering::Release);
                            continue;
                        }
                    },
                    None => continue,
                };

                let queued = output_queue.enqueue(&output, &mut output_index);

                if !queued {
                    continue;
                }

                // The output duration is authoritative for the public playback timeline.
                if output_index >= duration_frames {
                    worker_finished.store(current_generation, Ordering::Release);
                }
            }
        });

        Ok(Self {
            stream: Some(stream),
            state,
            position_frames,
            seek_generation,
            seek_target_frame,
            finished_generation,
            duration_frames,
            output_rate,
            worker: Some(worker),
        })
    }

    pub fn play(&self) {
        // Stopped and Ended are terminal for this instance.
        start_playback(&self.state);
    }

    pub fn pause(&self) {
        if self.state.load(Ordering::Acquire) == PLAYING {
            self.state.store(PAUSED, Ordering::Release);
        }
    }

    pub fn resume(&self) {
        self.play();
    }

    pub fn stop(&self) {
        self.state.store(STOPPED, Ordering::Release);
        self.position_frames.store(0, Ordering::Relaxed);
    }

    pub fn seek_to_frame(&self, frame: u64) {
        let frame = frame.min(self.duration_frames);
        self.seek_target_frame.store(frame, Ordering::Relaxed);
        self.position_frames.store(frame, Ordering::Relaxed);
        self.finished_generation.store(u64::MAX, Ordering::Relaxed);
        self.seek_generation.fetch_add(1, Ordering::Release);
        update_state_after_seek(&self.state, frame, self.duration_frames);
    }

    pub fn seek(&self, seconds: f64) -> Result<(), Box<dyn Error>> {
        if !seconds.is_finite() || seconds < 0.0 {
            return Err(
                io::Error::other("Seek position must be a finite, non-negative number").into(),
            );
        }

        if self.state() == AudioEngineState::Stopped {
            return Err(io::Error::other("Cannot seek after stop; open the file again").into());
        }

        let target = (seconds * self.output_rate as f64)
            .floor()
            .min(self.duration_frames as f64) as u64;

        self.seek_target_frame.store(target, Ordering::Relaxed);

        self.seek_generation.fetch_add(1, Ordering::Release);
        self.position_frames.store(target, Ordering::Relaxed);

        update_state_after_seek(&self.state, target, self.duration_frames);

        Ok(())
    }

    pub fn duration_seconds(&self) -> f64 {
        self.duration_frames as f64 / self.output_rate as f64
    }

    pub fn state(&self) -> AudioEngineState {
        AudioEngineState::from_raw(self.state.load(Ordering::Acquire))
    }

    pub fn is_playing(&self) -> bool {
        self.state() == AudioEngineState::Playing
    }

    pub fn is_ended(&self) -> bool {
        self.state() == AudioEngineState::Ended
    }

    pub fn position_seconds(&self) -> f64 {
        self.position_frames.load(Ordering::Relaxed) as f64 / self.output_rate as f64
    }

    pub fn output_sample_rate(&self) -> u32 {
        self.output_rate
    }
}

impl Drop for AudioEngine {
    fn drop(&mut self) {
        self.state.store(STOPPED, Ordering::Release);

        drop(self.stream.take());

        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn output_frame(generation: u64, output_index: u64) -> OutputFrame {
        OutputFrame {
            samples: [generation as f32; MAX_CHANNELS],
            generation,
            output_index,
        }
    }

    #[test]
    fn stale_buffered_frames_are_discarded_after_rapid_seeks() {
        let ring = HeapRb::<OutputFrame>::new(8);
        let (mut producer, mut consumer) = ring.split();

        for generation in 0..5 {
            assert!(
                producer
                    .try_push(output_frame(generation, generation))
                    .is_ok()
            );
        }

        let latest_generation = 5;
        assert!(
            producer
                .try_push(output_frame(latest_generation, latest_generation))
                .is_ok()
        );

        let generation = AtomicU64::new(latest_generation);
        let (frame, generation) = pop_current_frame(&mut consumer, &generation);
        let frame = frame.unwrap();
        assert_eq!(frame.generation, latest_generation);
        assert_eq!(frame.output_index, latest_generation);
        assert_eq!(generation, latest_generation);
        assert!(consumer.try_pop().is_none());
    }

    #[test]
    fn resampling_linearly_interpolates_between_source_samples() {
        let mut resampler = LinearResampler::new(2, 4, 1).unwrap();
        let mut samples = resampler.push(&[0.0, 1.0]).unwrap();
        samples.extend(resampler.finish());

        assert_eq!(samples, [0.0, 0.5, 1.0, 1.0]);
    }

    #[test]
    fn seeking_backward_after_eof_allows_playback_again() {
        let state = AtomicU8::new(ENDED);

        update_state_after_seek(&state, 25, 100);
        assert_eq!(
            AudioEngineState::from_raw(state.load(Ordering::Acquire)),
            AudioEngineState::Paused
        );

        start_playback(&state);
        assert_eq!(
            AudioEngineState::from_raw(state.load(Ordering::Acquire)),
            AudioEngineState::Playing
        );
    }
}

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

pub trait Clock: Send + Sync {
    fn time(&self) -> f64;
    fn reset(&mut self);
    fn adjust(&mut self, delta: f64);
}

#[derive(Debug, Clone)]
pub struct AudioClock {
    inner: Arc<Mutex<AudioClockInner>>,
}

#[derive(Debug)]
struct AudioClockInner {
    samples_played: u64,
    sample_rate: u32,
    start_instant: Instant,
    base_time: f64,
}

impl AudioClock {
    pub fn new(sample_rate: u32) -> Self {
        AudioClock {
            inner: Arc::new(Mutex::new(AudioClockInner {
                samples_played: 0,
                sample_rate,
                start_instant: Instant::now(),
                base_time: 0.0,
            })),
        }
    }

    pub fn update(&self, samples: u64) {
        let mut inner = self.inner.lock().unwrap();
        inner.samples_played = samples;
    }

    pub fn increment(&self, samples: u64) {
        let mut inner = self.inner.lock().unwrap();
        inner.samples_played += samples;
    }

    pub fn samples_played(&self) -> u64 {
        self.inner.lock().unwrap().samples_played
    }

    pub fn sample_rate(&self) -> u32 {
        self.inner.lock().unwrap().sample_rate
    }
}

impl Clock for AudioClock {
    fn time(&self) -> f64 {
        let inner = self.inner.lock().unwrap();
        inner.base_time + (inner.samples_played as f64 / inner.sample_rate as f64)
    }

    fn reset(&mut self) {
        let mut inner = self.inner.lock().unwrap();
        inner.samples_played = 0;
        inner.base_time = 0.0;
        inner.start_instant = Instant::now();
    }

    fn adjust(&mut self, delta: f64) {
        let mut inner = self.inner.lock().unwrap();
        inner.base_time += delta;
    }
}

#[derive(Debug)]
pub struct SystemClock {
    start: Instant,
    offset: f64,
}

impl SystemClock {
    pub fn new() -> Self {
        SystemClock {
            start: Instant::now(),
            offset: 0.0,
        }
    }
}

impl Default for SystemClock {
    fn default() -> Self {
        Self::new()
    }
}

impl Clock for SystemClock {
    fn time(&self) -> f64 {
        let elapsed = self.start.elapsed();
        elapsed.as_secs_f64() + self.offset
    }

    fn reset(&mut self) {
        self.start = Instant::now();
        self.offset = 0.0;
    }

    fn adjust(&mut self, delta: f64) {
        self.offset += delta;
    }
}

pub fn pts_to_seconds(pts: u64, timescale: u32) -> f64 {
    pts as f64 / timescale as f64
}

pub fn seconds_to_pts(seconds: f64, timescale: u32) -> u64 {
    (seconds * timescale as f64) as u64
}

pub fn duration_to_seconds(duration: u64, timescale: u32) -> f64 {
    duration as f64 / timescale as f64
}

pub fn time_until(target_time: f64, current_time: f64) -> Duration {
    let delta = target_time - current_time;
    if delta <= 0.0 {
        Duration::ZERO
    } else {
        Duration::from_secs_f64(delta)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn audio_clock_tracks_samples() {
        let clock = AudioClock::new(48000);

        assert_eq!(clock.time(), 0.0);

        clock.update(48000); // 1 second
        assert!((clock.time() - 1.0).abs() < 0.001);

        clock.update(96000); // 2 seconds
        assert!((clock.time() - 2.0).abs() < 0.001);
    }

    #[test]
    fn audio_clock_increments() {
        let clock = AudioClock::new(44100);

        clock.increment(22050); // 0.5 seconds
        assert!((clock.time() - 0.5).abs() < 0.001);

        clock.increment(22050); // 1 second total
        assert!((clock.time() - 1.0).abs() < 0.001);
    }

    #[test]
    fn audio_clock_resets() {
        let mut clock = AudioClock::new(48000);

        clock.update(48000);
        assert!(clock.time() > 0.0);

        clock.reset();
        assert_eq!(clock.time(), 0.0);
    }

    #[test]
    fn audio_clock_adjusts() {
        let mut clock = AudioClock::new(48000);

        clock.update(48000);
        assert!((clock.time() - 1.0).abs() < 0.001);

        clock.adjust(0.5); // Add 0.5 seconds
        assert!((clock.time() - 1.5).abs() < 0.001);
    }

    #[test]
    fn converts_pts_to_seconds() {
        assert_eq!(pts_to_seconds(90000, 90000), 1.0);
        assert_eq!(pts_to_seconds(48000, 48000), 1.0);
        assert!((pts_to_seconds(22050, 44100) - 0.5).abs() < 0.001);
    }

    #[test]
    fn converts_seconds_to_pts() {
        assert_eq!(seconds_to_pts(1.0, 90000), 90000);
        assert_eq!(seconds_to_pts(0.5, 48000), 24000);
    }

    #[test]
    fn calculates_time_until() {
        let duration = time_until(2.0, 1.0);
        assert!((duration.as_secs_f64() - 1.0).abs() < 0.001);

        let duration = time_until(1.0, 2.0); // Past
        assert_eq!(duration, Duration::ZERO);
    }

    #[test]
    fn system_clock_advances() {
        let clock = SystemClock::new();
        let t1 = clock.time();

        std::thread::sleep(Duration::from_millis(10));

        let t2 = clock.time();
        assert!(t2 > t1);
    }
}

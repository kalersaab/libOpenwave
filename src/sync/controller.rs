use super::clock::{AudioClock, Clock};
use super::scheduler::{FrameAction, FrameScheduler};
use std::sync::{Arc, Mutex};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StreamState {
    Idle,
    Playing,
    Paused,
    Buffering,
    Seeking,
    Finished,
}

pub struct SyncController {
    audio_clock: AudioClock,
    video_scheduler: FrameScheduler,
    state: Arc<Mutex<StreamState>>,
    video_timescale: u32,
    audio_timescale: u32,
}

impl SyncController {
    pub fn new(
        audio_sample_rate: u32,
        video_timescale: u32,
        audio_timescale: u32,
        video_fps: f64,
    ) -> Self {
        SyncController {
            audio_clock: AudioClock::new(audio_sample_rate),
            video_scheduler: FrameScheduler::new(video_timescale, audio_timescale, video_fps),
            state: Arc::new(Mutex::new(StreamState::Idle)),
            video_timescale,
            audio_timescale,
        }
    }

    pub fn start(&self) {
        let mut state = self.state.lock().unwrap();
        *state = StreamState::Playing;
    }

    pub fn pause(&self) {
        let mut state = self.state.lock().unwrap();
        *state = StreamState::Paused;
    }

    pub fn resume(&self) {
        let mut state = self.state.lock().unwrap();
        if *state == StreamState::Paused {
            *state = StreamState::Playing;
        }
    }

    pub fn stop(&self) {
        let mut state = self.state.lock().unwrap();
        *state = StreamState::Finished;
    }

    pub fn state(&self) -> StreamState {
        *self.state.lock().unwrap()
    }

    pub fn is_playing(&self) -> bool {
        self.state() == StreamState::Playing
    }

    pub fn update_audio_clock(&self, samples: u64) {
        self.audio_clock.update(samples);
    }

    pub fn increment_audio_clock(&self, samples: u64) {
        self.audio_clock.increment(samples);
    }

    pub fn audio_time(&self) -> f64 {
        self.audio_clock.time()
    }

    pub fn schedule_video_frame(&mut self, frame_pts: u64) -> FrameAction {
        let audio_time = self.audio_clock.time();
        self.video_scheduler.schedule_frame(frame_pts, audio_time)
    }

    pub fn sync_drift(&self, frame_pts: u64) -> i64 {
        let audio_time = self.audio_clock.time();
        self.video_scheduler.calculate_drift(frame_pts, audio_time)
    }

    pub fn scheduler_stats(&self) -> super::scheduler::SchedulerStats {
        self.video_scheduler.stats()
    }

    pub fn seek(&mut self, time_seconds: f64) {
        let mut state = self.state.lock().unwrap();
        *state = StreamState::Seeking;
        drop(state);

        self.video_scheduler.reset();
        self.audio_clock.reset();
        self.audio_clock.adjust(time_seconds);
    }

    pub fn reset(&mut self) {
        self.audio_clock.reset();
        self.video_scheduler.reset();
        let mut state = self.state.lock().unwrap();
        *state = StreamState::Idle;
    }

    pub fn audio_clock(&self) -> &AudioClock {
        &self.audio_clock
    }

    pub fn video_pts_to_seconds(&self, pts: u64) -> f64 {
        pts as f64 / self.video_timescale as f64
    }

    pub fn audio_pts_to_seconds(&self, pts: u64) -> f64 {
        pts as f64 / self.audio_timescale as f64
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn starts_in_idle_state() {
        let controller = SyncController::new(48000, 90000, 48000, 30.0);
        assert_eq!(controller.state(), StreamState::Idle);
    }

    #[test]
    fn transitions_states() {
        let controller = SyncController::new(48000, 90000, 48000, 30.0);

        controller.start();
        assert_eq!(controller.state(), StreamState::Playing);
        assert!(controller.is_playing());

        controller.pause();
        assert_eq!(controller.state(), StreamState::Paused);
        assert!(!controller.is_playing());

        controller.resume();
        assert_eq!(controller.state(), StreamState::Playing);

        controller.stop();
        assert_eq!(controller.state(), StreamState::Finished);
    }

    #[test]
    fn updates_audio_clock() {
        let controller = SyncController::new(48000, 90000, 48000, 30.0);

        assert_eq!(controller.audio_time(), 0.0);

        controller.update_audio_clock(48000);
        assert!((controller.audio_time() - 1.0).abs() < 0.001);
    }

    #[test]
    fn increments_audio_clock() {
        let controller = SyncController::new(48000, 90000, 48000, 30.0);

        controller.increment_audio_clock(24000);
        assert!((controller.audio_time() - 0.5).abs() < 0.001);

        controller.increment_audio_clock(24000);
        assert!((controller.audio_time() - 1.0).abs() < 0.001);
    }

    #[test]
    fn schedules_video_frames() {
        let mut controller = SyncController::new(48000, 90000, 48000, 30.0);

        controller.start();
        controller.update_audio_clock(0);

        // Frame at time 0 with audio at 0
        let action = controller.schedule_video_frame(0);
        assert_eq!(action, FrameAction::DisplayNow);
    }

    #[test]
    fn calculates_sync_drift() {
        let controller = SyncController::new(48000, 90000, 48000, 30.0);

        controller.update_audio_clock(24000); // 0.5 seconds

        // Frame at 1.0s, audio at 0.5s = 500ms drift
        let drift = controller.sync_drift(90000);
        assert_eq!(drift, 500);
    }

    #[test]
    fn seeks_to_position() {
        let mut controller = SyncController::new(48000, 90000, 48000, 30.0);

        controller.start();
        controller.update_audio_clock(48000); // 1 second

        controller.seek(5.0);
        assert_eq!(controller.state(), StreamState::Seeking);
        assert!((controller.audio_time() - 5.0).abs() < 0.001);
    }

    #[test]
    fn resets_to_initial_state() {
        let mut controller = SyncController::new(48000, 90000, 48000, 30.0);

        controller.start();
        controller.update_audio_clock(48000);

        controller.reset();
        assert_eq!(controller.state(), StreamState::Idle);
        assert_eq!(controller.audio_time(), 0.0);
    }

    #[test]
    fn converts_pts_to_seconds() {
        let controller = SyncController::new(48000, 90000, 48000, 30.0);

        assert_eq!(controller.video_pts_to_seconds(90000), 1.0);
        assert_eq!(controller.audio_pts_to_seconds(48000), 1.0);
    }
}

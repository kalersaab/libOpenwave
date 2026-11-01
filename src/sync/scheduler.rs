use std::time::Duration;

pub const MAX_SYNC_DRIFT_MS: i64 = 100;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FrameAction {
    Display(Duration),
    Drop,
    DisplayNow,
}

#[derive(Debug)]
pub struct FrameScheduler {
    video_timescale: u32,
    _audio_timescale: u32,
    target_fps: f64,
    last_frame_pts: Option<u64>,
    _dropped_frames: u64,
    _duplicated_frames: u64,
}

impl FrameScheduler {
    pub fn new(video_timescale: u32, audio_timescale: u32, target_fps: f64) -> Self {
        FrameScheduler {
            video_timescale,
            _audio_timescale: audio_timescale,
            target_fps,
            last_frame_pts: None,
            _dropped_frames: 0,
            _duplicated_frames: 0,
        }
    }

    pub fn schedule_frame(&mut self, frame_pts: u64, audio_time: f64) -> FrameAction {
        let frame_time = frame_pts as f64 / self.video_timescale as f64;
        let drift_seconds = frame_time - audio_time;
        let drift_ms = (drift_seconds * 1000.0) as i64;

        self.last_frame_pts = Some(frame_pts);

        if drift_ms < -MAX_SYNC_DRIFT_MS {
            self._dropped_frames += 1;
            FrameAction::Drop
        } else if drift_ms > MAX_SYNC_DRIFT_MS {
            let wait_duration = Duration::from_secs_f64(drift_seconds);
            FrameAction::Display(wait_duration)
        } else {
            FrameAction::DisplayNow
        }
    }

    pub fn expected_next_pts(&self) -> Option<u64> {
        self.last_frame_pts.map(|pts| {
            let frame_duration = self.video_timescale as f64 / self.target_fps;
            pts + frame_duration as u64
        })
    }

    pub fn stats(&self) -> SchedulerStats {
        SchedulerStats {
            dropped_frames: self._dropped_frames,
            duplicated_frames: self._duplicated_frames,
        }
    }

    pub fn reset(&mut self) {
        self.last_frame_pts = None;
        self._dropped_frames = 0;
        self._duplicated_frames = 0;
    }

    pub fn calculate_drift(&self, frame_pts: u64, audio_time: f64) -> i64 {
        let frame_time = frame_pts as f64 / self.video_timescale as f64;
        let drift_seconds = frame_time - audio_time;
        (drift_seconds * 1000.0) as i64
    }
}

#[derive(Debug, Clone, Copy)]
pub struct SchedulerStats {
    pub dropped_frames: u64,
    pub duplicated_frames: u64,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn schedules_on_time_frame() {
        let mut scheduler = FrameScheduler::new(90000, 48000, 30.0);

        let action = scheduler.schedule_frame(0, 0.0);
        assert_eq!(action, FrameAction::DisplayNow);
    }

    #[test]
    fn schedules_frame_with_wait() {
        let mut scheduler = FrameScheduler::new(90000, 48000, 30.0);

        let action = scheduler.schedule_frame(90000, 0.0);
        match action {
            FrameAction::Display(duration) => {
                assert!((duration.as_secs_f64() - 1.0).abs() < 0.01);
            }
            _ => panic!("Expected Display action"),
        }
    }

    #[test]
    fn drops_late_frame() {
        let mut scheduler = FrameScheduler::new(90000, 48000, 30.0);

        let action = scheduler.schedule_frame(0, 1.0);
        assert_eq!(action, FrameAction::Drop);
        assert_eq!(scheduler.stats().dropped_frames, 1);
    }

    #[test]
    fn displays_frame_in_sync_range() {
        let mut scheduler = FrameScheduler::new(90000, 48000, 30.0);

        let action = scheduler.schedule_frame(4500, 0.0);
        assert_eq!(action, FrameAction::DisplayNow);
    }

    #[test]
    fn calculates_expected_next_pts() {
        let mut scheduler = FrameScheduler::new(90000, 48000, 30.0);

        assert_eq!(scheduler.expected_next_pts(), None);

        scheduler.schedule_frame(0, 0.0);

        let next_pts = scheduler.expected_next_pts().unwrap();
        assert_eq!(next_pts, 3000);
    }

    #[test]
    fn calculates_drift() {
        let scheduler = FrameScheduler::new(90000, 48000, 30.0);

        let drift = scheduler.calculate_drift(90000, 0.5);
        assert_eq!(drift, 500);

        let drift = scheduler.calculate_drift(45000, 1.0);
        assert_eq!(drift, -500);
    }

    #[test]
    fn resets_state() {
        let mut scheduler = FrameScheduler::new(90000, 48000, 30.0);

        scheduler.schedule_frame(0, 1.0); // Drop
        assert!(scheduler.stats().dropped_frames > 0);

        scheduler.reset();
        assert_eq!(scheduler.stats().dropped_frames, 0);
        assert_eq!(scheduler.expected_next_pts(), None);
    }
}

use crate::video::h264::frame::Yuv420Frame;
use std::collections::VecDeque;

#[derive(Debug, Clone)]
pub struct DecodedPicture {
    pub frame: Yuv420Frame,
    pub poc: i32,
    pub frame_num: u32,
    pub is_idr: bool,
    pub is_reference: bool,
}

impl DecodedPicture {
    pub fn new(frame: Yuv420Frame, poc: i32, frame_num: u32, is_idr: bool) -> Self {
        Self {
            frame,
            poc,
            frame_num,
            is_idr,
            is_reference: true,
        }
    }

    pub fn new_non_reference(frame: Yuv420Frame, poc: i32, frame_num: u32) -> Self {
        Self {
            frame,
            poc,
            frame_num,
            is_idr: false,
            is_reference: false,
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct DpbConfig {
    pub max_num_ref_frames: u32,
    pub max_reorder_delay: u32,
}

impl Default for DpbConfig {
    fn default() -> Self {
        Self {
            max_num_ref_frames: 4,
            max_reorder_delay: 16,
        }
    }
}

pub struct DecodedPictureBuffer {
    config: DpbConfig,
    output_queue: VecDeque<DecodedPicture>,
    decode_count: u64,
    output_count: u64,
    flush_mode: bool,
}

impl DecodedPictureBuffer {
    pub fn new(config: DpbConfig) -> Self {
        Self {
            config,
            output_queue: VecDeque::new(),
            decode_count: 0,
            output_count: 0,
            flush_mode: false,
        }
    }

    pub fn add_picture(&mut self, picture: DecodedPicture) {
        if picture.is_idr {
            self.flush_output();
            self.flush_mode = false;
        }

        let insert_pos = self
            .output_queue
            .iter()
            .position(|p| p.poc > picture.poc)
            .unwrap_or(self.output_queue.len());

        self.output_queue.insert(insert_pos, picture);
        self.decode_count += 1;
    }

    pub fn next_output(&mut self) -> Option<DecodedPicture> {
        if self.output_queue.is_empty() {
            return None;
        }

        let should_output = self.flush_mode
            || self.output_queue.len() >= self.config.max_reorder_delay as usize
            || self.has_sufficient_future_pictures();

        if should_output {
            self.output_count += 1;
            self.output_queue.pop_front()
        } else {
            None
        }
    }

    fn has_sufficient_future_pictures(&self) -> bool {
        if self.output_queue.is_empty() {
            return false;
        }

        if self.output_queue.len() == 1 {
            return false;
        }

        self.output_queue[0].poc < self.output_queue[1].poc
    }

    pub fn flush(&mut self) -> Vec<DecodedPicture> {
        self.flush_mode = true;
        let mut output = Vec::new();

        while let Some(pic) = self.next_output() {
            output.push(pic);
        }

        self.flush_mode = false;
        output
    }

    fn flush_output(&mut self) {
        self.output_queue.clear();
    }

    pub fn clear(&mut self) {
        self.output_queue.clear();
        self.decode_count = 0;
        self.output_count = 0;
        self.flush_mode = false;
    }

    pub fn pending_count(&self) -> usize {
        self.output_queue.len()
    }

    pub fn stats(&self) -> DpbStats {
        DpbStats {
            decoded: self.decode_count,
            output: self.output_count,
            pending: self.output_queue.len() as u64,
        }
    }

    pub fn contains_poc(&self, poc: i32) -> bool {
        self.output_queue.iter().any(|p| p.poc == poc)
    }

    pub fn peek_next(&self) -> Option<&DecodedPicture> {
        self.output_queue.front()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DpbStats {
    pub decoded: u64,
    pub output: u64,
    pub pending: u64,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_frame(width: u32, height: u32) -> Yuv420Frame {
        Yuv420Frame::new(width, height).unwrap()
    }

    fn make_picture(poc: i32, frame_num: u32, is_idr: bool) -> DecodedPicture {
        DecodedPicture::new(make_frame(16, 16), poc, frame_num, is_idr)
    }

    #[test]
    fn outputs_frames_in_poc_order() {
        let mut dpb = DecodedPictureBuffer::new(DpbConfig {
            max_num_ref_frames: 2,
            max_reorder_delay: 4,
        });

        dpb.add_picture(make_picture(0, 0, true));
        dpb.add_picture(make_picture(4, 2, false));
        dpb.add_picture(make_picture(2, 1, false));

        let pic0 = dpb.next_output().unwrap();
        assert_eq!(pic0.poc, 0);

        let pic1 = dpb.next_output().unwrap();
        assert_eq!(pic1.poc, 2);

        let remaining = dpb.flush();
        assert_eq!(remaining.len(), 1);
        assert_eq!(remaining[0].poc, 4);
    }

    #[test]
    fn delays_output_until_reorder_window_filled() {
        let mut dpb = DecodedPictureBuffer::new(DpbConfig {
            max_num_ref_frames: 2,
            max_reorder_delay: 3,
        });

        dpb.add_picture(make_picture(0, 0, true));

        dpb.add_picture(make_picture(2, 1, false));

        let pic = dpb.next_output().unwrap();
        assert_eq!(pic.poc, 0);
    }

    #[test]
    fn flush_outputs_all_pending_frames() {
        let mut dpb = DecodedPictureBuffer::new(DpbConfig::default());

        dpb.add_picture(make_picture(0, 0, true));
        dpb.add_picture(make_picture(4, 2, false));
        dpb.add_picture(make_picture(2, 1, false));
        dpb.add_picture(make_picture(6, 3, false));

        let flushed = dpb.flush();
        assert_eq!(flushed.len(), 4);
        assert_eq!(flushed[0].poc, 0);
        assert_eq!(flushed[1].poc, 2);
        assert_eq!(flushed[2].poc, 4);
        assert_eq!(flushed[3].poc, 6);
    }

    #[test]
    fn idr_flushes_previous_pictures() {
        let mut dpb = DecodedPictureBuffer::new(DpbConfig::default());

        dpb.add_picture(make_picture(0, 0, true));
        dpb.add_picture(make_picture(2, 1, false));

        assert_eq!(dpb.pending_count(), 2);

        dpb.add_picture(make_picture(0, 0, true));
        assert_eq!(dpb.pending_count(), 1);
    }

    #[test]
    fn tracks_statistics() {
        let mut dpb = DecodedPictureBuffer::new(DpbConfig::default());

        dpb.add_picture(make_picture(0, 0, true));
        dpb.add_picture(make_picture(2, 1, false));

        let stats = dpb.stats();
        assert_eq!(stats.decoded, 2);
        assert_eq!(stats.output, 0);
        assert_eq!(stats.pending, 2);

        dpb.next_output();
        let stats = dpb.stats();
        assert_eq!(stats.output, 1);
    }

    #[test]
    fn clears_buffer_completely() {
        let mut dpb = DecodedPictureBuffer::new(DpbConfig::default());

        dpb.add_picture(make_picture(0, 0, true));
        dpb.add_picture(make_picture(2, 1, false));

        dpb.clear();

        assert_eq!(dpb.pending_count(), 0);
        let stats = dpb.stats();
        assert_eq!(stats.decoded, 0);
        assert_eq!(stats.output, 0);
    }

    #[test]
    fn contains_poc_check_works() {
        let mut dpb = DecodedPictureBuffer::new(DpbConfig::default());

        dpb.add_picture(make_picture(0, 0, true));
        dpb.add_picture(make_picture(4, 2, false));

        assert!(dpb.contains_poc(0));
        assert!(dpb.contains_poc(4));
        assert!(!dpb.contains_poc(2));
    }

    #[test]
    fn peek_does_not_remove_picture() {
        let mut dpb = DecodedPictureBuffer::new(DpbConfig::default());

        dpb.add_picture(make_picture(0, 0, true));
        dpb.add_picture(make_picture(2, 1, false));

        let peeked = dpb.peek_next().unwrap();
        assert_eq!(peeked.poc, 0);
        assert_eq!(dpb.pending_count(), 2);

        let output = dpb.next_output().unwrap();
        assert_eq!(output.poc, 0);
        assert_eq!(dpb.pending_count(), 1);
    }

    #[test]
    fn handles_complex_reordering_scenario() {
        let mut dpb = DecodedPictureBuffer::new(DpbConfig {
            max_num_ref_frames: 4,
            max_reorder_delay: 4,
        });

        let decode_order = vec![
            (0, 0, true),
            (8, 2, false),
            (2, 1, false),
            (4, 3, false),
            (6, 4, false),
            (16, 6, false),
            (10, 5, false),
            (12, 7, false),
            (14, 8, false),
        ];

        for (poc, frame_num, is_idr) in decode_order {
            dpb.add_picture(make_picture(poc, frame_num, is_idr));
        }

        let all = dpb.flush();
        assert_eq!(all.len(), 9);

        let pocs: Vec<i32> = all.iter().map(|p| p.poc).collect();
        assert_eq!(pocs, vec![0, 2, 4, 6, 8, 10, 12, 14, 16]);
    }
}

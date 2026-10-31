use crate::video::h264::frame::Yuv420Frame;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReferencePicture {
    pub frame_num: u32,

    pub pic_order_cnt: i64,

    pub long_term_frame_idx: Option<u32>,

    pub frame: Yuv420Frame,
}

impl ReferencePicture {
    pub fn new(frame_num: u32, pic_order_cnt: i64, frame: Yuv420Frame) -> Self {
        Self {
            frame_num,
            pic_order_cnt,
            long_term_frame_idx: None,
            frame,
        }
    }

    pub fn is_short_term(&self) -> bool {
        self.long_term_frame_idx.is_none()
    }

    pub fn is_long_term(&self) -> bool {
        self.long_term_frame_idx.is_some()
    }

    pub fn mark_as_long_term(&mut self, frame_idx: u32) {
        self.long_term_frame_idx = Some(frame_idx);
    }

    pub fn mark_as_short_term(&mut self) {
        self.long_term_frame_idx = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::video::h264::frame::Yuv420Frame;

    #[test]
    fn creates_short_term_reference() {
        let frame = Yuv420Frame::new(16, 16).unwrap();
        let picture = ReferencePicture::new(5, 10, frame);

        assert_eq!(picture.frame_num, 5);
        assert_eq!(picture.pic_order_cnt, 10);
        assert!(picture.is_short_term());
        assert!(!picture.is_long_term());
    }

    #[test]
    fn marks_as_long_term() {
        let frame = Yuv420Frame::new(16, 16).unwrap();
        let mut picture = ReferencePicture::new(5, 10, frame);

        picture.mark_as_long_term(3);
        assert!(picture.is_long_term());
        assert_eq!(picture.long_term_frame_idx, Some(3));

        picture.mark_as_short_term();
        assert!(picture.is_short_term());
        assert_eq!(picture.long_term_frame_idx, None);
    }
}

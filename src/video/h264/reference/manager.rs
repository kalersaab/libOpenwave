use crate::video::h264::reference::picture::ReferencePicture;

#[derive(Default)]
pub struct ReferenceManager {
    pictures: Vec<ReferencePicture>,
}

impl ReferenceManager {
    pub fn new() -> Self {
        Self {
            pictures: Vec::new(),
        }
    }

    pub fn clear(&mut self) {
        self.pictures.clear();
    }

    pub fn pictures(&self) -> &[ReferencePicture] {
        &self.pictures
    }

    pub fn insert(
        &mut self,
        picture: ReferencePicture,
        max_num_ref_frames: u32,
        current_frame_num: u32,
        max_frame_num: u32,
    ) -> Result<(), super::super::decoder::H264DecodeError> {
        let capacity = usize::try_from(max_num_ref_frames)
            .map_err(|_| super::super::decoder::H264DecodeError::InvalidStream)?;

        if capacity == 0 {
            return Ok(());
        }

        while self.pictures.len() >= capacity {
            let oldest_short_term = self
                .pictures
                .iter()
                .enumerate()
                .filter(|(_, picture)| picture.is_short_term())
                .min_by_key(|(_, picture)| {
                    frame_num_wrap(picture.frame_num, current_frame_num, max_frame_num)
                })
                .map(|(index, _)| index);

            let Some(index) = oldest_short_term else {
                return Err(super::super::decoder::H264DecodeError::InvalidStream);
            };

            self.pictures.remove(index);
        }

        self.pictures.push(picture);
        Ok(())
    }

    pub fn remove_short_term_by_pic_num(
        &mut self,
        pic_num_diff: u32,
        current_frame_num: u32,
        max_frame_num: u32,
    ) {
        let pic_num = (current_frame_num as i64 - pic_num_diff as i64 - 1)
            .rem_euclid(max_frame_num as i64) as u32;
        self.pictures
            .retain(|p| !p.is_short_term() || p.frame_num != pic_num);
    }

    pub fn remove_long_term_by_frame_idx(&mut self, frame_idx: u32) {
        self.pictures
            .retain(|p| p.long_term_frame_idx != Some(frame_idx));
    }

    pub fn mark_short_term_as_long_term(
        &mut self,
        pic_num_diff: u32,
        frame_idx: u32,
        current_frame_num: u32,
        max_frame_num: u32,
    ) {
        let pic_num = (current_frame_num as i64 - pic_num_diff as i64 - 1)
            .rem_euclid(max_frame_num as i64) as u32;

        for picture in &mut self.pictures {
            if picture.is_short_term() && picture.frame_num == pic_num {
                picture.mark_as_long_term(frame_idx);
                break;
            }
        }
    }

    pub fn set_max_long_term_frame_idx(&mut self, max_frame_idx: u32) {
        self.pictures.retain(|p| {
            p.is_short_term() || p.long_term_frame_idx.is_none_or(|idx| idx <= max_frame_idx)
        });
    }

    pub fn len(&self) -> usize {
        self.pictures.len()
    }

    pub fn is_empty(&self) -> bool {
        self.pictures.is_empty()
    }
}

fn frame_num_wrap(frame_num: u32, current_frame_num: u32, max_frame_num: u32) -> i64 {
    if frame_num > current_frame_num {
        i64::from(frame_num) - i64::from(max_frame_num)
    } else {
        i64::from(frame_num)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::video::h264::frame::Yuv420Frame;

    fn make_picture(frame_num: u32, poc: i64) -> ReferencePicture {
        ReferencePicture::new(frame_num, poc, Yuv420Frame::new(16, 16).unwrap())
    }

    #[test]
    fn manages_reference_buffer_capacity() {
        let mut manager = ReferenceManager::new();

        manager.insert(make_picture(1, 1), 2, 4, 16).unwrap();
        manager.insert(make_picture(2, 2), 2, 4, 16).unwrap();
        assert_eq!(manager.len(), 2);

        manager.insert(make_picture(3, 3), 2, 4, 16).unwrap();
        assert_eq!(manager.len(), 2);
        assert_eq!(manager.pictures()[0].frame_num, 2);
        assert_eq!(manager.pictures()[1].frame_num, 3);
    }

    #[test]
    fn clears_all_references() {
        let mut manager = ReferenceManager::new();
        manager.insert(make_picture(1, 1), 2, 2, 16).unwrap();
        manager.insert(make_picture(2, 2), 2, 3, 16).unwrap();

        manager.clear();
        assert!(manager.is_empty());
    }

    #[test]
    fn marks_pictures_as_long_term() {
        let mut manager = ReferenceManager::new();
        manager.insert(make_picture(5, 5), 3, 6, 16).unwrap();

        manager.mark_short_term_as_long_term(0, 0, 6, 16);
        assert!(manager.pictures()[0].is_long_term());
        assert_eq!(manager.pictures()[0].long_term_frame_idx, Some(0));
    }

    #[test]
    fn removes_long_term_references() {
        let mut manager = ReferenceManager::new();
        let mut pic = make_picture(5, 5);
        pic.mark_as_long_term(2);
        manager.insert(pic, 3, 6, 16).unwrap();

        manager.remove_long_term_by_frame_idx(2);
        assert!(manager.is_empty());
    }
}

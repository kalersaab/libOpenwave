use crate::video::h264::{reference::picture::ReferencePicture, slice::SliceType};

#[derive(Debug, Clone, Copy)]
pub struct ReferencePictureListConfig {
    pub slice_type: SliceType,
    pub current_frame_num: u32,
    pub max_frame_num: u32,
    pub current_pic_order_cnt: i64,
    pub max_pic_order_cnt: Option<i64>,
    pub list0_count: u32,
    pub list1_count: u32,
}

pub struct ReferencePictureLists<'a> {
    pub list0: Vec<&'a ReferencePicture>,
    pub list1: Vec<&'a ReferencePicture>,
}

impl<'a> ReferencePictureLists<'a> {
    pub fn build(
        pictures: &'a [ReferencePicture],
        config: ReferencePictureListConfig,
    ) -> Result<Self, super::super::decoder::H264DecodeError> {
        let short_term = pictures
            .iter()
            .filter(|picture| picture.is_short_term())
            .collect::<Vec<_>>();

        let mut long_term = pictures
            .iter()
            .filter(|picture| picture.is_long_term())
            .collect::<Vec<_>>();

        long_term.sort_by_key(|picture| picture.long_term_frame_idx);

        let (mut list0, mut list1) = match config.slice_type {
            SliceType::P => Self::build_p_slice_lists(short_term, long_term, config)?,
            SliceType::B => Self::build_b_slice_lists(short_term, long_term, config)?,
            _ => (Vec::new(), Vec::new()),
        };

        list0.truncate(
            usize::try_from(config.list0_count)
                .map_err(|_| super::super::decoder::H264DecodeError::InvalidStream)?,
        );
        list1.truncate(
            usize::try_from(config.list1_count)
                .map_err(|_| super::super::decoder::H264DecodeError::InvalidStream)?,
        );

        Ok(Self { list0, list1 })
    }

    fn build_p_slice_lists(
        mut short_term: Vec<&'a ReferencePicture>,
        long_term: Vec<&'a ReferencePicture>,
        config: ReferencePictureListConfig,
    ) -> Result<
        (Vec<&'a ReferencePicture>, Vec<&'a ReferencePicture>),
        super::super::decoder::H264DecodeError,
    > {
        short_term.sort_by_key(|picture| {
            std::cmp::Reverse(frame_num_wrap(
                picture.frame_num,
                config.current_frame_num,
                config.max_frame_num,
            ))
        });

        let mut list0 = short_term;
        list0.extend(long_term);

        Ok((list0, Vec::new()))
    }

    fn build_b_slice_lists(
        short_term: Vec<&'a ReferencePicture>,
        long_term: Vec<&'a ReferencePicture>,
        config: ReferencePictureListConfig,
    ) -> Result<
        (Vec<&'a ReferencePicture>, Vec<&'a ReferencePicture>),
        super::super::decoder::H264DecodeError,
    > {
        let mut before = Vec::new();
        let mut after = Vec::new();

        for picture in short_term {
            let order = unwrap_pic_order_cnt(
                picture.pic_order_cnt,
                config.current_pic_order_cnt,
                config.max_pic_order_cnt,
            );

            if order < config.current_pic_order_cnt {
                before.push((order, picture));
            } else {
                after.push((order, picture));
            }
        }

        before.sort_by_key(|(order, _)| std::cmp::Reverse(*order));
        after.sort_by_key(|(order, _)| *order);

        let mut list0 = before
            .iter()
            .chain(after.iter())
            .map(|(_, picture)| *picture)
            .collect::<Vec<_>>();

        let mut list1 = after
            .iter()
            .chain(before.iter())
            .map(|(_, picture)| *picture)
            .collect::<Vec<_>>();

        list0.extend(long_term.iter().copied());
        list1.extend(long_term);

        if list0.len() > 1
            && list0.len() == list1.len()
            && list0
                .iter()
                .zip(&list1)
                .all(|(left, right)| std::ptr::eq(*left, *right))
        {
            list1.swap(0, 1);
        }

        Ok((list0, list1))
    }

    pub fn get_list0(
        &self,
        index: u32,
    ) -> Result<&'a ReferencePicture, super::super::decoder::H264DecodeError> {
        self.get_from_list(&self.list0, index)
    }

    pub fn get_list1(
        &self,
        index: u32,
    ) -> Result<&'a ReferencePicture, super::super::decoder::H264DecodeError> {
        self.get_from_list(&self.list1, index)
    }

    fn get_from_list(
        &self,
        list: &[&'a ReferencePicture],
        index: u32,
    ) -> Result<&'a ReferencePicture, super::super::decoder::H264DecodeError> {
        let idx = usize::try_from(index)
            .map_err(|_| super::super::decoder::H264DecodeError::InvalidStream)?;

        list.get(idx).copied().ok_or(
            super::super::decoder::H264DecodeError::InvalidReferenceIndex {
                index,
                list_length: list.len(),
            },
        )
    }
}

fn frame_num_wrap(frame_num: u32, current_frame_num: u32, max_frame_num: u32) -> i64 {
    if frame_num > current_frame_num {
        i64::from(frame_num) - i64::from(max_frame_num)
    } else {
        i64::from(frame_num)
    }
}

fn unwrap_pic_order_cnt(
    reference_pic_order_cnt: i64,
    current_pic_order_cnt: i64,
    max_pic_order_cnt: Option<i64>,
) -> i64 {
    let Some(max_pic_order_cnt) = max_pic_order_cnt else {
        return reference_pic_order_cnt;
    };

    let mut difference =
        (reference_pic_order_cnt - current_pic_order_cnt).rem_euclid(max_pic_order_cnt);

    if difference >= max_pic_order_cnt / 2 {
        difference -= max_pic_order_cnt;
    }

    current_pic_order_cnt + difference
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::video::h264::frame::Yuv420Frame;

    fn make_short_term_picture(frame_num: u32, poc: i64) -> ReferencePicture {
        ReferencePicture::new(frame_num, poc, Yuv420Frame::new(16, 16).unwrap())
    }

    #[test]
    fn builds_p_slice_list() {
        let pictures = vec![
            make_short_term_picture(3, 3),
            make_short_term_picture(5, 5),
            make_short_term_picture(4, 4),
        ];

        let config = ReferencePictureListConfig {
            slice_type: SliceType::P,
            current_frame_num: 6,
            max_frame_num: 16,
            current_pic_order_cnt: 6,
            max_pic_order_cnt: Some(16),
            list0_count: 3,
            list1_count: 0,
        };

        let lists = ReferencePictureLists::build(&pictures, config).unwrap();

        assert_eq!(lists.list0.len(), 3);
        assert_eq!(lists.list0[0].frame_num, 5);
        assert_eq!(lists.list0[1].frame_num, 4);
        assert_eq!(lists.list0[2].frame_num, 3);
        assert_eq!(lists.list1.len(), 0);
    }

    #[test]
    fn builds_b_slice_lists() {
        let pictures = vec![
            make_short_term_picture(1, 1),
            make_short_term_picture(2, 2),
            make_short_term_picture(4, 4),
        ];

        let config = ReferencePictureListConfig {
            slice_type: SliceType::B,
            current_frame_num: 3,
            max_frame_num: 16,
            current_pic_order_cnt: 3,
            max_pic_order_cnt: Some(16),
            list0_count: 3,
            list1_count: 3,
        };

        let lists = ReferencePictureLists::build(&pictures, config).unwrap();

        assert_eq!(lists.list0.len(), 3);
        assert_eq!(lists.list0[0].pic_order_cnt, 2);
        assert_eq!(lists.list0[1].pic_order_cnt, 1);
        assert_eq!(lists.list0[2].pic_order_cnt, 4);

        assert_eq!(lists.list1.len(), 3);
        assert_eq!(lists.list1[0].pic_order_cnt, 4);
        assert_eq!(lists.list1[1].pic_order_cnt, 2);
        assert_eq!(lists.list1[2].pic_order_cnt, 1);
    }
}

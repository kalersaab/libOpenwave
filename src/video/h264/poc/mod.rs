use crate::video::h264::{slice::SliceHeader, sps::Sps};

#[derive(Debug, Clone)]
pub struct PocCalculator {
    poc_type: u32,
    max_pic_order_cnt_lsb: u32,
    prev_poc_msb: i32,
    prev_poc_lsb: u32,
    prev_frame_num: u32,
    prev_frame_num_offset: i32,
}

impl PocCalculator {
    pub fn new(sps: &Sps) -> Self {
        let max_pic_order_cnt_lsb = if sps.pic_order_cnt_type == 0 {
            1 << sps.log2_max_pic_order_cnt_lsb
        } else {
            0
        };

        Self {
            poc_type: sps.pic_order_cnt_type,
            max_pic_order_cnt_lsb,
            prev_poc_msb: 0,
            prev_poc_lsb: 0,
            prev_frame_num: 0,
            prev_frame_num_offset: 0,
        }
    }

    pub fn calculate(
        &mut self,
        slice: &SliceHeader,
        sps: &Sps,
        max_frame_num: u32,
    ) -> Result<(i32, i32), PocError> {
        match self.poc_type {
            0 => self.calculate_type_0(slice, sps),
            1 => self.calculate_type_1(slice, sps, max_frame_num),
            2 => self.calculate_type_2(slice, max_frame_num),
            _ => Err(PocError::UnsupportedPocType),
        }
    }

    fn calculate_type_0(&mut self, slice: &SliceHeader, sps: &Sps) -> Result<(i32, i32), PocError> {
        let pic_order_cnt_lsb = slice.pic_order_cnt_lsb.ok_or(PocError::MissingPocLsb)? as i32;

        let pic_order_cnt_msb = if slice.prefix.is_idr {
            0
        } else {
            let prev_poc_lsb = self.prev_poc_lsb as i32;
            let max_poc_lsb = self.max_pic_order_cnt_lsb as i32;

            if pic_order_cnt_lsb < prev_poc_lsb
                && (prev_poc_lsb - pic_order_cnt_lsb) >= (max_poc_lsb / 2)
            {
                self.prev_poc_msb + max_poc_lsb
            } else {
                self.prev_poc_msb
            }
        };

        let top_field_order_cnt = pic_order_cnt_msb + pic_order_cnt_lsb;

        let bottom_field_order_cnt = if sps.frame_mbs_only {
            top_field_order_cnt
        } else if slice.field_pic_flag {
            if slice.bottom_field_flag {
                top_field_order_cnt + slice.delta_pic_order_cnt_bottom.unwrap_or(0)
            } else {
                top_field_order_cnt
            }
        } else {
            top_field_order_cnt + slice.delta_pic_order_cnt_bottom.unwrap_or(0)
        };

        if !slice.field_pic_flag || !slice.bottom_field_flag {
            self.prev_poc_msb = pic_order_cnt_msb;
            self.prev_poc_lsb = pic_order_cnt_lsb as u32;
        }

        let frame_poc = top_field_order_cnt.min(bottom_field_order_cnt);
        Ok((frame_poc, frame_poc))
    }

    fn calculate_type_1(
        &mut self,
        slice: &SliceHeader,
        sps: &Sps,
        max_frame_num: u32,
    ) -> Result<(i32, i32), PocError> {
        let frame_num_offset = if slice.prefix.is_idr {
            0
        } else if self.prev_frame_num > slice.frame_num {
            self.prev_frame_num_offset + max_frame_num as i32
        } else {
            self.prev_frame_num_offset
        };

        let abs_frame_num = if sps.max_num_ref_frames > 0 {
            frame_num_offset + slice.frame_num as i32
        } else {
            0
        };

        let poc = abs_frame_num * 2;

        self.prev_frame_num = slice.frame_num;
        self.prev_frame_num_offset = frame_num_offset;

        Ok((poc, poc))
    }

    fn calculate_type_2(
        &mut self,
        slice: &SliceHeader,
        max_frame_num: u32,
    ) -> Result<(i32, i32), PocError> {
        let frame_num_offset = if slice.prefix.is_idr {
            0
        } else if self.prev_frame_num > slice.frame_num {
            self.prev_frame_num_offset + max_frame_num as i32
        } else {
            self.prev_frame_num_offset
        };

        let temp_poc = if slice.prefix.is_idr {
            0
        } else {
            2 * (frame_num_offset + slice.frame_num as i32)
        };

        self.prev_frame_num = slice.frame_num;
        self.prev_frame_num_offset = frame_num_offset;

        Ok((temp_poc, temp_poc))
    }

    pub fn reset(&mut self) {
        self.prev_poc_msb = 0;
        self.prev_poc_lsb = 0;
        self.prev_frame_num = 0;
        self.prev_frame_num_offset = 0;
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PocError {
    MissingPocLsb,
    UnsupportedPocType,
}

impl std::fmt::Display for PocError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::MissingPocLsb => write!(f, "Missing pic_order_cnt_lsb in slice header"),
            Self::UnsupportedPocType => write!(f, "Unsupported POC type"),
        }
    }
}

impl std::error::Error for PocError {}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::video::h264::slice::SliceHeaderPrefix;

    fn make_sps(poc_type: u32, log2_max_poc_lsb: u32) -> Sps {
        Sps {
            profile_idc: 66,
            level_idc: 30,
            id: 0,
            width: 1920,
            height: 1080,
            chroma_format_idc: 1,
            bit_depth_luma: 8,
            bit_depth_chroma: 8,
            frame_mbs_only: true,
            max_num_ref_frames: 4,
            separate_colour_plane: false,
            log2_max_frame_num: 4,
            pic_order_cnt_type: poc_type,
            log2_max_pic_order_cnt_lsb: log2_max_poc_lsb,
            delta_pic_order_always_zero: false,
            mb_adaptive_frame_field: false,
            pic_width_in_mbs: 120,
            pic_height_in_map_units: 68,
        }
    }

    fn make_slice(is_idr: bool, frame_num: u32, poc_lsb: Option<u32>) -> SliceHeader {
        SliceHeader {
            slice_data_bit_offset: 0,
            prefix: SliceHeaderPrefix {
                first_mb_in_slice: 0,
                slice_type: crate::video::h264::slice::SliceType::I,
                pps_id: 0,
                is_idr,
            },
            colour_plane_id: None,
            frame_num,
            field_pic_flag: false,
            bottom_field_flag: false,
            idr_pic_id: if is_idr { Some(0) } else { None },
            pic_order_cnt_lsb: poc_lsb,
            delta_pic_order_cnt_bottom: None,
            delta_pic_order_cnt: None,
            redundant_pic_cnt: None,
            direct_spatial_mv_pred_flag: None,
            num_ref_idx_l0_active: 0,
            num_ref_idx_l1_active: 0,
            ref_pic_list_modification_l0: vec![],
            ref_pic_list_modification_l1: vec![],
            pred_weight_table: None,
            dec_ref_pic_marking: None,
            slice_qp_delta: 0,
            sp_for_switch_flag: None,
            slice_qs_delta: None,
            deblocking_filter: None,
            slice_group_change_cycle: None,
        }
    }

    #[test]
    fn calculates_poc_type_0_for_idr() {
        let sps = make_sps(0, 4);
        let mut calc = PocCalculator::new(&sps);

        let slice = make_slice(true, 0, Some(0));
        let (top, bottom) = calc.calculate(&slice, &sps, 16).unwrap();

        assert_eq!(top, 0);
        assert_eq!(bottom, 0);
    }

    #[test]
    fn calculates_poc_type_0_increasing_sequence() {
        let sps = make_sps(0, 4);
        let mut calc = PocCalculator::new(&sps);

        let slice0 = make_slice(true, 0, Some(0));
        let (poc0, _) = calc.calculate(&slice0, &sps, 16).unwrap();
        assert_eq!(poc0, 0);

        let slice1 = make_slice(false, 1, Some(2));
        let (poc1, _) = calc.calculate(&slice1, &sps, 16).unwrap();
        assert_eq!(poc1, 2);

        let slice2 = make_slice(false, 2, Some(4));
        let (poc2, _) = calc.calculate(&slice2, &sps, 16).unwrap();
        assert_eq!(poc2, 4);
    }

    #[test]
    fn calculates_poc_type_0_with_b_frames() {
        let sps = make_sps(0, 4);
        let mut calc = PocCalculator::new(&sps);

        let slice0 = make_slice(true, 0, Some(0));
        calc.calculate(&slice0, &sps, 16).unwrap();

        let slice1 = make_slice(false, 2, Some(4));
        let (poc1, _) = calc.calculate(&slice1, &sps, 16).unwrap();
        assert_eq!(poc1, 4);

        let slice2 = make_slice(false, 1, Some(2));
        let (poc2, _) = calc.calculate(&slice2, &sps, 16).unwrap();
        assert_eq!(poc2, 2);
    }

    #[test]
    fn handles_poc_lsb_wraparound() {
        let sps = make_sps(0, 4);
        let mut calc = PocCalculator::new(&sps);

        let slice0 = make_slice(true, 0, Some(0));
        let (poc0, _) = calc.calculate(&slice0, &sps, 16).unwrap();
        assert_eq!(poc0, 0);

        let slice1 = make_slice(false, 1, Some(2));
        let (poc1, _) = calc.calculate(&slice1, &sps, 16).unwrap();
        assert_eq!(poc1, 2);

        let slice2 = make_slice(false, 2, Some(14));
        let (poc2, _) = calc.calculate(&slice2, &sps, 16).unwrap();
        assert_eq!(poc2, 14);

        let slice3 = make_slice(false, 3, Some(2));
        let (poc3, _) = calc.calculate(&slice3, &sps, 16).unwrap();
        assert_eq!(poc3, 18);
    }

    #[test]
    fn calculates_poc_type_2() {
        let sps = make_sps(2, 0);
        let mut calc = PocCalculator::new(&sps);

        let slice0 = make_slice(true, 0, None);
        let (poc0, _) = calc.calculate(&slice0, &sps, 16).unwrap();
        assert_eq!(poc0, 0);

        let slice1 = make_slice(false, 1, None);
        let (poc1, _) = calc.calculate(&slice1, &sps, 16).unwrap();
        assert_eq!(poc1, 2);

        let slice2 = make_slice(false, 2, None);
        let (poc2, _) = calc.calculate(&slice2, &sps, 16).unwrap();
        assert_eq!(poc2, 4);
    }

    #[test]
    fn resets_state_correctly() {
        let sps = make_sps(0, 4);
        let mut calc = PocCalculator::new(&sps);

        let slice1 = make_slice(false, 5, Some(10));
        calc.calculate(&slice1, &sps, 16).unwrap();

        calc.reset();

        let slice2 = make_slice(true, 0, Some(0));
        let (poc, _) = calc.calculate(&slice2, &sps, 16).unwrap();
        assert_eq!(poc, 0);
    }
}

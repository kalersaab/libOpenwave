use crate::video::{
    bitstream::{BitReader, BitstreamError},
    h264::{pps::Pps, sps::Sps},
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SliceError {
    InvalidNalType,
    InvalidSliceType,
    InvalidPpsId,
    InvalidParameter,
    ParameterSetMismatch,
    UnsupportedFeature,
    Bitstream(BitstreamError),
}

impl std::fmt::Display for SliceError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "H.264 slice error: {self:?}")
    }
}

impl std::error::Error for SliceError {}

impl From<BitstreamError> for SliceError {
    fn from(value: BitstreamError) -> Self {
        Self::Bitstream(value)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SliceType {
    P,
    B,
    I,
    Sp,
    Si,
}

impl TryFrom<u32> for SliceType {
    type Error = SliceError;

    fn try_from(value: u32) -> Result<Self, Self::Error> {
        match value {
            0 | 5 => Ok(Self::P),
            1 | 6 => Ok(Self::B),
            2 | 7 => Ok(Self::I),
            3 | 8 => Ok(Self::Sp),
            4 | 9 => Ok(Self::Si),
            _ => Err(SliceError::InvalidSliceType),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SliceHeaderPrefix {
    pub first_mb_in_slice: u32,
    pub slice_type: SliceType,
    pub pps_id: u32,
    pub is_idr: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RefPicListModification {
    SubtractShortTermPicNum(u32),
    AddShortTermPicNum(u32),
    LongTermPicNum(u32),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PredWeight {
    pub luma: Option<(i32, i32)>,
    pub chroma: Option<[(i32, i32); 2]>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PredWeightTable {
    pub luma_log2_weight_denom: u32,
    pub chroma_log2_weight_denom: Option<u32>,
    pub list0: Vec<PredWeight>,
    pub list1: Vec<PredWeight>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MemoryManagementOperation {
    ForgetShortTermPicNum(u32),
    ForgetLongTermPicNum(u32),
    AssignLongTermFrameIdx { difference: u32, frame_idx: u32 },
    SetMaxLongTermFrameIdx(u32),
    Reset,
    MarkCurrentAsLongTerm(u32),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DecRefPicMarking {
    pub no_output_of_prior_pics_flag: Option<bool>,
    pub long_term_reference_flag: Option<bool>,
    pub adaptive_ref_pic_marking_mode_flag: bool,
    pub operations: Vec<MemoryManagementOperation>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DeblockingFilterParameters {
    pub disable_deblocking_filter_idc: u32,
    pub alpha_c0_offset_div2: Option<i32>,
    pub beta_offset_div2: Option<i32>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SliceHeader {
    pub slice_data_bit_offset: usize,
    pub prefix: SliceHeaderPrefix,
    pub colour_plane_id: Option<u8>,
    pub frame_num: u32,
    pub field_pic_flag: bool,
    pub bottom_field_flag: bool,
    pub idr_pic_id: Option<u32>,
    pub pic_order_cnt_lsb: Option<u32>,
    pub delta_pic_order_cnt_bottom: Option<i32>,
    pub delta_pic_order_cnt: Option<[i32; 2]>,
    pub redundant_pic_cnt: Option<u32>,
    pub direct_spatial_mv_pred_flag: Option<bool>,
    pub num_ref_idx_l0_active: u32,
    pub num_ref_idx_l1_active: u32,
    pub ref_pic_list_modification_l0: Vec<RefPicListModification>,
    pub ref_pic_list_modification_l1: Vec<RefPicListModification>,
    pub pred_weight_table: Option<PredWeightTable>,
    pub dec_ref_pic_marking: Option<DecRefPicMarking>,
    pub slice_qp_delta: i32,
    pub sp_for_switch_flag: Option<bool>,
    pub slice_qs_delta: Option<i32>,
    pub deblocking_filter: Option<DeblockingFilterParameters>,
    pub slice_group_change_cycle: Option<u32>,
}

fn ebsp_to_rbsp(ebsp: &[u8]) -> Vec<u8> {
    let mut rbsp = Vec::with_capacity(ebsp.len());
    let mut zeros = 0usize;

    for &byte in ebsp {
        if zeros >= 2 && byte == 3 {
            zeros = 0;
            continue;
        }

        rbsp.push(byte);

        if byte == 0 {
            zeros += 1;
        } else {
            zeros = 0;
        }
    }

    rbsp
}

fn parse_prefix_fields(
    reader: &mut BitReader<'_>,
    is_idr: bool,
) -> Result<SliceHeaderPrefix, SliceError> {
    let first_mb_in_slice = reader.read_ue()?;
    let raw_slice_type = reader.read_ue()?;

    if raw_slice_type > 9 {
        return Err(SliceError::InvalidSliceType);
    }

    let pps_id = reader.read_ue()?;
    if pps_id > 255 {
        return Err(SliceError::InvalidPpsId);
    }

    Ok(SliceHeaderPrefix {
        first_mb_in_slice,
        slice_type: SliceType::try_from(raw_slice_type)?,
        pps_id,
        is_idr,
    })
}

fn read_ref_pic_list_modification(
    reader: &mut BitReader<'_>,
) -> Result<Vec<RefPicListModification>, SliceError> {
    let mut modifications = Vec::new();
    if !reader.read_bit()? {
        return Ok(modifications);
    }

    for _ in 0..64 {
        match reader.read_ue()? {
            0 => modifications.push(RefPicListModification::SubtractShortTermPicNum(
                reader.read_ue()?,
            )),
            1 => modifications.push(RefPicListModification::AddShortTermPicNum(
                reader.read_ue()?,
            )),
            2 => modifications.push(RefPicListModification::LongTermPicNum(reader.read_ue()?)),
            3 => return Ok(modifications),
            _ => return Err(SliceError::InvalidParameter),
        }
    }

    Err(SliceError::InvalidParameter)
}

fn read_pred_weight(
    reader: &mut BitReader<'_>,
    chroma_array_type: u32,
) -> Result<PredWeight, SliceError> {
    let luma = if reader.read_bit()? {
        Some((reader.read_se()?, reader.read_se()?))
    } else {
        None
    };

    let chroma = if chroma_array_type != 0 && reader.read_bit()? {
        Some([
            (reader.read_se()?, reader.read_se()?),
            (reader.read_se()?, reader.read_se()?),
        ])
    } else {
        None
    };

    Ok(PredWeight { luma, chroma })
}

fn read_pred_weight_table(
    reader: &mut BitReader<'_>,
    chroma_array_type: u32,
    l0_count: u32,
    l1_count: u32,
    is_b_slice: bool,
) -> Result<PredWeightTable, SliceError> {
    let luma_log2_weight_denom = reader.read_ue()?;
    if luma_log2_weight_denom > 7 {
        return Err(SliceError::InvalidParameter);
    }

    let chroma_log2_weight_denom = if chroma_array_type != 0 {
        let value = reader.read_ue()?;
        if value > 7 {
            return Err(SliceError::InvalidParameter);
        }
        Some(value)
    } else {
        None
    };

    let mut list0 = Vec::with_capacity(l0_count as usize);
    for _ in 0..l0_count {
        list0.push(read_pred_weight(reader, chroma_array_type)?);
    }

    let mut list1 = Vec::new();
    if is_b_slice {
        list1.reserve(l1_count as usize);
        for _ in 0..l1_count {
            list1.push(read_pred_weight(reader, chroma_array_type)?);
        }
    }

    Ok(PredWeightTable {
        luma_log2_weight_denom,
        chroma_log2_weight_denom,
        list0,
        list1,
    })
}

fn read_dec_ref_pic_marking(
    reader: &mut BitReader<'_>,
    is_idr: bool,
) -> Result<DecRefPicMarking, SliceError> {
    if is_idr {
        return Ok(DecRefPicMarking {
            no_output_of_prior_pics_flag: Some(reader.read_bit()?),
            long_term_reference_flag: Some(reader.read_bit()?),
            adaptive_ref_pic_marking_mode_flag: false,
            operations: Vec::new(),
        });
    }

    let adaptive_ref_pic_marking_mode_flag = reader.read_bit()?;
    let mut operations = Vec::new();
    if adaptive_ref_pic_marking_mode_flag {
        for _ in 0..64 {
            match reader.read_ue()? {
                0 => {
                    return Ok(DecRefPicMarking {
                        no_output_of_prior_pics_flag: None,
                        long_term_reference_flag: None,
                        adaptive_ref_pic_marking_mode_flag,
                        operations,
                    });
                }
                1 => operations.push(MemoryManagementOperation::ForgetShortTermPicNum(
                    reader.read_ue()?,
                )),
                2 => operations.push(MemoryManagementOperation::ForgetLongTermPicNum(
                    reader.read_ue()?,
                )),
                3 => operations.push(MemoryManagementOperation::AssignLongTermFrameIdx {
                    difference: reader.read_ue()?,
                    frame_idx: reader.read_ue()?,
                }),
                4 => operations.push(MemoryManagementOperation::SetMaxLongTermFrameIdx(
                    reader.read_ue()?,
                )),
                5 => operations.push(MemoryManagementOperation::Reset),
                6 => operations.push(MemoryManagementOperation::MarkCurrentAsLongTerm(
                    reader.read_ue()?,
                )),
                _ => return Err(SliceError::InvalidParameter),
            }
        }
        return Err(SliceError::InvalidParameter);
    }

    Ok(DecRefPicMarking {
        no_output_of_prior_pics_flag: None,
        long_term_reference_flag: None,
        adaptive_ref_pic_marking_mode_flag,
        operations,
    })
}

fn parse_nal_header(nal: &[u8]) -> Result<(u8, u8), SliceError> {
    let header = *nal.first().ok_or(SliceError::InvalidNalType)?;
    if header & 0x80 != 0 {
        return Err(SliceError::InvalidNalType);
    }

    let nal_type = header & 0x1f;
    if nal_type != 1 && nal_type != 5 {
        return Err(SliceError::InvalidNalType);
    }

    Ok(((header >> 5) & 0x03, nal_type))
}

/// Parses the full slice header for CAVLC streams using their active parameter sets.
/// The returned header ends immediately before slice data.
pub fn parse_slice_header(nal: &[u8], sps: &Sps, pps: &Pps) -> Result<SliceHeader, SliceError> {
    let (nal_ref_idc, nal_type) = parse_nal_header(nal)?;
    let is_idr = nal_type == 5;

    if pps.entropy_coding_mode {
        return Err(SliceError::UnsupportedFeature);
    }
    if pps.id > 255 || pps.sps_id != sps.id {
        return Err(SliceError::ParameterSetMismatch);
    }
    if pps.num_slice_groups == 0
        || pps.num_slice_groups > 1 && !matches!(pps.slice_group_map_type, Some(0..=6))
    {
        return Err(SliceError::UnsupportedFeature);
    }

    let rbsp = ebsp_to_rbsp(&nal[1..]);
    let mut reader = BitReader::new(&rbsp);
    let prefix = parse_prefix_fields(&mut reader, is_idr)?;
    if prefix.pps_id != pps.id {
        return Err(SliceError::ParameterSetMismatch);
    }

    let chroma_array_type = if sps.separate_colour_plane {
        0
    } else {
        sps.chroma_format_idc
    };
    let colour_plane_id = if sps.separate_colour_plane {
        let value = reader.read_bits(2)? as u8;
        if value > 2 {
            return Err(SliceError::InvalidParameter);
        }
        Some(value)
    } else {
        None
    };

    if !(4..=16).contains(&sps.log2_max_frame_num) {
        return Err(SliceError::InvalidParameter);
    }
    let frame_num = reader.read_bits(sps.log2_max_frame_num)?;
    let field_pic_flag = if sps.frame_mbs_only {
        false
    } else {
        reader.read_bit()?
    };
    let bottom_field_flag = field_pic_flag && reader.read_bit()?;
    let idr_pic_id = if is_idr {
        Some(reader.read_ue()?)
    } else {
        None
    };

    let mut pic_order_cnt_lsb = None;
    let mut delta_pic_order_cnt_bottom = None;
    let mut delta_pic_order_cnt = None;
    match sps.pic_order_cnt_type {
        0 => {
            if !(4..=16).contains(&sps.log2_max_pic_order_cnt_lsb) {
                return Err(SliceError::InvalidParameter);
            }
            pic_order_cnt_lsb = Some(reader.read_bits(sps.log2_max_pic_order_cnt_lsb)?);
            if pps.bottom_field_pic_order_in_frame_present && !field_pic_flag {
                delta_pic_order_cnt_bottom = Some(reader.read_se()?);
            }
        }
        1 if !sps.delta_pic_order_always_zero => {
            let first = reader.read_se()?;
            let second = if pps.bottom_field_pic_order_in_frame_present && !field_pic_flag {
                reader.read_se()?
            } else {
                0
            };
            delta_pic_order_cnt = Some([first, second]);
        }
        1 | 2 => {}
        _ => return Err(SliceError::InvalidParameter),
    }

    let redundant_pic_cnt = if pps.redundant_pic_cnt_present {
        Some(reader.read_ue()?)
    } else {
        None
    };
    let is_b_slice = prefix.slice_type == SliceType::B;
    let is_i_or_si_slice = matches!(prefix.slice_type, SliceType::I | SliceType::Si);
    let direct_spatial_mv_pred_flag = if is_b_slice {
        Some(reader.read_bit()?)
    } else {
        None
    };

    let (mut num_ref_idx_l0_active, mut num_ref_idx_l1_active) = (
        pps.num_ref_idx_l0_default_active,
        pps.num_ref_idx_l1_default_active,
    );
    if !is_i_or_si_slice && reader.read_bit()? {
        num_ref_idx_l0_active = reader
            .read_ue()?
            .checked_add(1)
            .ok_or(SliceError::InvalidParameter)?;
        if is_b_slice {
            num_ref_idx_l1_active = reader
                .read_ue()?
                .checked_add(1)
                .ok_or(SliceError::InvalidParameter)?;
        }
        if num_ref_idx_l0_active > 32 || num_ref_idx_l1_active > 32 {
            return Err(SliceError::InvalidParameter);
        }
    }
    if num_ref_idx_l0_active == 0
        || num_ref_idx_l0_active > 32
        || !is_b_slice && num_ref_idx_l1_active > 32
        || is_b_slice && num_ref_idx_l1_active == 0
    {
        return Err(SliceError::InvalidParameter);
    }

    let (ref_pic_list_modification_l0, ref_pic_list_modification_l1) = if is_i_or_si_slice {
        (Vec::new(), Vec::new())
    } else {
        let l0 = read_ref_pic_list_modification(&mut reader)?;
        let l1 = if is_b_slice {
            read_ref_pic_list_modification(&mut reader)?
        } else {
            Vec::new()
        };
        (l0, l1)
    };

    let weighted_pred =
        pps.weighted_pred && matches!(prefix.slice_type, SliceType::P | SliceType::Sp);
    let weighted_bipred = is_b_slice && pps.weighted_bipred_idc == 1;
    let pred_weight_table = if weighted_pred || weighted_bipred {
        Some(read_pred_weight_table(
            &mut reader,
            chroma_array_type,
            num_ref_idx_l0_active,
            num_ref_idx_l1_active,
            is_b_slice,
        )?)
    } else {
        None
    };

    let dec_ref_pic_marking = if nal_ref_idc != 0 {
        Some(read_dec_ref_pic_marking(&mut reader, is_idr)?)
    } else {
        None
    };
    let slice_qp_delta = reader.read_se()?;
    let qp_y = 26i64 + i64::from(pps.pic_init_qp_minus26) + i64::from(slice_qp_delta);
    if !(0..=51).contains(&qp_y) {
        return Err(SliceError::InvalidParameter);
    }

    let sp_for_switch_flag = if prefix.slice_type == SliceType::Sp {
        Some(reader.read_bit()?)
    } else {
        None
    };
    let slice_qs_delta = if matches!(prefix.slice_type, SliceType::Sp | SliceType::Si) {
        let delta = reader.read_se()?;
        let qs_y = 26i64 + i64::from(pps.pic_init_qs_minus26) + i64::from(delta);
        if !(0..=51).contains(&qs_y) {
            return Err(SliceError::InvalidParameter);
        }
        Some(delta)
    } else {
        None
    };

    let deblocking_filter = if pps.deblocking_filter_control_present {
        let disable_deblocking_filter_idc = reader.read_ue()?;
        if disable_deblocking_filter_idc > 2 {
            return Err(SliceError::InvalidParameter);
        }
        let (alpha_c0_offset_div2, beta_offset_div2) = if disable_deblocking_filter_idc != 1 {
            let alpha = reader.read_se()?;
            let beta = reader.read_se()?;
            if !(-6..=6).contains(&alpha) || !(-6..=6).contains(&beta) {
                return Err(SliceError::InvalidParameter);
            }
            (Some(alpha), Some(beta))
        } else {
            (None, None)
        };
        Some(DeblockingFilterParameters {
            disable_deblocking_filter_idc,
            alpha_c0_offset_div2,
            beta_offset_div2,
        })
    } else {
        None
    };

    let slice_group_change_cycle = match (pps.num_slice_groups, pps.slice_group_map_type) {
        (1, _) | (_, Some(0..=2 | 6)) | (_, None) => None,
        (_, Some(3..=5)) => {
            if pps.slice_group_change_rate == 0
                || sps.pic_width_in_mbs == 0
                || sps.pic_height_in_map_units == 0
            {
                return Err(SliceError::InvalidParameter);
            }
            let pic_size = sps
                .pic_width_in_mbs
                .checked_mul(sps.pic_height_in_map_units)
                .ok_or(SliceError::InvalidParameter)?;
            let units = pic_size.div_ceil(pps.slice_group_change_rate);
            let bits = u32::BITS - units.leading_zeros();
            Some(reader.read_bits(bits)?)
        }
        _ => return Err(SliceError::UnsupportedFeature),
    };

    Ok(SliceHeader {
        slice_data_bit_offset: reader.bit_position(),
        prefix,
        colour_plane_id,
        frame_num,
        field_pic_flag,
        bottom_field_flag,
        idr_pic_id,
        pic_order_cnt_lsb,
        delta_pic_order_cnt_bottom,
        delta_pic_order_cnt,
        redundant_pic_cnt,
        direct_spatial_mv_pred_flag,
        num_ref_idx_l0_active,
        num_ref_idx_l1_active,
        ref_pic_list_modification_l0,
        ref_pic_list_modification_l1,
        pred_weight_table,
        dec_ref_pic_marking,
        slice_qp_delta,
        sp_for_switch_flag,
        slice_qs_delta,
        deblocking_filter,
        slice_group_change_cycle,
    })
}

pub fn parse_slice_prefix(nal: &[u8]) -> Result<SliceHeaderPrefix, SliceError> {
    let (_, nal_type) = parse_nal_header(nal)?;
    let rbsp = ebsp_to_rbsp(&nal[1..]);
    let mut reader = BitReader::new(&rbsp);
    parse_prefix_fields(&mut reader, nal_type == 5)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::video::h264::{pps::Pps, sps::Sps};

    #[derive(Default)]
    struct TestBitWriter {
        bits: Vec<bool>,
    }

    impl TestBitWriter {
        fn bit(&mut self, bit: bool) {
            self.bits.push(bit);
        }

        fn bits(&mut self, count: u32, value: u32) {
            for shift in (0..count).rev() {
                self.bit(((value >> shift) & 1) != 0);
            }
        }

        fn ue(&mut self, value: u32) {
            let code = u64::from(value) + 1;
            let width = 64 - code.leading_zeros();
            for _ in 1..width {
                self.bit(false);
            }
            self.bits(width, code as u32);
        }

        fn se(&mut self, value: i32) {
            let code = if value <= 0 {
                value.unsigned_abs() * 2
            } else {
                value as u32 * 2 - 1
            };
            self.ue(code);
        }

        fn finish(self, nal_header: u8) -> Vec<u8> {
            let mut bits = self.bits;
            bits.push(true);
            while !bits.len().is_multiple_of(8) {
                bits.push(false);
            }

            let mut nal = vec![nal_header];
            let (chunks, _) = bits.as_chunks::<8>();
            for chunk in chunks {
                let mut byte = 0u8;
                for &bit in chunk {
                    byte = (byte << 1) | u8::from(bit);
                }
                nal.push(byte);
            }
            nal
        }
    }

    fn test_sps(pic_order_cnt_type: u32, frame_mbs_only: bool) -> Sps {
        Sps {
            profile_idc: 66,
            level_idc: 30,
            id: 0,
            width: 32,
            height: 32,
            chroma_format_idc: 1,
            bit_depth_luma: 8,
            bit_depth_chroma: 8,
            frame_mbs_only,
            max_num_ref_frames: 2,
            separate_colour_plane: false,
            log2_max_frame_num: 4,
            pic_order_cnt_type,
            log2_max_pic_order_cnt_lsb: if pic_order_cnt_type == 0 { 4 } else { 0 },
            delta_pic_order_always_zero: false,
            mb_adaptive_frame_field: !frame_mbs_only,
            pic_width_in_mbs: 2,
            pic_height_in_map_units: 2,
        }
    }

    fn test_pps() -> Pps {
        Pps {
            id: 0,
            sps_id: 0,
            entropy_coding_mode: false,
            bottom_field_pic_order_in_frame_present: false,
            num_slice_groups: 1,
            slice_group_map_type: None,
            slice_group_change_rate: 0,
            num_ref_idx_l0_default_active: 1,
            num_ref_idx_l1_default_active: 1,
            weighted_pred: false,
            weighted_bipred_idc: 0,
            pic_init_qp_minus26: 0,
            pic_init_qs_minus26: 0,
            chroma_qp_index_offset: 0,
            deblocking_filter_control_present: true,
            constrained_intra_pred: false,
            redundant_pic_cnt_present: false,
        }
    }

    // Encode unsigned Exp-Golomb values MSB-first.
    fn ue_bits(value: u32, out: &mut Vec<bool>) {
        let code = u64::from(value) + 1;
        let width = 64 - code.leading_zeros();

        out.extend(std::iter::repeat_n(false, (width - 1) as usize));

        for shift in (0..width).rev() {
            out.push(((code >> shift) & 1) != 0);
        }
    }

    fn make_nal(nal_type: u8, values: &[u32]) -> Vec<u8> {
        let mut bits = Vec::new();

        for &value in values {
            ue_bits(value, &mut bits);
        }

        // Add RBSP trailing bits.
        bits.push(true);
        while !bits.len().is_multiple_of(8) {
            bits.push(false);
        }

        let mut nal = vec![nal_type];
        let (chunks, _) = bits.as_chunks::<8>();
        for chunk in chunks {
            let mut byte = 0u8;
            for &bit in chunk {
                byte = (byte << 1) | u8::from(bit);
            }
            nal.push(byte);
        }

        nal
    }

    #[test]
    fn parses_p_slice_prefix() {
        let nal = make_nal(0x41, &[0, 0, 3]);

        let parsed = parse_slice_prefix(&nal).unwrap();

        assert_eq!(parsed.first_mb_in_slice, 0);
        assert_eq!(parsed.slice_type, SliceType::P);
        assert_eq!(parsed.pps_id, 3);
        assert!(!parsed.is_idr);
    }

    #[test]
    fn parses_idr_slice_prefix() {
        let nal = make_nal(0x65, &[0, 1, 0]);

        let parsed = parse_slice_prefix(&nal).unwrap();

        assert_eq!(parsed.slice_type, SliceType::B);
        assert!(parsed.is_idr);
    }

    #[test]
    fn accepts_all_slice_type_values() {
        for raw in 0..=9 {
            assert!(SliceType::try_from(raw).is_ok());
        }

        assert_eq!(SliceType::try_from(10), Err(SliceError::InvalidSliceType));
    }

    #[test]
    fn rejects_sps_nal() {
        assert_eq!(
            parse_slice_prefix(&[0x67, 0x42, 0, 0]),
            Err(SliceError::InvalidNalType)
        );
    }

    #[test]
    fn parses_idr_i_slice_header() {
        let mut writer = TestBitWriter::default();
        writer.ue(0); // first_mb_in_slice
        writer.ue(2); // I slice
        writer.ue(0); // pic_parameter_set_id
        writer.bits(4, 5); // frame_num
        writer.ue(3); // idr_pic_id
        writer.bits(4, 7); // pic_order_cnt_lsb
        writer.bit(false); // no_output_of_prior_pics_flag
        writer.bit(true); // long_term_reference_flag
        writer.se(0); // slice_qp_delta
        writer.ue(0); // disable_deblocking_filter_idc
        writer.se(-1);
        writer.se(1);
        let nal = writer.finish(0x65);

        let header = parse_slice_header(&nal, &test_sps(0, true), &test_pps()).unwrap();

        assert_eq!(header.frame_num, 5);
        assert_eq!(header.idr_pic_id, Some(3));
        assert_eq!(header.pic_order_cnt_lsb, Some(7));
        assert_eq!(
            header
                .dec_ref_pic_marking
                .as_ref()
                .unwrap()
                .long_term_reference_flag,
            Some(true)
        );
        assert_eq!(
            header.deblocking_filter,
            Some(DeblockingFilterParameters {
                disable_deblocking_filter_idc: 0,
                alpha_c0_offset_div2: Some(-1),
                beta_offset_div2: Some(1),
            })
        );
    }

    #[test]
    fn parses_p_slice_modifications_weights_and_marking() {
        let mut pps = test_pps();
        pps.bottom_field_pic_order_in_frame_present = true;
        pps.redundant_pic_cnt_present = true;
        pps.weighted_pred = true;

        let mut writer = TestBitWriter::default();
        writer.ue(0); // first_mb_in_slice
        writer.ue(0); // P slice
        writer.ue(0); // pic_parameter_set_id
        writer.bits(4, 9); // frame_num
        writer.bits(4, 3); // pic_order_cnt_lsb
        writer.se(-2); // delta_pic_order_cnt_bottom
        writer.ue(1); // redundant_pic_cnt
        writer.bit(true); // num_ref_idx_active_override_flag
        writer.ue(0); // num_ref_idx_l0_active_minus1
        writer.bit(true); // ref_pic_list_modification_flag_l0
        writer.ue(0); // subtract short-term pic_num
        writer.ue(3); // abs_diff_pic_num_minus1
        writer.ue(3); // modification_of_pic_nums_idc terminator
        writer.ue(1); // luma_log2_weight_denom
        writer.ue(2); // chroma_log2_weight_denom
        writer.bit(true); // luma_weight_l0_flag
        writer.se(1);
        writer.se(-1);
        writer.bit(true); // chroma_weight_l0_flag
        writer.se(0);
        writer.se(1);
        writer.se(-1);
        writer.se(0);
        writer.bit(true); // adaptive_ref_pic_marking_mode_flag
        writer.ue(1); // memory_management_control_operation
        writer.ue(2); // difference_of_pic_nums_minus1
        writer.ue(0); // end of operations
        writer.se(-1); // slice_qp_delta
        writer.ue(1); // disable_deblocking_filter_idc
        let nal = writer.finish(0x41);

        let header = parse_slice_header(&nal, &test_sps(0, true), &pps).unwrap();

        assert_eq!(header.frame_num, 9);
        assert_eq!(header.delta_pic_order_cnt_bottom, Some(-2));
        assert_eq!(header.redundant_pic_cnt, Some(1));
        assert_eq!(
            header.ref_pic_list_modification_l0,
            vec![RefPicListModification::SubtractShortTermPicNum(3)]
        );
        let weights = header.pred_weight_table.unwrap();
        assert_eq!(weights.luma_log2_weight_denom, 1);
        assert_eq!(weights.chroma_log2_weight_denom, Some(2));
        assert_eq!(weights.list0[0].luma, Some((1, -1)));
        assert_eq!(weights.list0[0].chroma, Some([(0, 1), (-1, 0)]));
        assert_eq!(
            header.dec_ref_pic_marking.unwrap().operations,
            vec![MemoryManagementOperation::ForgetShortTermPicNum(2)]
        );
        assert_eq!(header.slice_qp_delta, -1);
        assert_eq!(
            header
                .deblocking_filter
                .unwrap()
                .disable_deblocking_filter_idc,
            1
        );
    }

    #[test]
    fn parses_b_slice_list1_and_bipred_weights() {
        let mut pps = test_pps();
        pps.weighted_bipred_idc = 1;
        let mut writer = TestBitWriter::default();
        writer.ue(0); // first_mb_in_slice
        writer.ue(1); // B slice
        writer.ue(0); // pic_parameter_set_id
        writer.bits(4, 2); // frame_num
        writer.bit(true); // direct_spatial_mv_pred_flag
        writer.bit(false); // num_ref_idx_active_override_flag
        writer.bit(false); // ref_pic_list_modification_flag_l0
        writer.bit(true); // ref_pic_list_modification_flag_l1
        writer.ue(2); // long_term_pic_num
        writer.ue(4);
        writer.ue(3); // modification_of_pic_nums_idc terminator
        writer.ue(0); // luma_log2_weight_denom
        writer.ue(0); // chroma_log2_weight_denom
        writer.bit(false); // luma_weight_l0_flag
        writer.bit(false); // chroma_weight_l0_flag
        writer.bit(true); // luma_weight_l1_flag
        writer.se(2);
        writer.se(-1);
        writer.bit(false); // chroma_weight_l1_flag
        writer.bit(false); // adaptive_ref_pic_marking_mode_flag
        writer.se(0); // slice_qp_delta
        writer.ue(1); // disable_deblocking_filter_idc
        let nal = writer.finish(0x21);

        let header = parse_slice_header(&nal, &test_sps(2, true), &pps).unwrap();

        assert_eq!(header.direct_spatial_mv_pred_flag, Some(true));
        assert_eq!(
            header.ref_pic_list_modification_l1,
            vec![RefPicListModification::LongTermPicNum(4)]
        );
        let weights = header.pred_weight_table.unwrap();
        assert_eq!(weights.list0.len(), 1);
        assert_eq!(weights.list1[0].luma, Some((2, -1)));
    }

    #[test]
    fn parses_field_picture_and_type_one_poc() {
        let mut pps = test_pps();
        pps.bottom_field_pic_order_in_frame_present = true;
        let mut writer = TestBitWriter::default();
        writer.ue(0); // first_mb_in_slice
        writer.ue(2); // I slice
        writer.ue(0); // pic_parameter_set_id
        writer.bits(4, 4); // frame_num
        writer.bit(true); // field_pic_flag
        writer.bit(true); // bottom_field_flag
        writer.se(-3); // delta_pic_order_cnt[0]
        writer.se(0); // slice_qp_delta
        writer.ue(1); // disable_deblocking_filter_idc
        let nal = writer.finish(0x01);

        let header = parse_slice_header(&nal, &test_sps(1, false), &pps).unwrap();

        assert!(header.field_pic_flag);
        assert!(header.bottom_field_flag);
        assert_eq!(header.delta_pic_order_cnt, Some([-3, 0]));
    }

    #[test]
    fn rejects_cabac_parameter_set() {
        let mut pps = test_pps();
        pps.entropy_coding_mode = true;
        let nal = make_nal(0x41, &[0, 0, 0]);

        assert_eq!(
            parse_slice_header(&nal, &test_sps(0, true), &pps),
            Err(SliceError::UnsupportedFeature)
        );
    }
}

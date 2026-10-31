use crate::video::bitstream::{BitReader, BitstreamError};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SpsError {
    InvalidNalType,
    InvalidProfile,
    InvalidParameter,
    InvalidDimensions,
    Bitstream(BitstreamError),
}

impl std::fmt::Display for SpsError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "H.264 SPS error: {self:?}")
    }
}

impl std::error::Error for SpsError {}

impl From<BitstreamError> for SpsError {
    fn from(value: BitstreamError) -> Self {
        Self::Bitstream(value)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Sps {
    pub profile_idc: u8,
    pub level_idc: u8,
    pub id: u32,
    pub width: u32,
    pub height: u32,
    pub chroma_format_idc: u32,
    pub bit_depth_luma: u32,
    pub bit_depth_chroma: u32,
    pub frame_mbs_only: bool,
    pub max_num_ref_frames: u32,
    pub separate_colour_plane: bool,
    pub log2_max_frame_num: u32,
    pub pic_order_cnt_type: u32,
    pub log2_max_pic_order_cnt_lsb: u32,
    pub delta_pic_order_always_zero: bool,
    pub mb_adaptive_frame_field: bool,
    pub pic_width_in_mbs: u32,
    pub pic_height_in_map_units: u32,
}

fn read_flag(reader: &mut BitReader<'_>) -> Result<bool, SpsError> {
    Ok(reader.read_bit()?)
}

fn read_ue(reader: &mut BitReader<'_>) -> Result<u32, SpsError> {
    Ok(reader.read_ue()?)
}

fn skip_scaling_list(reader: &mut BitReader<'_>, size: usize) -> Result<(), SpsError> {
    let mut last_scale = 8i32;
    let mut next_scale = 8i32;

    for _ in 0..size {
        if next_scale != 0 {
            let delta_scale = reader.read_se()?;

            next_scale = (last_scale + delta_scale + 256) % 256;
        }

        if next_scale != 0 {
            last_scale = next_scale;
        }
    }

    Ok(())
}

fn is_high_profile(profile: u8) -> bool {
    matches!(
        profile,
        44 | 83 | 86 | 100 | 110 | 118 | 122 | 128 | 134 | 135 | 138 | 139 | 244
    )
}

pub fn parse_sps(nal: &[u8]) -> Result<Sps, SpsError> {
    let header = *nal.first().ok_or(SpsError::InvalidNalType)?;

    if header & 0x80 != 0 || header & 0x1f != 7 {
        return Err(SpsError::InvalidNalType);
    }

    let mut rbsp = Vec::with_capacity(nal.len().saturating_sub(1));
    let mut zero_count = 0usize;

    for &byte in &nal[1..] {
        if zero_count >= 2 && byte == 3 {
            zero_count = 0;
            continue;
        }

        rbsp.push(byte);

        if byte == 0 {
            zero_count += 1;
        } else {
            zero_count = 0;
        }
    }

    let mut reader = BitReader::new(&rbsp);

    let profile_idc = reader.read_bits(8)? as u8;
    let _constraint_flags = reader.read_bits(8)?;
    let level_idc = reader.read_bits(8)? as u8;
    let id = read_ue(&mut reader)?;

    if id > 31 {
        return Err(SpsError::InvalidParameter);
    }

    let mut chroma_format_idc = 1u32;
    let mut separate_colour_plane = false;
    let mut bit_depth_luma = 8u32;
    let mut bit_depth_chroma = 8u32;

    if is_high_profile(profile_idc) {
        chroma_format_idc = read_ue(&mut reader)?;

        if chroma_format_idc > 3 {
            return Err(SpsError::InvalidParameter);
        }

        if chroma_format_idc == 3 {
            separate_colour_plane = read_flag(&mut reader)?;
        }

        bit_depth_luma = read_ue(&mut reader)?
            .checked_add(8)
            .ok_or(SpsError::InvalidParameter)?;
        bit_depth_chroma = read_ue(&mut reader)?
            .checked_add(8)
            .ok_or(SpsError::InvalidParameter)?;

        if bit_depth_luma > 16 || bit_depth_chroma > 16 {
            return Err(SpsError::InvalidParameter);
        }

        let _qpprime_y_zero_transform_bypass = read_flag(&mut reader)?;
        let scaling_matrix_present = read_flag(&mut reader)?;

        if scaling_matrix_present {
            let count = if chroma_format_idc == 3 { 12 } else { 8 };

            for index in 0..count {
                if read_flag(&mut reader)? {
                    skip_scaling_list(&mut reader, if index < 6 { 16 } else { 64 })?;
                }
            }
        }
    }

    let log2_max_frame_num_minus4 = read_ue(&mut reader)?;
    if log2_max_frame_num_minus4 > 12 {
        return Err(SpsError::InvalidParameter);
    }
    let log2_max_frame_num = log2_max_frame_num_minus4 + 4;

    let pic_order_cnt_type = read_ue(&mut reader)?;
    let mut log2_max_pic_order_cnt_lsb = 0;
    let mut delta_pic_order_always_zero = false;

    match pic_order_cnt_type {
        0 => {
            let log2_max_pic_order_cnt_lsb_minus4 = read_ue(&mut reader)?;
            if log2_max_pic_order_cnt_lsb_minus4 > 12 {
                return Err(SpsError::InvalidParameter);
            }
            log2_max_pic_order_cnt_lsb = log2_max_pic_order_cnt_lsb_minus4 + 4;
        }
        1 => {
            delta_pic_order_always_zero = read_flag(&mut reader)?;
            let _offset_for_non_ref_pic = reader.read_se()?;
            let _offset_for_top_to_bottom_field = reader.read_se()?;
            let cycle = read_ue(&mut reader)?;

            if cycle > 255 {
                return Err(SpsError::InvalidParameter);
            }

            for _ in 0..cycle {
                let _offset_for_ref_frame = reader.read_se()?;
            }
        }
        2 => {}
        _ => return Err(SpsError::InvalidParameter),
    }

    let max_num_ref_frames = read_ue(&mut reader)?;
    let _gaps_in_frame_num_value_allowed = read_flag(&mut reader)?;

    let width_mbs = read_ue(&mut reader)?
        .checked_add(1)
        .ok_or(SpsError::InvalidDimensions)?;

    let height_map_units = read_ue(&mut reader)?
        .checked_add(1)
        .ok_or(SpsError::InvalidDimensions)?;

    let frame_mbs_only = read_flag(&mut reader)?;

    let mb_adaptive_frame_field = if frame_mbs_only {
        false
    } else {
        read_flag(&mut reader)?
    };

    let _direct_8x8_inference = read_flag(&mut reader)?;
    let cropping = read_flag(&mut reader)?;

    let (mut crop_left, mut crop_right, mut crop_top, mut crop_bottom) = (0u32, 0u32, 0u32, 0u32);

    if cropping {
        crop_left = read_ue(&mut reader)?;
        crop_right = read_ue(&mut reader)?;
        crop_top = read_ue(&mut reader)?;
        crop_bottom = read_ue(&mut reader)?;
    }

    let _vui_parameters_present = read_flag(&mut reader)?;

    let chroma_array_type = if separate_colour_plane {
        0
    } else {
        chroma_format_idc
    };

    let (sub_width, sub_height) = match chroma_array_type {
        0 => (1u32, 1u32),
        1 => (2, 2),
        2 => (2, 1),
        3 => (1, 1),
        _ => return Err(SpsError::InvalidParameter),
    };

    let frame_factor = if frame_mbs_only { 1u32 } else { 2u32 };

    let coded_width = width_mbs
        .checked_mul(16)
        .ok_or(SpsError::InvalidDimensions)?;

    let coded_height = height_map_units
        .checked_mul(16)
        .and_then(|v| v.checked_mul(frame_factor))
        .ok_or(SpsError::InvalidDimensions)?;

    let crop_unit_x = sub_width;
    let crop_unit_y = sub_height * frame_factor;

    let crop_x = crop_left
        .checked_add(crop_right)
        .and_then(|v| v.checked_mul(crop_unit_x))
        .ok_or(SpsError::InvalidDimensions)?;

    let crop_y = crop_top
        .checked_add(crop_bottom)
        .and_then(|v| v.checked_mul(crop_unit_y))
        .ok_or(SpsError::InvalidDimensions)?;

    let width = coded_width
        .checked_sub(crop_x)
        .filter(|v| *v > 0)
        .ok_or(SpsError::InvalidDimensions)?;

    let height = coded_height
        .checked_sub(crop_y)
        .filter(|v| *v > 0)
        .ok_or(SpsError::InvalidDimensions)?;

    Ok(Sps {
        profile_idc,
        level_idc,
        id,
        width,
        height,
        chroma_format_idc,
        bit_depth_luma,
        bit_depth_chroma,
        frame_mbs_only,
        max_num_ref_frames,
        separate_colour_plane,
        log2_max_frame_num,
        pic_order_cnt_type,
        log2_max_pic_order_cnt_lsb,
        delta_pic_order_always_zero,
        mb_adaptive_frame_field,
        pic_width_in_mbs: width_mbs,
        pic_height_in_map_units: height_map_units,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    struct BitWriter {
        bytes: Vec<u8>,
        bit_pos: usize,
    }

    impl BitWriter {
        fn new() -> Self {
            Self {
                bytes: Vec::new(),
                bit_pos: 0,
            }
        }

        fn bit(&mut self, value: bool) {
            if self.bit_pos == 0 {
                self.bytes.push(0);
            }

            if value {
                let last = self.bytes.len() - 1;
                self.bytes[last] |= 1 << (7 - self.bit_pos);
            }

            self.bit_pos = (self.bit_pos + 1) % 8;
        }

        fn ue(&mut self, value: u32) {
            let code = u64::from(value) + 1;
            let width = 64 - code.leading_zeros();

            for _ in 1..width {
                self.bit(false);
            }

            for shift in (0..width).rev() {
                self.bit(((code >> shift) & 1) != 0);
            }
        }

        fn finish(mut self) -> Vec<u8> {
            self.bit(true);

            while self.bit_pos != 0 {
                self.bit(false);
            }

            self.bytes
        }
    }

    fn make_baseline_sps(
        width_mbs_minus1: u32,
        height_map_units_minus1: u32,
        crop_right: u32,
        crop_bottom: u32,
    ) -> Vec<u8> {
        let mut writer = BitWriter::new();

        writer.ue(0);
        writer.ue(0);
        writer.ue(0);
        writer.ue(0);
        writer.ue(1);
        writer.bit(false);

        writer.ue(width_mbs_minus1);
        writer.ue(height_map_units_minus1);

        writer.bit(true);
        writer.bit(true);

        let cropping = crop_right != 0 || crop_bottom != 0;
        writer.bit(cropping);

        if cropping {
            writer.ue(0);
            writer.ue(crop_right);
            writer.ue(0);
            writer.ue(crop_bottom);
        }

        writer.bit(false);

        let mut nal = vec![0x67, 66, 0, 30];
        nal.extend(writer.finish());
        nal
    }

    #[test]
    fn parses_baseline_dimensions() {
        let nal = make_baseline_sps(39, 29, 0, 0);
        let sps = parse_sps(&nal).unwrap();

        assert_eq!(sps.profile_idc, 66);
        assert_eq!(sps.level_idc, 30);
        assert_eq!(sps.width, 640);
        assert_eq!(sps.height, 480);
        assert_eq!(sps.chroma_format_idc, 1);
        assert_eq!(sps.bit_depth_luma, 8);
        assert_eq!(sps.bit_depth_chroma, 8);
        assert!(sps.frame_mbs_only);
        assert_eq!(sps.log2_max_frame_num, 4);
        assert_eq!(sps.pic_order_cnt_type, 0);
        assert_eq!(sps.log2_max_pic_order_cnt_lsb, 4);
        assert_eq!(sps.pic_width_in_mbs, 40);
        assert_eq!(sps.pic_height_in_map_units, 30);
    }

    #[test]
    fn applies_420_frame_cropping() {
        let nal = make_baseline_sps(39, 29, 2, 2);
        let sps = parse_sps(&nal).unwrap();

        assert_eq!(sps.width, 636);
        assert_eq!(sps.height, 476);
    }

    #[test]
    fn rejects_non_sps_nal() {
        assert_eq!(parse_sps(&[0x68, 0, 0, 0]), Err(SpsError::InvalidNalType));
    }

    #[test]
    fn rejects_empty_nal() {
        assert_eq!(parse_sps(&[]), Err(SpsError::InvalidNalType));
    }
}

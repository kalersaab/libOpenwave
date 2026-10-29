use crate::video::bitstream::{BitReader, BitstreamError};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PpsError {
    InvalidNalType,
    InvalidParameter,
    UnsupportedFeature,
    Bitstream(BitstreamError),
}

impl std::fmt::Display for PpsError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "H.264 PPS error: {self:?}")
    }
}

impl std::error::Error for PpsError {}

impl From<BitstreamError> for PpsError {
    fn from(value: BitstreamError) -> Self {
        Self::Bitstream(value)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pps {
    pub id: u32,
    pub sps_id: u32,
    pub entropy_coding_mode: bool,
    pub bottom_field_pic_order_in_frame_present: bool,
    pub num_slice_groups: u32,
    pub num_ref_idx_l0_default_active: u32,
    pub num_ref_idx_l1_default_active: u32,
    pub weighted_pred: bool,
    pub weighted_bipred_idc: u8,
    pub pic_init_qp_minus26: i32,
    pub pic_init_qs_minus26: i32,
    pub chroma_qp_index_offset: i32,
    pub deblocking_filter_control_present: bool,
    pub constrained_intra_pred: bool,
    pub redundant_pic_cnt_present: bool,
}

fn flag(r: &mut BitReader<'_>) -> Result<bool, PpsError> {
    Ok(r.read_bit()?)
}

fn ue(r: &mut BitReader<'_>) -> Result<u32, PpsError> {
    Ok(r.read_ue()?)
}

fn se(r: &mut BitReader<'_>) -> Result<i32, PpsError> {
    Ok(r.read_se()?)
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

fn skip_slice_groups(r: &mut BitReader<'_>, groups_minus1: u32) -> Result<(), PpsError> {
    if groups_minus1 == 0 {
        return Ok(());
    }

    // Defensive bound for malformed streams.
    if groups_minus1 > 7 {
        return Err(PpsError::InvalidParameter);
    }

    match ue(r)? {
        0 => {
            // run_length_minus1[i]
            for _ in 0..=groups_minus1 {
                let _run_length_minus1 = ue(r)?;
            }
        }
        1 => {
            // Dispersed map: no additional syntax.
        }
        2 => {
            // top_left[i], bottom_right[i]
            for _ in 0..groups_minus1 {
                let _top_left = ue(r)?;
                let _bottom_right = ue(r)?;
            }
        }
        3..=5 => {
            let _direction = flag(r)?;
            let _rate = ue(r)?;
        }
        6 => {
            let units_minus1 = ue(r)?;

            if units_minus1 > 1_000_000 {
                return Err(PpsError::InvalidParameter);
            }

            let groups = groups_minus1 + 1;
            let bits = u32::BITS - (groups - 1).leading_zeros();

            for _ in 0..=units_minus1 {
                let id = r.read_bits(bits)?;

                if id >= groups {
                    return Err(PpsError::InvalidParameter);
                }
            }
        }
        _ => return Err(PpsError::InvalidParameter),
    }

    Ok(())
}

// `nal` includes the one-byte NAL header.
// This initial parser handles the base PPS syntax. Optional PPS extensions
// and scaling matrices are deliberately left for the next iteration.
pub fn parse_pps(nal: &[u8]) -> Result<Pps, PpsError> {
    let header = *nal.first().ok_or(PpsError::InvalidNalType)?;

    if header & 0x80 != 0 || header & 0x1f != 8 {
        return Err(PpsError::InvalidNalType);
    }

    let rbsp = ebsp_to_rbsp(&nal[1..]);
    let mut r = BitReader::new(&rbsp);

    let id = ue(&mut r)?;
    let sps_id = ue(&mut r)?;

    if id > 255 || sps_id > 31 {
        return Err(PpsError::InvalidParameter);
    }

    let entropy_coding_mode = flag(&mut r)?;
    let bottom_field_pic_order_in_frame_present = flag(&mut r)?;

    let groups_minus1 = ue(&mut r)?;
    if groups_minus1 > 7 {
        return Err(PpsError::InvalidParameter);
    }

    skip_slice_groups(&mut r, groups_minus1)?;

    let l0_minus1 = ue(&mut r)?;
    let l1_minus1 = ue(&mut r)?;

    if l0_minus1 > 31 || l1_minus1 > 31 {
        return Err(PpsError::InvalidParameter);
    }

    let weighted_pred = flag(&mut r)?;
    let weighted_bipred_idc = r.read_bits(2)? as u8;

    let pic_init_qp_minus26 = se(&mut r)?;
    let pic_init_qs_minus26 = se(&mut r)?;
    let chroma_qp_index_offset = se(&mut r)?;

    if !(-26..=25).contains(&pic_init_qp_minus26)
        || !(-26..=25).contains(&pic_init_qs_minus26)
        || !(-12..=12).contains(&chroma_qp_index_offset)
    {
        return Err(PpsError::InvalidParameter);
    }

    let deblocking_filter_control_present = flag(&mut r)?;
    let constrained_intra_pred = flag(&mut r)?;
    let redundant_pic_cnt_present = flag(&mut r)?;

    Ok(Pps {
        id,
        sps_id,
        entropy_coding_mode,
        bottom_field_pic_order_in_frame_present,
        num_slice_groups: groups_minus1 + 1,
        num_ref_idx_l0_default_active: l0_minus1 + 1,
        num_ref_idx_l1_default_active: l1_minus1 + 1,
        weighted_pred,
        weighted_bipred_idc,
        pic_init_qp_minus26,
        pic_init_qs_minus26,
        chroma_qp_index_offset,
        deblocking_filter_control_present,
        constrained_intra_pred,
        redundant_pic_cnt_present,
    })
}

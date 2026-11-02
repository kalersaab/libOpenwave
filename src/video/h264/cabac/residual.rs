use super::{context::CabacContexts, engine::CabacEngine};
use crate::video::h264::decoder::H264DecodeError;

const ZIGZAG_4X4: [usize; 16] = [0, 1, 4, 8, 5, 2, 3, 6, 9, 12, 13, 10, 7, 11, 14, 15];

const CBF_BASE_CTX: [usize; 5] = [85, 89, 93, 97, 101];

const SIGNIFICANT_COEFF_FLAG_OFFSET: [usize; 5] = [105, 120, 134, 149, 152];
const LAST_COEFF_FLAG_OFFSET: [usize; 5] = [166, 181, 195, 210, 213];
const COEFF_ABS_LEVEL_M1_OFFSET: [usize; 5] = [227, 237, 247, 257, 266];

const COEFF_ABS_LEVEL1_CTX: [usize; 8] = [1, 2, 3, 4, 0, 0, 0, 0];
const COEFF_ABS_LEVELGT1_CTX: [usize; 8] = [5, 5, 5, 5, 6, 7, 8, 9];
const COEFF_ABS_LEVEL_TRANS_LEVEL1: [usize; 8] = [1, 2, 3, 3, 4, 5, 6, 7];
const COEFF_ABS_LEVEL_TRANS_LEVELGT1: [usize; 8] = [4, 4, 4, 4, 5, 6, 7, 7];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResidualCategory {
    LumaDc = 0,
    LumaAc = 1,
    Luma4x4 = 2,
    ChromaDc = 3,
    ChromaAc = 4,
}

pub fn decode_cabac_residual_block(
    engine: &mut CabacEngine<'_>,
    contexts: &mut CabacContexts,
    category: ResidualCategory,
    cond_a_nz: bool,
    cond_b_nz: bool,
) -> Result<([i32; 16], usize), H264DecodeError> {
    let cat = category as usize;

    // 1. Decode Coded Block Flag (CBF)
    let cbf_ctx = CBF_BASE_CTX[cat] + (cond_a_nz as usize) + 2 * (cond_b_nz as usize);
    let cbf = engine.decode_decision(&mut contexts.states[cbf_ctx])?;
    if cbf == 0 {
        return Ok(([0i32; 16], 0));
    }

    let max_coeff = match category {
        ResidualCategory::LumaDc | ResidualCategory::Luma4x4 => 16,
        ResidualCategory::LumaAc | ResidualCategory::ChromaAc => 15,
        ResidualCategory::ChromaDc => 4,
    };

    let sig_base = SIGNIFICANT_COEFF_FLAG_OFFSET[cat];
    let last_base = LAST_COEFF_FLAG_OFFSET[cat];
    let abs_base = COEFF_ABS_LEVEL_M1_OFFSET[cat];

    // 2. Decode Significance Map
    let mut coeff_indices = Vec::with_capacity(max_coeff);
    let mut reached_end = true;

    for i in 0..(max_coeff - 1) {
        let sig = engine.decode_decision(&mut contexts.states[sig_base + i])?;
        if sig == 1 {
            coeff_indices.push(i);
            let last = engine.decode_decision(&mut contexts.states[last_base + i])?;
            if last == 1 {
                reached_end = false;
                break;
            }
        }
    }

    if reached_end {
        coeff_indices.push(max_coeff - 1);
    }

    let nnz = coeff_indices.len();
    if nnz == 0 {
        return Ok(([0i32; 16], 0));
    }

    // 3. Decode Absolute Levels and Signs (in reverse order)
    let mut block = [0i32; 16];
    let mut node_ctx = 0;

    for &scan_idx in coeff_indices.iter().rev() {
        let ctx1 = abs_base + COEFF_ABS_LEVEL1_CTX[node_ctx];
        let bin0 = engine.decode_decision(&mut contexts.states[ctx1])?;

        let abs_level = if bin0 == 0 {
            node_ctx = COEFF_ABS_LEVEL_TRANS_LEVEL1[node_ctx];
            1u32
        } else {
            let ctx_gt1 = abs_base + COEFF_ABS_LEVELGT1_CTX[node_ctx];
            node_ctx = COEFF_ABS_LEVEL_TRANS_LEVELGT1[node_ctx];
            let mut level = 2u32;
            while level < 15 && engine.decode_decision(&mut contexts.states[ctx_gt1])? == 1 {
                level += 1;
            }
            if level >= 15 {
                let mut j = 0;
                while engine.decode_bypass()? == 1 && j < 23 {
                    j += 1;
                }
                let mut suffix = 1u32;
                for _ in 0..j {
                    suffix = (suffix << 1) | (engine.decode_bypass()? as u32);
                }
                level = suffix + 14;
            }
            level
        };

        let sign = engine.decode_bypass()?;
        let value = if sign == 1 {
            -(abs_level as i32)
        } else {
            abs_level as i32
        };

        // Map scan_idx to output block position
        let target_idx = match category {
            ResidualCategory::Luma4x4 => ZIGZAG_4X4[scan_idx],
            ResidualCategory::LumaAc | ResidualCategory::ChromaAc => ZIGZAG_4X4[scan_idx + 1],
            ResidualCategory::LumaDc | ResidualCategory::ChromaDc => scan_idx,
        };
        block[target_idx] = value;
    }

    Ok((block, nnz))
}

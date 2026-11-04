use super::{context::CabacContexts, engine::CabacEngine};
use crate::video::h264::{
    decoder::H264DecodeError,
    macroblock::types::{Intra16x16Mode, MacroblockType},
    slice::SliceType,
};

pub fn decode_mb_skip_flag(
    engine: &mut CabacEngine<'_>,
    contexts: &mut CabacContexts,
    slice_type: SliceType,
    left_available: bool,
    left_skip: bool,
    top_available: bool,
    top_skip: bool,
) -> Result<bool, H264DecodeError> {
    let base = if slice_type == SliceType::B { 24 } else { 11 };
    let mut ctx = 0;
    if left_available && !left_skip {
        ctx += 1;
    }
    if top_available && !top_skip {
        ctx += 1;
    }
    let flag = engine.decode_decision(&mut contexts.states[base + ctx])?;
    Ok(flag == 1)
}

pub fn decode_i_slice_mb_type(
    engine: &mut CabacEngine<'_>,
    contexts: &mut CabacContexts,
    left_is_intra16: bool,
    top_is_intra16: bool,
) -> Result<MacroblockType, H264DecodeError> {
    let mut ctx = 0;
    if left_is_intra16 {
        ctx += 1;
    }
    if top_is_intra16 {
        ctx += 1;
    }

    if engine.decode_decision(&mut contexts.states[3 + ctx])? == 0 {
        return Ok(MacroblockType::Intra4x4);
    }

    if engine.decode_terminate()? == 1 {
        return Ok(MacroblockType::Ipcm);
    }

    let cbp_luma_nz = engine.decode_decision(&mut contexts.states[6])?;
    let mut cbp_chroma = 0;
    if engine.decode_decision(&mut contexts.states[7])? == 1 {
        cbp_chroma = 1 + engine.decode_decision(&mut contexts.states[8])?;
    }
    let pred_mode_bit1 = engine.decode_decision(&mut contexts.states[9])?;
    let pred_mode_bit0 = engine.decode_decision(&mut contexts.states[10])?;
    let mode_val = (pred_mode_bit1 << 1) | pred_mode_bit0;
    eprintln!(
        "[decode_i_slice_mb_type] cbp_luma_nz={} cbp_chroma={} bit1={} bit0={} mode_val={}",
        cbp_luma_nz, cbp_chroma, pred_mode_bit1, pred_mode_bit0, mode_val
    );
    let prediction_mode =
        Intra16x16Mode::try_from(mode_val).map_err(|_| H264DecodeError::InvalidStream)?;
    let coded_luma_pattern = if cbp_luma_nz == 1 { 15 } else { 0 };

    Ok(MacroblockType::Intra16x16 {
        prediction_mode,
        coded_chroma_pattern: cbp_chroma,
        coded_luma_pattern,
    })
}

pub fn decode_p_slice_mb_type(
    engine: &mut CabacEngine<'_>,
    contexts: &mut CabacContexts,
    _left_is_intra16: bool,
    _top_is_intra16: bool,
) -> Result<Option<MacroblockType>, H264DecodeError> {
    let b0 = engine.decode_decision(&mut contexts.states[14])?;
    if b0 == 0 {
        let b1 = engine.decode_decision(&mut contexts.states[15])?;
        if b1 == 0 {
            let b2 = engine.decode_decision(&mut contexts.states[16])?;
            if b2 == 0 {
                Ok(None)
            } else {
                eprintln!("[decode_p_slice_mb_type] P_8x8 (b0=0, b1=0, b2=1)");
                Ok(None)
            }
        } else {
            let b2 = engine.decode_decision(&mut contexts.states[17])?;
            if b2 == 0 {
                eprintln!("[decode_p_slice_mb_type] P_16x8 (b0=0, b1=1, b2=0)");
                Ok(None)
            } else {
                eprintln!("[decode_p_slice_mb_type] P_8x16 (b0=0, b1=1, b2=1)");
                Ok(None)
            }
        }
    } else {
        eprintln!("[decode_p_slice_mb_type] Intra in P-slice (b0=1)");
        if engine.decode_decision(&mut contexts.states[17])? == 0 {
            return Ok(Some(MacroblockType::Intra4x4));
        }
        if engine.decode_terminate()? == 1 {
            return Ok(Some(MacroblockType::Ipcm));
        }
        let cbp_luma_nz = engine.decode_decision(&mut contexts.states[18])?;
        let mut cbp_chroma = 0;
        if engine.decode_decision(&mut contexts.states[19])? == 1 {
            cbp_chroma = 1 + engine.decode_decision(&mut contexts.states[19])?;
        }
        let pred_mode_bit1 = engine.decode_decision(&mut contexts.states[20])?;
        let pred_mode_bit0 = engine.decode_decision(&mut contexts.states[20])?;
        let mode_val = (pred_mode_bit1 << 1) | pred_mode_bit0;
        let prediction_mode =
            Intra16x16Mode::try_from(mode_val).map_err(|_| H264DecodeError::InvalidStream)?;
        let coded_luma_pattern = if cbp_luma_nz == 1 { 15 } else { 0 };
        Ok(Some(MacroblockType::Intra16x16 {
            prediction_mode,
            coded_chroma_pattern: cbp_chroma,
            coded_luma_pattern,
        }))
    }
}

pub fn decode_cabac_ref_idx(
    engine: &mut CabacEngine<'_>,
    contexts: &mut CabacContexts,
    ref_a: Option<u32>,
    ref_b: Option<u32>,
    num_ref_idx_active: u32,
) -> Result<u32, H264DecodeError> {
    if num_ref_idx_active <= 1 {
        return Ok(0);
    }

    let mut ctx = 0;
    if ref_a.unwrap_or(0) > 0 {
        ctx += 1;
    }
    if ref_b.unwrap_or(0) > 0 {
        ctx += 2;
    }

    let mut ref_idx = 0u32;
    while engine.decode_decision(&mut contexts.states[54 + ctx])? == 1 {
        ref_idx += 1;
        ctx = 4;
        if ref_idx >= num_ref_idx_active {
            return Err(H264DecodeError::InvalidStream);
        }
    }
    Ok(ref_idx)
}

pub fn decode_cabac_mvd(
    engine: &mut CabacEngine<'_>,
    contexts: &mut CabacContexts,
    amvd: i32,
    ctx_base: usize,
) -> Result<i32, H264DecodeError> {
    let ctx_inc = if amvd < 3 {
        0
    } else if amvd <= 32 {
        1
    } else {
        2
    };

    if engine.decode_decision(&mut contexts.states[ctx_base + ctx_inc])? == 0 {
        return Ok(0);
    }

    let mut mvd = 1i32;
    let mut ctx = ctx_base + 3;
    while mvd < 9 && engine.decode_decision(&mut contexts.states[ctx])? == 1 {
        if mvd < 4 {
            ctx += 1;
        }
        mvd += 1;
    }

    if mvd >= 9 {
        let mut k = 3;
        while engine.decode_bypass()? == 1 {
            mvd += 1 << k;
            k += 1;
            if k > 24 {
                return Err(H264DecodeError::InvalidStream);
            }
        }
        while k > 0 {
            k -= 1;
            mvd += (engine.decode_bypass()? as i32) << k;
        }
    }

    let sign = engine.decode_bypass()?;
    Ok(if sign == 1 { -mvd } else { mvd })
}

pub fn decode_cabac_cbp(
    engine: &mut CabacEngine<'_>,
    contexts: &mut CabacContexts,
    left_cbp: u32,
    top_cbp: u32,
) -> Result<u32, H264DecodeError> {
    let mut cbp = 0u32;

    let ctx0 = usize::from((left_cbp & 0x02) == 0) + 2 * usize::from((top_cbp & 0x04) == 0);
    cbp |= engine.decode_decision(&mut contexts.states[73 + ctx0])? as u32;

    let ctx1 = usize::from((cbp & 0x01) == 0) + 2 * usize::from((top_cbp & 0x08) == 0);
    cbp |= (engine.decode_decision(&mut contexts.states[73 + ctx1])? as u32) << 1;

    let ctx2 = usize::from((left_cbp & 0x08) == 0) + 2 * usize::from((cbp & 0x01) == 0);
    cbp |= (engine.decode_decision(&mut contexts.states[73 + ctx2])? as u32) << 2;

    let ctx3 = usize::from((cbp & 0x04) == 0) + 2 * usize::from((cbp & 0x02) == 0);
    cbp |= (engine.decode_decision(&mut contexts.states[73 + ctx3])? as u32) << 3;

    let cbp_a = (left_cbp >> 4) & 0x03;
    let cbp_b = (top_cbp >> 4) & 0x03;

    let mut ctx_c = 0;
    if cbp_a > 0 {
        ctx_c += 1;
    }
    if cbp_b > 0 {
        ctx_c += 2;
    }

    if engine.decode_decision(&mut contexts.states[77 + ctx_c])? == 1 {
        let mut ctx_c2 = 4;
        if cbp_a == 2 {
            ctx_c2 += 1;
        }
        if cbp_b == 2 {
            ctx_c2 += 2;
        }
        let bit = engine.decode_decision(&mut contexts.states[77 + ctx_c2])? as u32;
        let chroma_cbp = 1 + bit;
        cbp |= chroma_cbp << 4;
    }

    Ok(cbp)
}

pub fn decode_cabac_mb_qp_delta(
    engine: &mut CabacEngine<'_>,
    contexts: &mut CabacContexts,
    prev_mb_qp_delta: i32,
) -> Result<i32, H264DecodeError> {
    let ctx = if prev_mb_qp_delta != 0 { 1 } else { 0 };
    if engine.decode_decision(&mut contexts.states[60 + ctx])? == 0 {
        return Ok(0);
    }

    let mut val = 1i32;
    let mut c = 2;
    while engine.decode_decision(&mut contexts.states[60 + c])? == 1 {
        c = 3;
        val += 1;
        if val > 104 {
            return Err(H264DecodeError::InvalidStream);
        }
    }

    let delta = if val % 2 == 1 {
        (val + 1) / 2
    } else {
        -((val + 1) / 2)
    };
    Ok(delta)
}

pub fn decode_cabac_chroma_pred_mode(
    engine: &mut CabacEngine<'_>,
    contexts: &mut CabacContexts,
    left_chroma_mode: u8,
    top_chroma_mode: u8,
) -> Result<u8, H264DecodeError> {
    let mut ctx = 0;
    if left_chroma_mode > 0 {
        ctx += 1;
    }
    if top_chroma_mode > 0 {
        ctx += 1;
    }

    if engine.decode_decision(&mut contexts.states[64 + ctx])? == 0 {
        return Ok(0);
    }
    if engine.decode_decision(&mut contexts.states[67])? == 0 {
        return Ok(1);
    }
    if engine.decode_decision(&mut contexts.states[67])? == 0 {
        return Ok(2);
    }
    Ok(3)
}

pub fn decode_intra4x4_pred_mode(
    engine: &mut CabacEngine<'_>,
    contexts: &mut CabacContexts,
    predicted_mode: u8,
) -> Result<u8, H264DecodeError> {
    let flag = engine.decode_decision(&mut contexts.states[68])?;
    if flag == 1 {
        eprintln!(
            "[intra4x4] predicted={}, flag=1 -> mode={}",
            predicted_mode, predicted_mode
        );
        return Ok(predicted_mode);
    }
    let b0 = engine.decode_decision(&mut contexts.states[69])?;
    let b1 = engine.decode_decision(&mut contexts.states[69])?;
    let b2 = engine.decode_decision(&mut contexts.states[69])?;
    let rem_mode = b0 | (b1 << 1) | (b2 << 2);
    let res = if rem_mode < predicted_mode {
        rem_mode
    } else {
        rem_mode + 1
    };
    eprintln!(
        "[intra4x4] predicted={}, flag=0, b=[{},{},{}], rem={}, res={}",
        predicted_mode, b0, b1, b2, rem_mode, res
    );
    Ok(res)
}

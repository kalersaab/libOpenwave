use super::tables::{CABAC_CONTEXT_INIT_I, CABAC_CONTEXT_INIT_PB};
use crate::video::h264::slice::SliceType;


#[derive(Debug, Clone)]
pub struct CabacContexts {
    pub states: [(u8, u8); 1024],
}

impl CabacContexts {
    
    pub fn new(slice_type: SliceType, cabac_init_idc: u32, slice_qp_y: i32) -> Self {
        let qp = slice_qp_y.clamp(0, 51);
        let mut states = [(0u8, 0u8); 1024];

        let is_i = matches!(slice_type, SliceType::I | SliceType::Si);
        let idc = (cabac_init_idc as usize).min(2);

        for i in 0..1024 {
            let [m, n] = if is_i {
                CABAC_CONTEXT_INIT_I[i]
            } else {
                CABAC_CONTEXT_INIT_PB[idc][i]
            };

            let pre_ctx_state = (((m as i32 * qp) >> 4) + n as i32).clamp(1, 126);
            if pre_ctx_state <= 63 {
                states[i] = ((63 - pre_ctx_state) as u8, 0);
            } else {
                states[i] = ((pre_ctx_state - 64) as u8, 1);
            }
        }

        Self { states }
    }
}

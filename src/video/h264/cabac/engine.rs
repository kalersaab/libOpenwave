use super::tables::{RANGE_TAB_LPS, TRANS_IDX_LPS, TRANS_IDX_MPS};
use crate::video::h264::decoder::H264DecodeError;

#[derive(Debug)]
pub struct CabacEngine<'a> {
    data: &'a [u8],
    byte_pos: usize,
    bit_pos: u8,
    pub range: u32,
    pub offset: u32,
    pub bits_consumed: u64,
}

impl<'a> CabacEngine<'a> {
    pub fn new(data: &'a [u8]) -> Result<Self, H264DecodeError> {
        let mut engine = Self {
            data,
            byte_pos: 0,
            bit_pos: 0,
            range: 510,
            offset: 0,
            bits_consumed: 0,
        };
        // Read initial 9 bits (Clause 9.3.1.2)
        for _ in 0..9 {
            let bit = engine.read_bit()?;
            engine.offset = (engine.offset << 1) | bit;
        }
        if engine.offset == 510 || engine.offset == 511 {
            eprintln!(
                "[CabacEngine::new] Invalid initial offset: {}",
                engine.offset
            );
            return Err(H264DecodeError::InvalidStream);
        }
        Ok(engine)
    }

    #[inline]
    fn read_bit(&mut self) -> Result<u32, H264DecodeError> {
        self.bits_consumed += 1;
        if self.byte_pos >= self.data.len() {
            // End of slice data: return 0 (trailing bits / padding)
            return Ok(0);
        }
        let byte = self.data[self.byte_pos];
        let bit = ((byte >> (7 - self.bit_pos)) & 1) as u32;
        self.bit_pos += 1;
        if self.bit_pos == 8 {
            self.bit_pos = 0;
            self.byte_pos += 1;
        }
        Ok(bit)
    }

    /// Decode a binary decision using context model state (Clause 9.3.3.2.1).
    /// `state` contains `(p_state_idx: 0..63, val_mps: 0..1)`.
    pub fn decode_decision(&mut self, state: &mut (u8, u8)) -> Result<u8, H264DecodeError> {
        let (p_state_idx, val_mps) = *state;
        let q_cod_i_range_idx = ((self.range >> 6) & 3) as usize;
        let cod_i_range_lps = RANGE_TAB_LPS[p_state_idx as usize][q_cod_i_range_idx] as u32;
        self.range -= cod_i_range_lps;

        let bin_val = if self.offset >= self.range {
            let bin = 1 - val_mps;
            self.offset -= self.range;
            self.range = cod_i_range_lps;
            if p_state_idx == 0 {
                state.1 = 1 - val_mps;
            }
            state.0 = TRANS_IDX_LPS[p_state_idx as usize];
            bin
        } else {
            let bin = val_mps;
            state.0 = TRANS_IDX_MPS[p_state_idx as usize];
            bin
        };

        // Renormalization
        while self.range < 256 {
            self.range <<= 1;
            self.offset = (self.offset << 1) | self.read_bit()?;
        }

        Ok(bin_val)
    }

    /// Decode in bypass mode (Clause 9.3.3.2.3).
    pub fn decode_bypass(&mut self) -> Result<u8, H264DecodeError> {
        self.offset = (self.offset << 1) | self.read_bit()?;
        if self.offset >= self.range {
            self.offset -= self.range;
            Ok(1)
        } else {
            Ok(0)
        }
    }

    /// Decode signed value in bypass mode (`val * sign`).
    pub fn decode_bypass_sign(&mut self, val: i32) -> Result<i32, H264DecodeError> {
        let sign = self.decode_bypass()?;
        if sign == 1 { Ok(-val) } else { Ok(val) }
    }

    /// Decode termination flag (Clause 9.3.3.2.4).
    pub fn decode_terminate(&mut self) -> Result<u8, H264DecodeError> {
        self.range -= 2;
        if self.offset >= self.range {
            Ok(1)
        } else {
            while self.range < 256 {
                self.range <<= 1;
                self.offset = (self.offset << 1) | self.read_bit()?;
            }
            Ok(0)
        }
    }
}

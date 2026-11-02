use crate::audio::aac::bitstream::BitReader;
use std::io;

#[derive(Debug, Clone)]
pub struct TnsData {
    pub n_filt: [u8; 8],
    pub coef_res: [bool; 8],
    pub length: [[u8; 4]; 8],
    pub order: [[u8; 4]; 8],
    pub direction: [[bool; 4]; 8],
    pub coef_compress: [[bool; 4]; 8],
    pub coef: [[[f32; 20]; 4]; 8],
}

impl TnsData {
    pub fn decode(br: &mut BitReader, is_eight_short: bool) -> io::Result<Self> {
        let num_windows = if is_eight_short { 8 } else { 1 };

        let mut tns_data = TnsData {
            n_filt: [0; 8],
            coef_res: [false; 8],
            length: [[0; 4]; 8],
            order: [[0; 4]; 8],
            direction: [[false; 4]; 8],
            coef_compress: [[false; 4]; 8],
            coef: [[[0.0; 20]; 4]; 8],
        };

        for w in 0..num_windows {
            let n_filt = br.read_bits(if is_eight_short { 1 } else { 2 })? as u8;
            tns_data.n_filt[w] = n_filt;

            if n_filt > 0 {
                tns_data.coef_res[w] = br.read_bit()?;
            }

            for f in 0..n_filt as usize {
                let length = br.read_bits(if is_eight_short { 4 } else { 6 })? as u8;
                tns_data.length[w][f] = length;

                let order = br.read_bits(if is_eight_short { 3 } else { 5 })? as u8;
                tns_data.order[w][f] = order;

                if order > 0 {
                    tns_data.direction[w][f] = br.read_bit()?;
                    tns_data.coef_compress[w][f] = br.read_bit()?;

                    let coef_bits = if tns_data.coef_res[w] { 4 } else { 3 };
                    let coef_bits = if tns_data.coef_compress[w][f] {
                        coef_bits - 1
                    } else {
                        coef_bits
                    };

                    for i in 0..order as usize {
                        let coef_raw = br.read_bits(coef_bits)? as i8;
                        tns_data.coef[w][f][i] =
                            Self::dequantize_coef(coef_raw, coef_bits, tns_data.coef_res[w]);
                    }
                }
            }
        }

        Ok(tns_data)
    }

    fn dequantize_coef(raw: i8, bits: u8, coef_res: bool) -> f32 {
        let max_val = (1 << bits) - 1;
        let signed = if raw > (max_val / 2) {
            raw - max_val - 1
        } else {
            raw
        };

        if coef_res {
            signed as f32 / 8.0
        } else {
            signed as f32 / 4.0
        }
    }

    pub fn apply(&self, spectral: &mut [f32], window: usize, max_sfb: usize) {
        if window >= 8 {
            return;
        }

        for f in 0..self.n_filt[window] as usize {
            let order = self.order[window][f] as usize;
            if order == 0 {
                continue;
            }

            let start = max_sfb.saturating_sub(self.length[window][f] as usize);
            let end = max_sfb;

            if self.direction[window][f] {
                for i in (start..end).rev() {
                    let mut sum = spectral[i];
                    for k in 0..order.min(i - start) {
                        sum -= self.coef[window][f][k] * spectral[i - k - 1];
                    }
                    spectral[i] = sum;
                }
            } else {
                for i in start..end {
                    let mut sum = spectral[i];
                    for k in 0..order.min(i - start) {
                        sum -= self.coef[window][f][k] * spectral[i - k - 1];
                    }
                    spectral[i] = sum;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dequantizes_4bit_coef() {
        let coef = TnsData::dequantize_coef(7, 4, true);
        assert!((coef - 0.875).abs() < 0.001);
    }

    #[test]
    fn applies_tns_identity() {
        let tns = TnsData {
            n_filt: [0; 8],
            coef_res: [false; 8],
            length: [[0; 4]; 8],
            order: [[0; 4]; 8],
            direction: [[false; 4]; 8],
            coef_compress: [[false; 4]; 8],
            coef: [[[0.0; 20]; 4]; 8],
        };

        let mut spectral = vec![1.0, 2.0, 3.0, 4.0, 5.0];
        let original = spectral.clone();

        tns.apply(&mut spectral, 0, 5);

        assert_eq!(spectral, original);
    }
}

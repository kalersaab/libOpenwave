// Symphonia AAC-LC decoder port.
// Copyright (c) 2019-2026 The Project Symphonia Developers.
// This file is licensed under the Mozilla Public License, v. 2.0.

use crate::audio::aac::bitstream::BitReader;
use crate::audio::aac::common::*;
use crate::audio::aac::ics::IcsInfo;
use std::f32::consts;
use std::io;

const TNS_MAX_ORDER: usize = 20;
const TNS_MAX_LONG_BANDS: [usize; 12] = [31, 31, 34, 40, 42, 51, 46, 46, 42, 42, 42, 39];
const TNS_MAX_SHORT_BANDS: [usize; 12] = [9, 9, 10, 14, 14, 14, 14, 14, 14, 14, 14, 14];

#[derive(Copy, Clone)]
struct TnsCoeffs {
    length: usize,
    order: usize,
    direction: bool,
    coef: [f32; TNS_MAX_ORDER + 1],
}

impl TnsCoeffs {
    fn new() -> Self {
        Self {
            length: 0,
            order: 0,
            direction: false,
            coef: [0.0; TNS_MAX_ORDER + 1],
        }
    }

    fn read(
        &mut self,
        br: &mut BitReader,
        long_win: bool,
        coef_res: bool,
        max_order: usize,
    ) -> io::Result<()> {
        self.length = br.read_bits(if long_win { 6 } else { 4 })? as usize;
        self.order = br.read_bits(if long_win { 5 } else { 3 })? as usize;

        if self.order > max_order {
            return Err(decode_error("aac: invalid tns order"));
        }

        if self.order > 0 {
            self.direction = br.read_bit()?;
            let coef_compress = br.read_bit()?;

            let mut coef_res_bits = if coef_res { 4 } else { 3 };
            if coef_compress {
                coef_res_bits -= 1;
            }

            let sign_mask = 1 << (coef_res_bits - 1);
            let neg_mask = !((1 << coef_res_bits) - 1);
            let fac_base = if coef_res { 8.0 } else { 4.0 };
            let iqfac = (fac_base - 0.5) / consts::FRAC_PI_2;
            let iqfac_m = (fac_base + 0.5) / consts::FRAC_PI_2;

            let mut tmp: [f32; TNS_MAX_ORDER] = [0.0; TNS_MAX_ORDER];

            for el in tmp[..self.order].iter_mut() {
                let val = br.read_bits(coef_res_bits)? as u8;
                let c = f32::from(if (val & sign_mask) != 0 {
                    (val | neg_mask) as i8
                } else {
                    val as i8
                });
                *el = (if c >= 0.0 { c / iqfac } else { c / iqfac_m }).sin();
            }

            let mut b: [f32; TNS_MAX_ORDER + 1] = [0.0; TNS_MAX_ORDER + 1];

            for m in 1..=self.order {
                for i in 1..m {
                    b[i] = self.coef[i - 1] + tmp[m - 1] * self.coef[m - i - 1];
                }
                self.coef[..(m - 1)].copy_from_slice(&b[1..m]);
                self.coef[m - 1] = tmp[m - 1];
            }
        }

        Ok(())
    }
}

#[derive(Copy, Clone)]
pub struct Tns {
    n_filt: [usize; MAX_WINDOWS],
    coeffs: [[TnsCoeffs; 4]; MAX_WINDOWS],
}

impl Tns {
    pub fn read(br: &mut BitReader, info: &IcsInfo) -> io::Result<Option<Self>> {
        let tns_data_present = br.read_bit()?;
        if !tns_data_present {
            return Ok(None);
        }

        let max_order = if !info.long_win { 7 } else { 12 };

        let mut n_filt: [usize; MAX_WINDOWS] = [0; MAX_WINDOWS];
        let mut coeffs: [[TnsCoeffs; 4]; MAX_WINDOWS] = [[TnsCoeffs::new(); 4]; MAX_WINDOWS];

        for w in 0..info.num_windows {
            n_filt[w] = br.read_bits(if info.long_win { 2 } else { 1 })? as usize;
            let coef_res = if n_filt[w] != 0 {
                br.read_bit()?
            } else {
                false
            };

            for filt in 0..n_filt[w] {
                coeffs[w][filt].read(br, info.long_win, coef_res, max_order)?;
            }
        }

        Ok(Some(Self { n_filt, coeffs }))
    }

    pub fn synth(
        &self,
        info: &IcsInfo,
        bands: &[usize],
        rate_idx: usize,
        coeffs: &mut [f32; 1024],
    ) {
        let tns_max_bands = (if info.long_win {
            TNS_MAX_LONG_BANDS[rate_idx]
        } else {
            TNS_MAX_SHORT_BANDS[rate_idx]
        })
        .min(info.max_sfb);

        for w in 0..info.num_windows {
            let mut bottom = bands.len() - 1;

            for f in 0..self.n_filt[w] {
                let top = bottom;
                bottom = top.saturating_sub(self.coeffs[w][f].length);
                let order = self.coeffs[w][f].order;

                if order == 0 {
                    continue;
                }

                let start = w * 128 + bands[bottom.min(tns_max_bands)];
                let end = w * 128 + bands[top.min(tns_max_bands)];
                let lpc = &self.coeffs[w][f].coef;

                if !self.coeffs[w][f].direction {
                    for (m, i) in (start..end).enumerate() {
                        for j in 0..order.min(m) {
                            coeffs[i] -= coeffs[i - j - 1] * lpc[j];
                        }
                    }
                } else {
                    for (m, i) in (start..end).rev().enumerate() {
                        for j in 0..order.min(m) {
                            coeffs[i] -= coeffs[i + j + 1] * lpc[j];
                        }
                    }
                }
            }
        }
    }
}

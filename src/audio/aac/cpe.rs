// Symphonia AAC-LC decoder port.
// Copyright (c) 2019-2026 The Project Symphonia Developers.
// This file is licensed under the Mozilla Public License, v. 2.0.

use crate::audio::aac::bitstream::BitReader;
use crate::audio::aac::common::*;
use crate::audio::aac::dsp;
use crate::audio::aac::ics;
use std::io;

#[derive(Clone)]
pub struct ChannelPair {
    pub is_pair: bool,
    pub channel: usize,
    ms_mask_present: u8,
    ms_used: [[bool; MAX_SFBS]; MAX_WINDOWS],
    ics0: ics::Ics,
    ics1: ics::Ics,
    lcg: Lcg,
}

impl ChannelPair {
    pub fn new(is_pair: bool, channel: usize, sbinfo: GASubbandInfo) -> Self {
        Self {
            is_pair,
            channel,
            ms_mask_present: 0,
            ms_used: [[false; MAX_SFBS]; MAX_WINDOWS],
            ics0: ics::Ics::new(sbinfo),
            ics1: ics::Ics::new(sbinfo),
            lcg: Lcg::new(0x1f2e3d4c),
        }
    }

    pub fn reset(&mut self) {
        self.ics0.reset();
        self.ics1.reset();
    }

    pub fn decode_ga_sce(&mut self, br: &mut BitReader) -> io::Result<()> {
        self.ics0.decode(br, &mut self.lcg, false)
    }

    pub fn decode_ga_cpe(&mut self, br: &mut BitReader) -> io::Result<()> {
        let common_window = br.read_bit()?;

        if common_window {
            self.ics0.decode_info(br)?;

            self.ms_mask_present = br.read_bits(2)? as u8;

            match self.ms_mask_present {
                0 | 2 => {
                    let is_used = self.ms_mask_present == 2;
                    for g in 0..self.ics0.info.window_groups {
                        for sfb in 0..self.ics0.info.max_sfb {
                            self.ms_used[g][sfb] = is_used;
                        }
                    }
                }
                1 => {
                    for g in 0..self.ics0.info.window_groups {
                        for sfb in 0..self.ics0.info.max_sfb {
                            self.ms_used[g][sfb] = br.read_bit()?;
                        }
                    }
                }
                3 => return Err(decode_error("aac: invalid mid-side mask")),
                _ => unreachable!(),
            }

            self.ics1.info.copy_from_common(&self.ics0.info);
        }

        self.ics0.decode(br, &mut self.lcg, common_window)?;
        self.ics1.decode(br, &mut self.lcg, common_window)?;

        if common_window {
            let bands = self.ics0.get_bands();
            let mut g = 0;

            for w in 0..self.ics0.info.num_windows {
                if w > 0 && !self.ics0.info.scale_factor_grouping[w - 1] {
                    g += 1;
                }

                for sfb in 0..self.ics0.info.max_sfb {
                    let start = w * 128 + bands[sfb];
                    let end = w * 128 + bands[sfb + 1];

                    if self.ics1.is_intensity(g, sfb) {
                        let invert = self.ms_mask_present == 1 && self.ms_used[g][sfb];
                        let dir = if self.ics1.get_intensity_dir(g, sfb) {
                            1.0
                        } else {
                            -1.0
                        };
                        let factor = if invert { -1.0 } else { 1.0 };
                        let scale = dir * factor * self.ics1.scales[g][sfb];

                        let left = &self.ics0.coeffs[start..end];
                        let right = &mut self.ics1.coeffs[start..end];

                        for (l, r) in left.iter().zip(right) {
                            *r = scale * l;
                        }
                    } else if self.ics0.is_noise(g, sfb) || self.ics1.is_noise(g, sfb) {
                    } else if self.ms_used[g][sfb] {
                        let mid = &mut self.ics0.coeffs[start..end];
                        let side = &mut self.ics1.coeffs[start..end];

                        for (m, s) in mid.iter_mut().zip(side) {
                            let tmp = *m - *s;
                            *m += *s;
                            *s = tmp;
                        }
                    }
                }
            }
        }

        Ok(())
    }

    pub fn synth_audio(
        &mut self,
        dsp: &mut dsp::Dsp,
        rate_idx: usize,
        left: &mut [f32],
        right: &mut [f32],
    ) {
        self.ics0.synth_channel(dsp, rate_idx, left);
        if self.is_pair {
            self.ics1.synth_channel(dsp, rate_idx, right);
        }
    }
}

// Symphonia AAC-LC decoder port.
// Copyright (c) 2019-2026 The Project Symphonia Developers.
// This file is licensed under the Mozilla Public License, v. 2.0.

use crate::audio::aac::bitstream::BitReader;
use crate::audio::aac::codebooks;
use crate::audio::aac::common::*;
use crate::audio::aac::dsp;
use crate::audio::aac::pulse::Pulse;
use crate::audio::aac::tns::Tns;
use std::io;
use std::sync::LazyLock;

const ZERO_HCB: u8 = 0;
const RESERVED_HCB: u8 = 12;
const NOISE_HCB: u8 = 13;
const INTENSITY_HCB2: u8 = 14;
const INTENSITY_HCB: u8 = 15;

const INTENSITY_SCALE_MIN: i16 = -155;
const NORMAL_SCALE_MIN: i16 = -100;

static NORMAL_SCF_TABLE: LazyLock<[f32; 256]> = LazyLock::new(|| {
    let mut table = [0.0f32; 256];
    for (i, v) in table.iter_mut().enumerate() {
        *v = 2.0f32.powf(0.25 * f32::from(i as i16 - 56 + NORMAL_SCALE_MIN));
    }
    table
});

static INTENSITY_SCF_TABLE: LazyLock<[f32; 256]> = LazyLock::new(|| {
    let mut table = [0.0f32; 256];
    for (i, v) in table.iter_mut().enumerate() {
        *v = 0.5f32.powf(0.25 * f32::from(i as i16 + INTENSITY_SCALE_MIN));
    }
    table
});

#[derive(Clone)]
pub struct IcsInfo {
    pub window_sequence: u8,
    pub prev_window_sequence: u8,
    pub window_shape: bool,
    pub prev_window_shape: bool,
    pub scale_factor_grouping: [bool; MAX_WINDOWS],
    pub group_start: [usize; MAX_WINDOWS],
    pub window_groups: usize,
    pub num_windows: usize,
    pub max_sfb: usize,
    pub long_win: bool,
}

impl IcsInfo {
    fn new() -> Self {
        Self {
            window_sequence: 0,
            prev_window_sequence: 0,
            window_shape: false,
            prev_window_shape: false,
            scale_factor_grouping: [false; MAX_WINDOWS],
            group_start: [0; MAX_WINDOWS],
            num_windows: 0,
            window_groups: 0,
            max_sfb: 0,
            long_win: true,
        }
    }

    pub fn decode(&mut self, br: &mut BitReader) -> io::Result<()> {
        self.prev_window_sequence = self.window_sequence;
        self.prev_window_shape = self.window_shape;

        if br.read_bit()? {
            return Err(decode_error("aac: ics reserved bit set"));
        }

        self.window_sequence = br.read_bits(2)? as u8;
        self.window_shape = br.read_bit()?;
        self.window_groups = 1;

        if self.window_sequence == EIGHT_SHORT_SEQUENCE {
            self.long_win = false;
            self.num_windows = 8;
            self.max_sfb = br.read_bits(4)? as usize;

            for i in 0..MAX_WINDOWS - 1 {
                self.scale_factor_grouping[i] = br.read_bit()?;
                if !self.scale_factor_grouping[i] {
                    self.group_start[self.window_groups] = i + 1;
                    self.window_groups += 1;
                }
            }
        } else {
            self.long_win = true;
            self.num_windows = 1;
            self.max_sfb = br.read_bits(6)? as usize;

            if br.read_bit()? {
                return Err(unsupported_error("aac: predictor data"));
            }
        }
        Ok(())
    }

    pub fn copy_from_common(&mut self, other: &IcsInfo) {
        let prev_window_sequence = self.window_sequence;
        let prev_window_shape = self.window_shape;
        *self = other.clone();
        self.prev_window_sequence = prev_window_sequence;
        self.prev_window_shape = prev_window_shape;
    }

    fn get_group_start(&self, g: usize) -> usize {
        if g == 0 {
            0
        } else if g >= self.window_groups {
            if self.long_win { 1 } else { 8 }
        } else {
            self.group_start[g]
        }
    }
}

#[derive(Clone)]
pub struct Ics {
    global_gain: u8,
    pub info: IcsInfo,
    pulse: Option<Pulse>,
    tns: Option<Tns>,
    sect_cb: [[u8; MAX_SFBS]; MAX_WINDOWS],
    sect_len: [[usize; MAX_SFBS]; MAX_WINDOWS],
    sfb_cb: [[u8; MAX_SFBS]; MAX_WINDOWS],
    num_sec: [usize; MAX_WINDOWS],
    pub scales: [[f32; MAX_SFBS]; MAX_WINDOWS],
    sbinfo: GASubbandInfo,
    pub coeffs: [f32; 1024],
    delay: [f32; 1024],
}

impl Ics {
    pub fn new(sbinfo: GASubbandInfo) -> Self {
        Self {
            global_gain: 0,
            info: IcsInfo::new(),
            pulse: None,
            tns: None,
            sect_cb: [[0; MAX_SFBS]; MAX_WINDOWS],
            sect_len: [[0; MAX_SFBS]; MAX_WINDOWS],
            sfb_cb: [[0; MAX_SFBS]; MAX_WINDOWS],
            scales: [[0.0; MAX_SFBS]; MAX_WINDOWS],
            num_sec: [0; MAX_WINDOWS],
            sbinfo,
            coeffs: [0.0; 1024],
            delay: [0.0; 1024],
        }
    }

    pub fn reset(&mut self) {
        self.info = IcsInfo::new();
        self.delay = [0.0; 1024];
    }

    fn decode_section_data(&mut self, br: &mut BitReader) -> io::Result<()> {
        let sect_bits = if self.info.long_win { 5 } else { 3 };
        let sect_esc_val = (1 << sect_bits) - 1;

        for g in 0..self.info.window_groups {
            let mut k = 0;
            let mut l = 0;

            while k < self.info.max_sfb {
                if l >= MAX_SFBS {
                    return Err(decode_error("aac: too many sections"));
                }

                self.sect_cb[g][l] = br.read_bits(4)? as u8;
                self.sect_len[g][l] = 0;

                if self.sect_cb[g][l] == RESERVED_HCB {
                    return Err(decode_error("aac: invalid band type"));
                }

                loop {
                    let sect_len_incr = br.read_bits(sect_bits)? as usize;
                    self.sect_len[g][l] += sect_len_incr;
                    if sect_len_incr < sect_esc_val {
                        break;
                    }
                }

                if k + self.sect_len[g][l] > self.info.max_sfb {
                    return Err(decode_error("aac: invalid section length"));
                }

                for sfb in k..k + self.sect_len[g][l] {
                    self.sfb_cb[g][sfb] = self.sect_cb[g][l];
                }

                k += self.sect_len[g][l];
                l += 1;
            }

            self.num_sec[g] = l;
        }
        Ok(())
    }

    pub fn is_zero(&self, g: usize, sfb: usize) -> bool {
        self.sfb_cb[g][sfb] == ZERO_HCB
    }

    pub fn is_intensity(&self, g: usize, sfb: usize) -> bool {
        self.sfb_cb[g][sfb] == INTENSITY_HCB || self.sfb_cb[g][sfb] == INTENSITY_HCB2
    }

    pub fn is_noise(&self, g: usize, sfb: usize) -> bool {
        self.sfb_cb[g][sfb] == NOISE_HCB
    }

    pub fn get_intensity_dir(&self, g: usize, sfb: usize) -> bool {
        self.sfb_cb[g][sfb] == INTENSITY_HCB
    }

    pub fn decode_info(&mut self, br: &mut BitReader) -> io::Result<()> {
        self.info.decode(br)?;
        if self.info.max_sfb + 1 > self.get_bands().len() {
            return Err(decode_error(
                "aac: ics info max_sfb is too big for the bands size",
            ));
        }
        Ok(())
    }

    fn decode_scale_factor_data(&mut self, br: &mut BitReader) -> io::Result<()> {
        let mut noise_pcm_flag = true;
        let mut scf_intensity = -INTENSITY_SCALE_MIN;
        let mut scf_noise = i16::from(self.global_gain) - 90 - NORMAL_SCALE_MIN;
        let mut scf_normal = i16::from(self.global_gain);

        for g in 0..self.info.window_groups {
            for sfb in 0..self.info.max_sfb {
                self.scales[g][sfb] = if self.is_zero(g, sfb) {
                    0.0
                } else if self.is_intensity(g, sfb) {
                    scf_intensity += codebooks::decode_scalefactor(br)?;
                    if !(0..256).contains(&scf_intensity) {
                        return Err(decode_error("aac: invalid intensity scale factor"));
                    }
                    INTENSITY_SCF_TABLE[scf_intensity as usize]
                } else if self.is_noise(g, sfb) {
                    if noise_pcm_flag {
                        noise_pcm_flag = false;
                        scf_noise += (br.read_bits(9)? as i16) - 256;
                    } else {
                        scf_noise += codebooks::decode_scalefactor(br)?;
                    }
                    if !(0..256).contains(&scf_noise) {
                        return Err(decode_error("aac: invalid noise scale factor"));
                    }
                    NORMAL_SCF_TABLE[scf_noise as usize]
                } else {
                    scf_normal += codebooks::decode_scalefactor(br)?;
                    if !(0..256).contains(&scf_normal) {
                        return Err(decode_error("aac: invalid scale factor"));
                    }
                    NORMAL_SCF_TABLE[scf_normal as usize]
                }
            }
        }
        Ok(())
    }

    pub fn get_bands(&self) -> &'static [usize] {
        if self.info.long_win {
            self.sbinfo.long_bands
        } else {
            self.sbinfo.short_bands
        }
    }

    fn decode_spectrum(&mut self, br: &mut BitReader, lcg: &mut Lcg) -> io::Result<()> {
        self.coeffs.fill(0.0);
        let bands = self.get_bands();

        for g in 0..self.info.window_groups {
            let cur_w = self.info.get_group_start(g);
            let next_w = self.info.get_group_start(g + 1);
            for sfb in 0..self.info.max_sfb {
                let start = bands[sfb];
                let end = bands[sfb + 1];
                let cb_idx = self.sfb_cb[g][sfb];
                let scale = self.scales[g][sfb];

                for w in cur_w..next_w {
                    let dst = &mut self.coeffs[start + w * 128..end + w * 128];

                    match cb_idx {
                        ZERO_HCB | RESERVED_HCB | INTENSITY_HCB2 | INTENSITY_HCB => (),
                        NOISE_HCB => decode_noise(lcg, scale, dst),
                        1..=11 => codebooks::decode_band(br, cb_idx, scale, dst)?,
                        _ => unreachable!(),
                    }
                }
            }
        }
        Ok(())
    }

    pub fn decode(
        &mut self,
        br: &mut BitReader,
        lcg: &mut Lcg,
        common_window: bool,
    ) -> io::Result<()> {
        self.global_gain = br.read_bits(8)? as u8;

        if !common_window {
            self.decode_info(br)?;
        }

        self.decode_section_data(br)?;
        self.decode_scale_factor_data(br)?;

        self.pulse = Pulse::read(br)?;
        if self.pulse.is_some() && !self.info.long_win {
            return Err(decode_error("aac: pulse data in short window"));
        }

        self.tns = Tns::read(br, &self.info)?;

        if br.read_bit()? {
            return Err(unsupported_error("aac: gain control data"));
        }

        self.decode_spectrum(br, lcg)?;
        Ok(())
    }

    pub fn synth_channel(&mut self, dsp: &mut dsp::Dsp, rate_idx: usize, dst: &mut [f32]) {
        let bands = self.get_bands();

        if let Some(pulse) = &self.pulse {
            pulse.synth(bands, &self.scales, &mut self.coeffs);
        }

        if let Some(tns) = &self.tns {
            tns.synth(&self.info, bands, rate_idx, &mut self.coeffs);
        }

        dsp.synth(
            &self.coeffs,
            &mut self.delay,
            self.info.window_sequence,
            self.info.window_shape,
            self.info.prev_window_shape,
            dst,
        );
    }
}

fn decode_noise(lcg: &mut Lcg, sf: f32, dst: &mut [f32]) {
    let mut energy = 0.0;

    for spec in dst.iter_mut() {
        *spec = f32::from((lcg.next_i32() >> 16) as i16);
        energy += *spec * *spec;
    }

    let scale = sf / energy.sqrt();

    for spec in dst.iter_mut() {
        *spec *= scale;
    }
}

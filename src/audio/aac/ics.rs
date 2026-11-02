use crate::audio::aac::bitstream::BitReader;
use crate::audio::aac::huffman;
use std::io;

pub const MAX_WINDOWS: usize = 8;
pub const MAX_SCALE_FACTOR_BANDS: usize = 64;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WindowSequence {
    OnlyLong,
    LongStart,
    EightShort,
    LongStop,
}

impl WindowSequence {
    pub fn from_u8(value: u8) -> io::Result<Self> {
        match value {
            0 => Ok(WindowSequence::OnlyLong),
            1 => Ok(WindowSequence::LongStart),
            2 => Ok(WindowSequence::EightShort),
            3 => Ok(WindowSequence::LongStop),
            _ => Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "Invalid window sequence",
            )),
        }
    }

    pub fn num_windows(self) -> usize {
        match self {
            WindowSequence::EightShort => 8,
            _ => 1,
        }
    }

    pub fn is_long(self) -> bool {
        !matches!(self, WindowSequence::EightShort)
    }
}

#[derive(Debug, Clone)]
pub struct IcsInfo {
    pub window_sequence: WindowSequence,
    pub window_shape: bool,
    pub max_sfb: u8,
    pub scale_factor_grouping: u8,
    pub predictor_data_present: bool,
}

impl IcsInfo {
    pub fn decode(br: &mut BitReader) -> io::Result<Self> {
        br.read_bit()?;

        let window_sequence = WindowSequence::from_u8(br.read_bits(2)? as u8)?;
        let window_shape = br.read_bit()?;

        let (max_sfb, scale_factor_grouping) = if window_sequence == WindowSequence::EightShort {
            let max_sfb = br.read_bits(4)? as u8;
            let scale_factor_grouping = br.read_bits(7)? as u8;
            (max_sfb, scale_factor_grouping)
        } else {
            let max_sfb = br.read_bits(6)? as u8;
            (max_sfb, 0)
        };

        let predictor_data_present = br.read_bit()?;
        if predictor_data_present {
            return Err(io::Error::new(
                io::ErrorKind::Unsupported,
                "Predictor data not supported",
            ));
        }

        Ok(IcsInfo {
            window_sequence,
            window_shape,
            max_sfb,
            scale_factor_grouping,
            predictor_data_present,
        })
    }

    pub fn num_window_groups(&self) -> usize {
        if self.window_sequence == WindowSequence::EightShort {
            let mut groups = 1;
            let mut mask = self.scale_factor_grouping;
            for _ in 0..7 {
                if (mask & 0x40) == 0 {
                    groups += 1;
                }
                mask <<= 1;
            }
            groups
        } else {
            1
        }
    }

    pub fn window_group_length(&self, g: usize) -> usize {
        if self.window_sequence != WindowSequence::EightShort {
            return 1;
        }

        let mut len = 1;
        let mut win = g;
        let mut mask = self.scale_factor_grouping >> (6 - g);

        while win < 7 && (mask & 1) != 0 {
            len += 1;
            win += 1;
            mask >>= 1;
        }

        len
    }
}

#[derive(Debug, Clone)]
pub struct Section {
    pub codebook: u8,
    pub start: usize,
    pub end: usize,
}

#[derive(Debug)]
pub struct IndividualChannelStream {
    pub ics_info: IcsInfo,
    pub global_gain: u8,
    pub pulse_data_present: bool,
    pub tns_data_present: bool,
    pub gain_control_present: bool,
    pub scale_factors: Vec<i16>,
    pub sections: Vec<Section>,
    pub spectral_data: Vec<f32>,
}

impl IndividualChannelStream {
    pub fn decode(br: &mut BitReader, common_window: bool) -> io::Result<Self> {
        let global_gain = br.read_bits(8)? as u8;

        let ics_info = if !common_window {
            IcsInfo::decode(br)?
        } else {
            return Err(io::Error::new(
                io::ErrorKind::Unsupported,
                "Common window not yet supported",
            ));
        };

        let (sections, scale_factors) = Self::decode_section_data(br, &ics_info)?;

        let pulse_data_present = br.read_bit()?;
        if pulse_data_present {
            return Err(io::Error::new(
                io::ErrorKind::Unsupported,
                "Pulse data not supported",
            ));
        }

        let tns_data_present = br.read_bit()?;

        let gain_control_present = br.read_bit()?;
        if gain_control_present {
            return Err(io::Error::new(
                io::ErrorKind::Unsupported,
                "Gain control not supported",
            ));
        }

        let spectral_data = Self::decode_spectral_data(br, &sections, &scale_factors)?;

        Ok(IndividualChannelStream {
            ics_info,
            global_gain,
            pulse_data_present,
            tns_data_present,
            gain_control_present,
            scale_factors,
            sections,
            spectral_data,
        })
    }

    fn decode_section_data(
        br: &mut BitReader,
        ics_info: &IcsInfo,
    ) -> io::Result<(Vec<Section>, Vec<i16>)> {
        let num_groups = ics_info.num_window_groups();
        let sect_len_bits = if ics_info.window_sequence.is_long() {
            5
        } else {
            3
        };

        let mut sections = Vec::new();
        let mut scale_factors = Vec::new();

        for _g in 0..num_groups {
            let mut sect_start = 0;

            loop {
                let sect_cb = br.read_bits(4)? as u8;

                let mut sect_len = 0;
                loop {
                    let len_incr = br.read_bits(sect_len_bits)? as usize;
                    sect_len += len_incr;
                    if len_incr != (1 << sect_len_bits) - 1 {
                        break;
                    }
                }

                let sect_end = sect_start + sect_len;

                if sect_cb != 0 {
                    sections.push(Section {
                        codebook: sect_cb,
                        start: sect_start,
                        end: sect_end,
                    });
                }

                sect_start = sect_end;
                if sect_start >= ics_info.max_sfb as usize {
                    break;
                }
            }
        }

        let mut noise_pcm_flag = br.read_bit()?;

        for section in &sections {
            if section.codebook < 11 {
                for _ in section.start..section.end {
                    let sf_delta = Self::decode_scale_factor(br)?;
                    scale_factors.push(sf_delta);
                }
            } else if section.codebook >= 13 {
                for _ in section.start..section.end {
                    if noise_pcm_flag {
                        let noise_val = br.read_bits(9)? as i16 - 256;
                        scale_factors.push(noise_val);
                    } else {
                        let sf_delta = Self::decode_scale_factor(br)?;
                        scale_factors.push(sf_delta);
                    }
                    noise_pcm_flag = false;
                }
            }
        }

        Ok((sections, scale_factors))
    }

    fn decode_scale_factor(br: &mut BitReader) -> io::Result<i16> {
        let mut value = 0i16;

        loop {
            let code = br.read_bits(2)? as u8;
            match code {
                0 => break,
                1 => value += 1,
                2 => value -= 1,
                3 => value += 2,
                _ => unreachable!(),
            }
        }

        Ok(value)
    }

    fn decode_spectral_data(
        br: &mut BitReader,
        sections: &[Section],
        scale_factors: &[i16],
    ) -> io::Result<Vec<f32>> {
        let mut spectral = vec![0.0f32; 1024];
        let mut sf_idx = 0;

        for section in sections {
            let codebook = huffman::get_codebook(section.codebook)
                .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "Invalid codebook"))?;

            for sfb in section.start..section.end {
                let scale_factor = if sf_idx < scale_factors.len() {
                    scale_factors[sf_idx]
                } else {
                    0
                };
                sf_idx += 1;

                let sfb_width = 4;
                let sfb_start = sfb * sfb_width;

                for i in (0..sfb_width).step_by(2) {
                    let (w, x) = codebook.decode(br)?;

                    if sfb_start + i < spectral.len() {
                        spectral[sfb_start + i] = w as f32 * (scale_factor as f32 * 0.25).exp2();
                    }
                    if sfb_start + i + 1 < spectral.len() {
                        spectral[sfb_start + i + 1] =
                            x as f32 * (scale_factor as f32 * 0.25).exp2();
                    }
                }
            }
        }

        Ok(spectral)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_window_sequence() {
        assert_eq!(
            WindowSequence::from_u8(0).unwrap(),
            WindowSequence::OnlyLong
        );
        assert_eq!(
            WindowSequence::from_u8(2).unwrap(),
            WindowSequence::EightShort
        );
        assert!(WindowSequence::from_u8(4).is_err());
    }

    #[test]
    fn counts_windows() {
        assert_eq!(WindowSequence::OnlyLong.num_windows(), 1);
        assert_eq!(WindowSequence::EightShort.num_windows(), 8);
    }

    #[test]
    fn detects_long_windows() {
        assert!(WindowSequence::OnlyLong.is_long());
        assert!(!WindowSequence::EightShort.is_long());
    }
}

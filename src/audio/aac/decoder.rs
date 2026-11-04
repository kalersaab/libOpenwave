// Symphonia AAC-LC decoder port.
// Copyright (c) 2019-2026 The Project Symphonia Developers.
// This file is licensed under the Mozilla Public License, v. 2.0.

use crate::audio::aac::bitstream::BitReader;
use crate::audio::aac::common::*;
use crate::audio::aac::config::AacConfig;
use crate::audio::aac::cpe::ChannelPair;
use crate::audio::aac::dsp::Dsp;
use std::io;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ElementType {
    SingleChannel = 0,
    ChannelPair = 1,
    ChannelPairCoupling = 2,
    LowFrequencyEffects = 3,
    DataStreamElement = 4,
    ProgramConfigElement = 5,
    FillElement = 6,
    End = 7,
}

impl ElementType {
    fn from_u8(value: u8) -> Option<Self> {
        match value {
            0 => Some(ElementType::SingleChannel),
            1 => Some(ElementType::ChannelPair),
            2 => Some(ElementType::ChannelPairCoupling),
            3 => Some(ElementType::LowFrequencyEffects),
            4 => Some(ElementType::DataStreamElement),
            5 => Some(ElementType::ProgramConfigElement),
            6 => Some(ElementType::FillElement),
            7 => Some(ElementType::End),
            _ => None,
        }
    }
}

pub struct AacDecoder {
    config: AacConfig,
    sbinfo: GASubbandInfo,
    pairs: Vec<ChannelPair>,
    dsp: Dsp,
}

impl AacDecoder {
    pub fn new(config: AacConfig) -> Self {
        let sbinfo = GASubbandInfo::find(config.sample_rate_hz());
        AacDecoder {
            config,
            sbinfo,
            pairs: Vec::new(),
            dsp: Dsp::new(),
        }
    }

    pub fn from_audio_specific_config(data: &[u8]) -> io::Result<Self> {
        let config = AacConfig::from_audio_specific_config(data)?;
        Ok(AacDecoder::new(config))
    }

    pub fn config(&self) -> &AacConfig {
        &self.config
    }

    pub fn decode_frame(&mut self, data: &[u8]) -> io::Result<Vec<f32>> {
        let header_size = if self.is_adts(data) {
            if data.len() < 7 {
                return Err(decode_error("aac: adts header too short"));
            }
            if (data[1] & 1) == 0 { 9 } else { 7 }
        } else {
            0
        };

        if data.len() < header_size {
            return Err(decode_error("aac: frame too short"));
        }

        let frame_data = &data[header_size..];
        let mut br = BitReader::new(frame_data);

        let mut cur_pair = 0;
        let mut cur_ch = 0;

        while br.bits_available() > 3 {
            let id = br.read_bits(3)? as u8;
            let elem_type = ElementType::from_u8(id)
                .ok_or_else(|| decode_error("aac: invalid element type"))?;

            match elem_type {
                ElementType::SingleChannel | ElementType::LowFrequencyEffects => {
                    let _tag = br.read_bits(4)?;
                    self.set_pair(cur_pair, cur_ch, false)?;
                    self.pairs[cur_pair].decode_ga_sce(&mut br)?;
                    cur_pair += 1;
                    cur_ch += 1;
                }
                ElementType::ChannelPair => {
                    let _tag = br.read_bits(4)?;
                    self.set_pair(cur_pair, cur_ch, true)?;
                    self.pairs[cur_pair].decode_ga_cpe(&mut br)?;
                    cur_pair += 1;
                    cur_ch += 2;
                }
                ElementType::ChannelPairCoupling => {
                    return Err(unsupported_error("aac: coupling channel element"));
                }
                ElementType::ProgramConfigElement => {
                    return Err(unsupported_error("aac: program config element"));
                }
                ElementType::DataStreamElement => {
                    let _id = br.read_bits(4)?;
                    let align = br.read_bit()?;
                    let mut count = br.read_bits(8)? as usize;
                    if count == 255 {
                        count += br.read_bits(8)? as usize;
                    }
                    if align {
                        br.byte_align();
                    }
                    br.ignore_bits(count * 8)?;
                }
                ElementType::FillElement => {
                    let mut count = br.read_bits(4)? as usize;
                    if count == 15 {
                        count += br.read_bits(8)? as usize;
                        count -= 1;
                    }

                    if count > 0 {
                        let ext_type = br.read_bits(4)?;
                        if ext_type == 0xd || ext_type == 0xe {
                            return Err(unsupported_error("aac: sbr"));
                        }
                        br.ignore_bits(4)?;
                        if count > 1 {
                            br.ignore_bits((count - 1) * 8)?;
                        }
                    }
                }
                ElementType::End => break,
            }
        }

        let channels = self.config.channels as usize;
        let frame_len = self.config.frame_length as usize;

        if cur_pair == 0 {
            return Ok(vec![0.0; frame_len * channels]);
        }

        let rate_idx = GASubbandInfo::find_idx(self.config.sample_rate_hz());
        let mut planes = vec![vec![0.0f32; frame_len]; channels];

        for i in 0..cur_pair {
            let channel = self.pairs[i].channel;
            if channel >= channels {
                continue;
            }
            let (left, right) = planes.split_at_mut(channel + 1);
            let left_plane = &mut left[channel];
            let right_plane = if self.pairs[i].is_pair && channel + 1 < channels {
                Some(&mut right[0])
            } else {
                None
            };

            if let Some(right_plane) = right_plane {
                self.pairs[i].synth_audio(&mut self.dsp, rate_idx, left_plane, right_plane);
            } else {
                let mut scratch = vec![0.0f32; frame_len];
                self.pairs[i].synth_audio(&mut self.dsp, rate_idx, left_plane, &mut scratch);
            }
        }

        let mut output = vec![0.0f32; frame_len * channels];
        for (ch, plane) in planes.iter().enumerate() {
            for (i, &sample) in plane.iter().enumerate() {
                output[i * channels + ch] = sample;
            }
        }

        Ok(output)
    }

    fn set_pair(&mut self, pair_no: usize, channel: usize, is_pair: bool) -> io::Result<()> {
        let channels = self.config.channels as usize;
        if is_pair {
            if channel + 1 >= channels {
                return Err(decode_error("aac: channel pair exceeds channel count"));
            }
        } else if channel >= channels {
            return Err(decode_error("aac: channel exceeds channel count"));
        }

        if self.pairs.len() <= pair_no {
            self.pairs
                .push(ChannelPair::new(is_pair, channel, self.sbinfo));
        } else {
            if self.pairs[pair_no].channel != channel || self.pairs[pair_no].is_pair != is_pair {
                return Err(decode_error("aac: channel layout changed"));
            }
        }

        Ok(())
    }

    fn is_adts(&self, data: &[u8]) -> bool {
        data.len() >= 7 && data[0] == 0xFF && (data[1] & 0xF0) == 0xF0
    }

    pub fn samples_per_frame(&self) -> usize {
        self.config.frame_length as usize
    }

    pub fn channels(&self) -> u8 {
        self.config.channels
    }

    pub fn sample_rate(&self) -> u32 {
        self.config.sample_rate_hz()
    }
}

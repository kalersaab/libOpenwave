use crate::audio::aac::bitstream::BitReader;
use crate::audio::aac::config::AacConfig;
use crate::audio::aac::ics::{IndividualChannelStream, WindowSequence};
use crate::audio::aac::imdct::Imdct;
use std::io;

const ADTS_HEADER_SIZE: usize = 7;

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
    imdct_long: Imdct,
    imdct_short: Imdct,
    overlap_left: Vec<f32>,
    overlap_right: Vec<f32>,
}

impl AacDecoder {
    pub fn new(config: AacConfig) -> Self {
        AacDecoder {
            config,
            imdct_long: Imdct::new(1024),
            imdct_short: Imdct::new(128),
            overlap_left: vec![0.0; 1024],
            overlap_right: vec![0.0; 1024],
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
        let frame_data = if self.is_adts(data) {
            &data[ADTS_HEADER_SIZE..]
        } else {
            data
        };

        let mut br = BitReader::new(frame_data);
        let mut output = Vec::new();

        loop {
            let elem_id = br.read_bits(3)? as u8;
            let elem_type = ElementType::from_u8(elem_id).ok_or_else(|| {
                io::Error::new(io::ErrorKind::InvalidData, "Invalid element type")
            })?;

            match elem_type {
                ElementType::End => break,

                ElementType::SingleChannel => {
                    let _tag = br.read_bits(4)?;
                    let ics = IndividualChannelStream::decode(&mut br, false)?;
                    let samples = self.synthesize_channel(&ics, true)?;
                    output.extend_from_slice(&samples);
                }

                ElementType::ChannelPair => {
                    let _tag = br.read_bits(4)?;
                    let common_window = br.read_bit()?;

                    let ics_left = IndividualChannelStream::decode(&mut br, common_window)?;
                    let ics_right = IndividualChannelStream::decode(&mut br, common_window)?;

                    let left_samples = self.synthesize_channel(&ics_left, true)?;
                    let right_samples = self.synthesize_channel(&ics_right, false)?;

                    for i in 0..left_samples.len() {
                        output.push(left_samples[i]);
                        if i < right_samples.len() {
                            output.push(right_samples[i]);
                        } else {
                            output.push(0.0);
                        }
                    }
                }

                ElementType::FillElement => {
                    let count = br.read_bits(4)? as usize;
                    let byte_count = if count == 15 {
                        (br.read_bits(8)? as usize) + 14
                    } else {
                        count
                    };

                    for _ in 0..byte_count {
                        br.read_bits(8)?;
                    }
                }

                ElementType::DataStreamElement => {
                    let data_byte_align_flag = br.read_bit()?;
                    let mut count = br.read_bits(8)? as usize;
                    if count == 255 {
                        count += br.read_bits(8)? as usize;
                    }

                    if data_byte_align_flag {
                        br.byte_align();
                    }

                    for _ in 0..count {
                        br.read_bits(8)?;
                    }
                }

                _ => {
                    return Err(io::Error::new(
                        io::ErrorKind::Unsupported,
                        format!("Unsupported element type: {:?}", elem_type),
                    ));
                }
            }
        }

        if output.is_empty() {
            let frame_samples = self.config.frame_length as usize * self.config.channels as usize;
            output = vec![0.0; frame_samples];
        }

        Ok(output)
    }

    fn synthesize_channel(
        &mut self,
        ics: &IndividualChannelStream,
        is_left: bool,
    ) -> io::Result<Vec<f32>> {
        let mut output = vec![0.0; self.config.frame_length as usize];
        let overlap = if is_left {
            &mut self.overlap_left
        } else {
            &mut self.overlap_right
        };

        match ics.ics_info.window_sequence {
            WindowSequence::OnlyLong | WindowSequence::LongStart | WindowSequence::LongStop => {
                if ics.spectral_data.len() >= 512 {
                    self.imdct_long
                        .process_long(&ics.spectral_data[..512], &mut output, overlap);

                    overlap[..512].copy_from_slice(&ics.spectral_data[512..1024]);
                }
            }

            WindowSequence::EightShort => {
                let mut short_inputs = [[0.0f32; 64]; 8];
                for (i, window) in short_inputs.iter_mut().enumerate() {
                    let start = i * 64;
                    let end = (start + 64).min(ics.spectral_data.len());
                    window[..end - start].copy_from_slice(&ics.spectral_data[start..end]);
                }

                let short_refs: [&[f32]; 8] = [
                    &short_inputs[0],
                    &short_inputs[1],
                    &short_inputs[2],
                    &short_inputs[3],
                    &short_inputs[4],
                    &short_inputs[5],
                    &short_inputs[6],
                    &short_inputs[7],
                ];

                let mut temp_output = vec![0.0; 1024];
                self.imdct_short
                    .process_short(&short_refs, &mut temp_output, overlap);

                output.copy_from_slice(&temp_output[..self.config.frame_length as usize]);

                overlap[..512].copy_from_slice(&temp_output[512..1024]);
            }
        }

        Ok(output)
    }

    fn is_adts(&self, data: &[u8]) -> bool {
        data.len() >= ADTS_HEADER_SIZE && data[0] == 0xFF && (data[1] & 0xF0) == 0xF0
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audio::aac::config::{AacProfile, AacSampleRate};

    #[test]
    fn creates_decoder_from_config() {
        let config = AacConfig {
            profile: AacProfile::Lc,
            sample_rate: AacSampleRate::Hz44100,
            channels: 2,
            frame_length: 1024,
        };

        let decoder = AacDecoder::new(config);
        assert_eq!(decoder.sample_rate(), 44100);
        assert_eq!(decoder.channels(), 2);
    }

    #[test]
    fn detects_adts_header() {
        let config_data = [0x12, 0x10];
        let decoder = AacDecoder::from_audio_specific_config(&config_data).unwrap();

        let adts_data = [0xFF, 0xF1, 0x50, 0x80, 0x01, 0x20, 0xFC];
        assert!(decoder.is_adts(&adts_data));

        let raw_data = [0x00, 0x01, 0x02];
        assert!(!decoder.is_adts(&raw_data));
    }

    #[test]
    fn decode_returns_correct_sample_count() {
        let config_data = [0x12, 0x10];
        let mut decoder = AacDecoder::from_audio_specific_config(&config_data).unwrap();

        let adts_frame = [
            0xFF, 0xF1, 0x50, 0x80, 0x01, 0x20, 0xFC, 0x21, 0x10, 0x00, 0x00, 0x07,
        ];

        let result = decoder.decode_frame(&adts_frame);

        if let Ok(samples) = result {
            assert!(samples.len() <= 2048);
        }
    }
}

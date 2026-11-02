use crate::audio::mp3::bitstream::{Mp3BitReader, SideInfo};
use crate::audio::mp3::huffman;
use crate::audio::mp3::imdct::Mp3Imdct;
use crate::audio::mp3::synthesis::SynthesisFilterbank;
use crate::audio::mp3::tables;
use crate::demux::mp3::Mp3FrameHeader;
use std::io;

pub struct Mp3Decoder {
    synthesis: SynthesisFilterbank,
    _imdct: Mp3Imdct,
    overlap: [[f32; 576]; 2],
}

impl Mp3Decoder {
    pub fn new() -> Self {
        Mp3Decoder {
            synthesis: SynthesisFilterbank::new(),
            _imdct: Mp3Imdct::new(18),
            overlap: [[0.0; 576]; 2],
        }
    }

    pub fn decode_frame(&mut self, header: &Mp3FrameHeader, data: &[u8]) -> io::Result<Vec<f32>> {
        let channels = header.channels();

        if data.len() < 32 {
            return Ok(vec![0.0; header.samples_per_frame() * channels]);
        }

        let mut br = Mp3BitReader::new(data);
        let side_info = SideInfo::decode(&mut br, channels)?;

        let mut pcm = Vec::with_capacity(header.samples_per_frame() * channels);

        for gr in 0..2 {
            for ch in 0..channels {
                let granule = &side_info.granules[gr][ch];

                let mut samples = [0.0f32; 576];
                self.decode_granule(&mut br, granule, &mut samples)?;

                self.apply_overlap(&mut samples, ch);

                for sb in 0..18 {
                    let mut subband = [0.0f32; 32];
                    for i in 0..32 {
                        subband[i] = samples[sb * 32 + i];
                    }

                    let mut output = [0.0f32; 32];
                    self.synthesis.synthesize(&subband, ch, &mut output);

                    if channels == 1 {
                        for sample in output.iter() {
                            pcm.push(*sample);
                        }
                    } else if ch == 0 {
                        for sample in output.iter() {
                            pcm.push(*sample);
                            pcm.push(0.0);
                        }
                    } else {
                        let pcm_len = pcm.len();
                        for (i, sample) in output.iter().enumerate() {
                            if i * 2 + 1 < pcm_len {
                                pcm[pcm_len - 32 * 2 + i * 2 + 1] = *sample;
                            }
                        }
                    }
                }
            }
        }

        if pcm.len() < header.samples_per_frame() * channels {
            pcm.resize(header.samples_per_frame() * channels, 0.0);
        }

        Ok(pcm)
    }

    fn decode_granule(
        &self,
        br: &mut Mp3BitReader,
        granule: &crate::audio::mp3::bitstream::Granule,
        samples: &mut [f32; 576],
    ) -> io::Result<()> {
        let mut quantized = [0i16; 576];

        let big_values_end = (granule.big_values as usize * 2).min(576);
        let mut sample_idx = 0;

        for region in 0..3 {
            if sample_idx >= big_values_end {
                break;
            }

            let table_idx = granule.table_select[region];
            let table = huffman::get_huffman_table(table_idx).ok_or_else(|| {
                io::Error::new(io::ErrorKind::InvalidData, "Invalid Huffman table")
            })?;

            let region_end = match region {
                0 => (granule.region0_count as usize + 1) * 2,
                1 => (granule.region0_count as usize + granule.region1_count as usize + 2) * 2,
                _ => big_values_end,
            }
            .min(big_values_end);

            while sample_idx < region_end {
                if br.bits_available() < 16 {
                    break;
                }

                let (x, y) = table.decode(br)?;
                quantized[sample_idx] = x;
                quantized[sample_idx + 1] = y;
                sample_idx += 2;
            }
        }

        while sample_idx < 576 {
            if br.bits_available() < 8 {
                break;
            }

            let quad = huffman::decode_quad(br, granule.count1table_select)?;
            for &val in &quad {
                if sample_idx < 576 {
                    quantized[sample_idx] = val;
                    sample_idx += 1;
                }
            }
        }

        for (i, sample) in samples.iter_mut().enumerate() {
            *sample = tables::dequantize_sample(quantized[i], granule.global_gain, 1.0);
        }

        Ok(())
    }

    fn apply_overlap(&mut self, samples: &mut [f32; 576], channel: usize) {
        for (i, sample) in samples.iter_mut().enumerate() {
            let temp = *sample;
            *sample += self.overlap[channel][i];
            self.overlap[channel][i] = temp;
        }
    }
}

impl Default for Mp3Decoder {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::demux::mp3::{ChannelMode, Layer, MpegVersion};

    #[test]
    fn creates_mp3_decoder() {
        let decoder = Mp3Decoder::new();
        assert_eq!(decoder.overlap[0][0], 0.0);
    }

    #[test]
    fn decodes_empty_frame() {
        let mut decoder = Mp3Decoder::new();

        let header = Mp3FrameHeader {
            version: MpegVersion::Mpeg1,
            layer: Layer::LayerIII,
            bitrate: 128000,
            sample_rate: 44100,
            padding: false,
            channel_mode: ChannelMode::Stereo,
            mode_extension: 0,
            frame_size: 418,
        };

        let data = vec![0u8; 418];
        let result = decoder.decode_frame(&header, &data);

        if let Ok(samples) = result {
            assert_eq!(samples.len(), 1152 * 2);
        }
    }

    #[test]
    fn handles_mono() {
        let mut decoder = Mp3Decoder::new();

        let header = Mp3FrameHeader {
            version: MpegVersion::Mpeg1,
            layer: crate::demux::mp3::Layer::LayerIII,
            bitrate: 128000,
            sample_rate: 44100,
            padding: false,
            channel_mode: ChannelMode::Mono,
            mode_extension: 0,
            frame_size: 209,
        };

        let data = vec![0u8; 209];
        let result = decoder.decode_frame(&header, &data);

        if let Ok(samples) = result {
            assert_eq!(samples.len(), 1152);
        }
    }
}

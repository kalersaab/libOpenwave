use std::error::Error;
use std::fmt;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PcmFormat {
    pub sample_rate: u32,
    pub channels: u16,
    pub bits_per_sample: u16,
}

impl PcmFormat {
    pub fn bytes_per_frame(self) -> Option<usize> {
        if self.sample_rate == 0 || self.channels == 0 {
            return None;
        }

        match self.bits_per_sample {
            8 | 16 | 24 | 32 => Some(self.channels as usize * (self.bits_per_sample / 8) as usize),
            _ => None,
        }
    }
}

#[derive(Debug)]
pub enum PcmError {
    UnsupportedBitDepth(u16),
    InvalidFrameAlignment,
}

impl fmt::Display for PcmError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{self:?}")
    }
}

impl Error for PcmError {}

/// Decode integer PCM WAV data to normalized f32 samples.
/// Samples are interleaved by channel.
pub fn decode_integer_pcm(bytes: &[u8], bits_per_sample: u16) -> Result<Vec<f32>, PcmError> {
    match bits_per_sample {
        8 => Ok(bytes
            .iter()
            .map(|&sample| (sample as f32 - 128.0) / 128.0)
            .collect()),

        16 => {
            if bytes.len() % 2 != 0 {
                return Err(PcmError::InvalidFrameAlignment);
            }

            Ok(bytes
                .chunks_exact(2)
                .map(|b| i16::from_le_bytes([b[0], b[1]]) as f32 / 32768.0)
                .collect())
        }

        24 => {
            if bytes.len() % 3 != 0 {
                return Err(PcmError::InvalidFrameAlignment);
            }

            Ok(bytes
                .chunks_exact(3)
                .map(|b| {
                    let raw = (b[0] as i32) | ((b[1] as i32) << 8) | ((b[2] as i32) << 16);

                    let signed = if raw & 0x80_0000 != 0 {
                        raw | !0xFF_FFFF
                    } else {
                        raw
                    };

                    signed as f32 / 8_388_608.0
                })
                .collect())
        }

        32 => {
            if bytes.len() % 4 != 0 {
                return Err(PcmError::InvalidFrameAlignment);
            }

            Ok(bytes
                .chunks_exact(4)
                .map(|b| i32::from_le_bytes([b[0], b[1], b[2], b[3]]) as f32 / 2_147_483_648.0)
                .collect())
        }

        other => Err(PcmError::UnsupportedBitDepth(other)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_unsigned_8_bit_pcm() {
        let samples = decode_integer_pcm(&[0, 128, 255], 8).unwrap();

        assert_eq!(samples[0], -1.0);
        assert_eq!(samples[1], 0.0);
        assert!((samples[2] - 127.0 / 128.0).abs() < 0.0001);
    }

    #[test]
    fn decodes_signed_16_bit_pcm() {
        let bytes = [
            0x00, 0x80, // -32768
            0x00, 0x00, // 0
            0xFF, 0x7F, // 32767
        ];

        let samples = decode_integer_pcm(&bytes, 16).unwrap();

        assert_eq!(samples[0], -1.0);
        assert_eq!(samples[1], 0.0);
        assert!(samples[2] < 1.0);
    }

    #[test]
    fn decodes_signed_24_bit_pcm() {
        let samples = decode_integer_pcm(&[0x00, 0x00, 0x80, 0xFF, 0xFF, 0x7F], 24).unwrap();

        assert_eq!(samples[0], -1.0);
        assert!(samples[1] < 1.0);
    }

    #[test]
    fn rejects_incomplete_sample() {
        assert!(matches!(
            decode_integer_pcm(&[0x01], 16),
            Err(PcmError::InvalidFrameAlignment)
        ));
    }
}

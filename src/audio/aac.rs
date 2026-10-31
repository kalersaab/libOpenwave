// AAC (Advanced Audio Codec) decoder implementation
// Supports AAC-LC (Low Complexity) profile

use std::io;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AacProfile {
    Main = 1,
    Lc = 2,  // Low Complexity (most common)
    Ssr = 3, // Scalable Sample Rate
    Ltp = 4, // Long Term Prediction
}

impl AacProfile {
    pub fn from_u8(value: u8) -> Option<Self> {
        match value {
            1 => Some(AacProfile::Main),
            2 => Some(AacProfile::Lc),
            3 => Some(AacProfile::Ssr),
            4 => Some(AacProfile::Ltp),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AacSampleRate {
    Hz96000 = 0,
    Hz88200 = 1,
    Hz64000 = 2,
    Hz48000 = 3,
    Hz44100 = 4,
    Hz32000 = 5,
    Hz24000 = 6,
    Hz22050 = 7,
    Hz16000 = 8,
    Hz12000 = 9,
    Hz11025 = 10,
    Hz8000 = 11,
    Hz7350 = 12,
}

impl AacSampleRate {
    pub fn from_index(index: u8) -> Option<Self> {
        match index {
            0 => Some(AacSampleRate::Hz96000),
            1 => Some(AacSampleRate::Hz88200),
            2 => Some(AacSampleRate::Hz64000),
            3 => Some(AacSampleRate::Hz48000),
            4 => Some(AacSampleRate::Hz44100),
            5 => Some(AacSampleRate::Hz32000),
            6 => Some(AacSampleRate::Hz24000),
            7 => Some(AacSampleRate::Hz22050),
            8 => Some(AacSampleRate::Hz16000),
            9 => Some(AacSampleRate::Hz12000),
            10 => Some(AacSampleRate::Hz11025),
            11 => Some(AacSampleRate::Hz8000),
            12 => Some(AacSampleRate::Hz7350),
            _ => None,
        }
    }

    pub fn to_hz(self) -> u32 {
        match self {
            AacSampleRate::Hz96000 => 96000,
            AacSampleRate::Hz88200 => 88200,
            AacSampleRate::Hz64000 => 64000,
            AacSampleRate::Hz48000 => 48000,
            AacSampleRate::Hz44100 => 44100,
            AacSampleRate::Hz32000 => 32000,
            AacSampleRate::Hz24000 => 24000,
            AacSampleRate::Hz22050 => 22050,
            AacSampleRate::Hz16000 => 16000,
            AacSampleRate::Hz12000 => 12000,
            AacSampleRate::Hz11025 => 11025,
            AacSampleRate::Hz8000 => 8000,
            AacSampleRate::Hz7350 => 7350,
        }
    }
}

#[derive(Debug, Clone)]
pub struct AacConfig {
    pub profile: AacProfile,
    pub sample_rate: AacSampleRate,
    pub channels: u8,
    pub frame_length: u16,
}

impl AacConfig {
    /// Parse AAC AudioSpecificConfig from MP4 esds or other container
    /// Format (simplified):
    /// - 5 bits: object type (profile + 1)
    /// - 4 bits: sampling frequency index
    /// - 4 bits: channel configuration
    pub fn from_audio_specific_config(data: &[u8]) -> io::Result<Self> {
        if data.len() < 2 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "AAC config too short",
            ));
        }

        let profile_index = (data[0] >> 3) & 0x1F;
        let profile = AacProfile::from_u8(profile_index)
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "Invalid AAC profile"))?;

        let sample_rate_index = ((data[0] & 0x07) << 1) | ((data[1] >> 7) & 0x01);
        let sample_rate = AacSampleRate::from_index(sample_rate_index)
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "Invalid AAC sample rate"))?;

        let channels = (data[1] >> 3) & 0x0F;
        if channels == 0 || channels > 7 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "Invalid AAC channel configuration",
            ));
        }

        // Standard AAC frame length is 1024 samples
        let frame_length = 1024;

        Ok(AacConfig {
            profile,
            sample_rate,
            channels,
            frame_length,
        })
    }

    pub fn sample_rate_hz(&self) -> u32 {
        self.sample_rate.to_hz()
    }
}

/// ADTS (Audio Data Transport Stream) header parser
/// Used for raw AAC streams (not MP4)
#[derive(Debug, Clone)]
pub struct AdtsHeader {
    pub profile: AacProfile,
    pub sample_rate: AacSampleRate,
    pub channels: u8,
    pub frame_length: u16,
}

impl AdtsHeader {
    pub fn parse(data: &[u8]) -> io::Result<Self> {
        if data.len() < 7 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "ADTS header too short",
            ));
        }

        // Sync word check (0xFFF)
        if data[0] != 0xFF || (data[1] & 0xF0) != 0xF0 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "Invalid ADTS sync word",
            ));
        }

        let profile_index = ((data[2] >> 6) & 0x03) + 1;
        let profile = AacProfile::from_u8(profile_index)
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "Invalid ADTS profile"))?;

        let sample_rate_index = (data[2] >> 2) & 0x0F;
        let sample_rate = AacSampleRate::from_index(sample_rate_index).ok_or_else(|| {
            io::Error::new(io::ErrorKind::InvalidData, "Invalid ADTS sample rate")
        })?;

        let channels = ((data[2] & 0x01) << 2) | ((data[3] >> 6) & 0x03);

        let frame_length =
            (((data[3] & 0x03) as u16) << 11) | ((data[4] as u16) << 3) | ((data[5] >> 5) as u16);

        Ok(AdtsHeader {
            profile,
            sample_rate,
            channels,
            frame_length,
        })
    }
}

/// Minimal AAC decoder stub
/// Full implementation would require MDCT, scale factor decoding, etc.
/// For now, this provides the interface for integration
pub struct AacDecoder {
    config: AacConfig,
}

impl AacDecoder {
    pub fn new(config: AacConfig) -> Self {
        AacDecoder { config }
    }

    pub fn from_audio_specific_config(data: &[u8]) -> io::Result<Self> {
        let config = AacConfig::from_audio_specific_config(data)?;
        Ok(AacDecoder::new(config))
    }

    pub fn config(&self) -> &AacConfig {
        &self.config
    }

    /// Decode an AAC frame to PCM samples
    /// Returns samples as f32 in range [-1.0, 1.0], interleaved
    /// Output length = frame_length * channels
    pub fn decode_frame(&mut self, _data: &[u8]) -> io::Result<Vec<f32>> {
        // TODO: Full AAC decoding requires:
        // 1. Bitstream parsing (Huffman tables, scale factors)
        // 2. Inverse quantization
        // 3. IMDCT (Inverse Modified Discrete Cosine Transform)
        // 4. Window and overlap-add
        // 5. TNS (Temporal Noise Shaping) if present
        // 6. MS stereo decoding if present
        //
        // For now, return silence as placeholder
        let sample_count = self.config.frame_length as usize * self.config.channels as usize;
        Ok(vec![0.0; sample_count])
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

    #[test]
    fn parses_aac_lc_44100_stereo_config() {
        // AAC-LC, 44.1kHz, stereo
        // 0x12 = 0001 0010
        //   profile=2 (LC), freq_idx=4 (44.1kHz) partial
        // 0x10 = 0001 0000
        //   freq_idx continued, channels=2
        let config_data = [0x12, 0x10];

        let config = AacConfig::from_audio_specific_config(&config_data).unwrap();

        assert_eq!(config.profile, AacProfile::Lc);
        assert_eq!(config.sample_rate, AacSampleRate::Hz44100);
        assert_eq!(config.channels, 2);
        assert_eq!(config.frame_length, 1024);
    }

    #[test]
    fn parses_aac_lc_48000_stereo_config() {
        // AAC-LC, 48kHz, stereo
        let config_data = [0x11, 0x90];

        let config = AacConfig::from_audio_specific_config(&config_data).unwrap();

        assert_eq!(config.profile, AacProfile::Lc);
        assert_eq!(config.sample_rate, AacSampleRate::Hz48000);
        assert_eq!(config.channels, 2);
    }

    #[test]
    fn rejects_invalid_aac_config() {
        // Too short
        assert!(AacConfig::from_audio_specific_config(&[0x12]).is_err());

        // Invalid profile (0)
        assert!(AacConfig::from_audio_specific_config(&[0x00, 0x10]).is_err());
    }

    #[test]
    fn parses_adts_header() {
        // ADTS header: sync(0xFFF), AAC-LC, 44.1kHz, stereo
        let adts_data = [
            0xFF, 0xF1, // Sync + MPEG-4 + no CRC
            0x50, // Profile=LC, freq=44.1kHz (partial)
            0x80, // freq+channels=2, frame_length (partial)
            0x01, 0x20, // frame_length continued
            0xFC, // buffer fullness
        ];

        let header = AdtsHeader::parse(&adts_data).unwrap();

        assert_eq!(header.profile, AacProfile::Lc);
        assert_eq!(header.sample_rate, AacSampleRate::Hz44100);
        assert_eq!(header.channels, 2);
    }

    #[test]
    fn rejects_invalid_adts_sync() {
        let invalid_adts = [0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00];
        assert!(AdtsHeader::parse(&invalid_adts).is_err());
    }

    #[test]
    fn creates_decoder_from_config() {
        let config_data = [0x12, 0x10]; // AAC-LC, 44.1kHz, stereo

        let decoder = AacDecoder::from_audio_specific_config(&config_data).unwrap();

        assert_eq!(decoder.sample_rate(), 44100);
        assert_eq!(decoder.channels(), 2);
        assert_eq!(decoder.samples_per_frame(), 1024);
    }

    #[test]
    fn decode_frame_returns_correct_size() {
        let config_data = [0x12, 0x10];
        let mut decoder = AacDecoder::from_audio_specific_config(&config_data).unwrap();

        let samples = decoder.decode_frame(&[0u8; 100]).unwrap();

        // 1024 samples/frame * 2 channels = 2048 samples
        assert_eq!(samples.len(), 2048);
    }
}

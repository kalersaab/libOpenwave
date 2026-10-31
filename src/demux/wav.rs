use std::error::Error;
use std::fmt;
use std::fs;
use std::path::Path;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WavInfo {
    pub channels: u16,
    pub sample_rate: u32,
    pub bits_per_sample: u16,
    pub block_align: u16,
    pub data_offset: usize,
    pub data_len: usize,
}

impl WavInfo {
    pub fn frame_count(&self) -> u64 {
        (self.data_len / self.block_align as usize) as u64
    }

    pub fn duration_seconds(&self) -> f64 {
        self.frame_count() as f64 / self.sample_rate as f64
    }

    pub fn bytes_per_second(&self) -> u64 {
        self.sample_rate as u64 * self.block_align as u64
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WavError {
    TooSmall,
    InvalidRiff,
    InvalidWave,
    MissingFormat,
    MissingData,
    UnsupportedFormat(u16),
    UnsupportedBitDepth(u16),
    InvalidChunk,
    InvalidFormat,
    TruncatedData,
}

impl fmt::Display for WavError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{self:?}")
    }
}

impl Error for WavError {}

#[derive(Debug)]
pub struct WavFile {
    bytes: Vec<u8>,
    pub info: WavInfo,
}

impl WavFile {
    pub fn open(path: impl AsRef<Path>) -> Result<Self, Box<dyn Error>> {
        let bytes = fs::read(path)?;
        let info = parse_wav(&bytes)?;
        Ok(Self { bytes, info })
    }

    pub fn parse(bytes: Vec<u8>) -> Result<Self, WavError> {
        let info = parse_wav(&bytes)?;
        Ok(Self { bytes, info })
    }

    pub fn pcm_data(&self) -> &[u8] {
        let start = self.info.data_offset;
        &self.bytes[start..start + self.info.data_len]
    }

    pub fn pcm_frame(&self, frame_index: usize) -> Option<&[u8]> {
        let align = self.info.block_align as usize;
        let start = frame_index.checked_mul(align)?;
        self.pcm_data().get(start..start.checked_add(align)?)
    }
}

fn u16_le(bytes: &[u8], offset: usize) -> Result<u16, WavError> {
    let slice = bytes
        .get(offset..offset + 2)
        .ok_or(WavError::InvalidChunk)?;
    Ok(u16::from_le_bytes([slice[0], slice[1]]))
}

fn u32_le(bytes: &[u8], offset: usize) -> Result<u32, WavError> {
    let slice = bytes
        .get(offset..offset + 4)
        .ok_or(WavError::InvalidChunk)?;
    Ok(u32::from_le_bytes([slice[0], slice[1], slice[2], slice[3]]))
}

fn parse_wav(bytes: &[u8]) -> Result<WavInfo, WavError> {
    if bytes.len() < 12 {
        return Err(WavError::TooSmall);
    }
    if &bytes[0..4] != b"RIFF" {
        return Err(WavError::InvalidRiff);
    }
    if &bytes[8..12] != b"WAVE" {
        return Err(WavError::InvalidWave);
    }

    let riff_size = u32_le(bytes, 4)? as usize;
    let riff_end = riff_size.checked_add(8).ok_or(WavError::InvalidChunk)?;
    if riff_end > bytes.len() || riff_end < 12 {
        return Err(WavError::InvalidChunk);
    }

    let mut cursor = 12usize;
    let mut format: Option<(u16, u16, u32, u16, u16)> = None;
    let mut data: Option<(usize, usize)> = None;

    while cursor.checked_add(8).ok_or(WavError::InvalidChunk)? <= riff_end {
        let id = &bytes[cursor..cursor + 4];
        let size = u32_le(bytes, cursor + 4)? as usize;
        let start = cursor.checked_add(8).ok_or(WavError::InvalidChunk)?;
        let end = start.checked_add(size).ok_or(WavError::InvalidChunk)?;

        if end > riff_end {
            return Err(WavError::InvalidChunk);
        }

        if id == b"fmt " {
            if size < 16 {
                return Err(WavError::InvalidFormat);
            }

            format = Some((
                u16_le(bytes, start)?,
                u16_le(bytes, start + 2)?,
                u32_le(bytes, start + 4)?,
                u16_le(bytes, start + 12)?,
                u16_le(bytes, start + 14)?,
            ));
        } else if id == b"data" && data.is_none() {
            data = Some((start, size));
        }

        cursor = end.checked_add(size & 1).ok_or(WavError::InvalidChunk)?;
    }

    let (encoding, channels, sample_rate, block_align, bits) =
        format.ok_or(WavError::MissingFormat)?;
    let (data_offset, data_len) = data.ok_or(WavError::MissingData)?;

    if encoding != 1 {
        return Err(WavError::UnsupportedFormat(encoding));
    }
    if !matches!(bits, 8 | 16 | 24 | 32) {
        return Err(WavError::UnsupportedBitDepth(bits));
    }
    if channels == 0 || sample_rate == 0 {
        return Err(WavError::InvalidFormat);
    }

    let expected_align = channels
        .checked_mul(bits / 8)
        .ok_or(WavError::InvalidFormat)?;

    if block_align != expected_align || block_align == 0 {
        return Err(WavError::InvalidFormat);
    }
    if data_len % block_align as usize != 0 {
        return Err(WavError::TruncatedData);
    }

    Ok(WavInfo {
        channels,
        sample_rate,
        bits_per_sample: bits,
        block_align,
        data_offset,
        data_len,
    })
}

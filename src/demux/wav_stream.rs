use crate::audio::decode_integer_pcm;

use std::{
    error::Error,
    fs::File,
    io::{self, Read, Seek, SeekFrom},
    path::Path,
};

#[derive(Debug, Clone, Copy)]
pub struct WavStreamInfo {
    pub channels: u16,
    pub sample_rate: u32,
    pub bits_per_sample: u16,
    pub block_align: u16,
    pub data_len: u64,
}

pub struct WavStreamReader {
    file: File,
    info: WavStreamInfo,
    data_offset: u64,
    bytes_read: u64,
    raw_buffer: Vec<u8>,
}

impl WavStreamReader {
    pub fn open(path: impl AsRef<Path>) -> Result<Self, Box<dyn Error>> {
        let mut file = File::open(path)?;
        let file_len = file.metadata()?.len();

        let mut header = [0u8; 12];
        file.read_exact(&mut header)?;

        if &header[0..4] != b"RIFF" || &header[8..12] != b"WAVE" {
            return Err(io::Error::other("Not a RIFF/WAVE file").into());
        }

        let riff_size = u32::from_le_bytes(header[4..8].try_into()?) as u64;
        let riff_end = riff_size + 8;

        if riff_end > file_len || riff_end < 12 {
            return Err(io::Error::other("Invalid WAV RIFF size").into());
        }

        let mut format: Option<(u16, u16, u32, u16)> = None;
        let mut data: Option<(u64, u64)> = None;
        let mut offset = 12u64;

        while offset + 8 <= riff_end {
            file.seek(SeekFrom::Start(offset))?;

            let mut chunk_header = [0u8; 8];
            file.read_exact(&mut chunk_header)?;

            let chunk_id = &chunk_header[..4];
            let chunk_size = u32::from_le_bytes(chunk_header[4..8].try_into()?) as u64;

            let chunk_data_start = offset + 8;
            let chunk_end = chunk_data_start
                .checked_add(chunk_size)
                .ok_or_else(|| io::Error::other("WAV chunk size overflow"))?;

            if chunk_end > riff_end || chunk_end > file_len {
                return Err(io::Error::other("WAV chunk exceeds file bounds").into());
            }

            if chunk_id == b"fmt " {
                if chunk_size < 16 {
                    return Err(io::Error::other("WAV fmt chunk is too small").into());
                }

                let mut fmt = [0u8; 16];
                file.read_exact(&mut fmt)?;

                let audio_format = u16::from_le_bytes([fmt[0], fmt[1]]);
                let channels = u16::from_le_bytes([fmt[2], fmt[3]]);
                let sample_rate = u32::from_le_bytes(fmt[4..8].try_into()?);
                let block_align = u16::from_le_bytes([fmt[12], fmt[13]]);
                let bits_per_sample = u16::from_le_bytes([fmt[14], fmt[15]]);

                if audio_format != 1 {
                    return Err(
                        io::Error::other("Only uncompressed integer PCM WAV is supported").into(),
                    );
                }

                if channels == 0 || sample_rate == 0 {
                    return Err(io::Error::other("Invalid WAV format").into());
                }

                if !matches!(bits_per_sample, 8 | 16 | 24 | 32) {
                    return Err(io::Error::other("Unsupported integer PCM bit depth").into());
                }

                let expected_align = channels as u32 * (bits_per_sample as u32 / 8);

                if block_align as u32 != expected_align {
                    return Err(io::Error::other("Invalid WAV block alignment").into());
                }

                format = Some((channels, bits_per_sample, sample_rate, block_align));
            } else if chunk_id == b"data" && data.is_none() {
                data = Some((chunk_data_start, chunk_size));
            }

            offset = chunk_end + (chunk_size & 1);

            if format.is_some() && data.is_some() {
                break;
            }
        }

        let (channels, bits_per_sample, sample_rate, block_align) =
            format.ok_or_else(|| io::Error::other("WAV fmt chunk not found"))?;

        let (data_offset, data_len) =
            data.ok_or_else(|| io::Error::other("WAV data chunk not found"))?;

        if data_len % block_align as u64 != 0 {
            return Err(io::Error::other("WAV data ends with an incomplete audio frame").into());
        }

        file.seek(SeekFrom::Start(data_offset))?;

        Ok(Self {
            file,
            info: WavStreamInfo {
                channels,
                sample_rate,
                bits_per_sample,
                block_align,
                data_len,
            },
            data_offset,
            bytes_read: 0,
            raw_buffer: Vec::new(),
        })
    }

    pub fn info(&self) -> &WavStreamInfo {
        &self.info
    }

    pub fn total_frames(&self) -> u64 {
        self.info.data_len / self.info.block_align as u64
    }

    pub fn seek_frame(&mut self, frame: u64) -> Result<(), Box<dyn Error>> {
        if frame > self.total_frames() {
            return Err(io::Error::other("Seek frame is beyond the end of the WAV").into());
        }

        let byte_offset = frame
            .checked_mul(self.info.block_align as u64)
            .and_then(|offset| self.data_offset.checked_add(offset))
            .ok_or_else(|| io::Error::other("Seek offset overflow"))?;

        self.file.seek(SeekFrom::Start(byte_offset))?;
        self.bytes_read = frame * self.info.block_align as u64;
        Ok(())
    }

    pub fn read_frames(&mut self, max_frames: usize) -> Result<Vec<f32>, Box<dyn Error>> {
        if max_frames == 0 {
            return Ok(Vec::new());
        }

        let remaining_bytes = self.info.data_len - self.bytes_read;
        let remaining_frames = remaining_bytes / self.info.block_align as u64;

        let frame_count = max_frames.min(remaining_frames.min(usize::MAX as u64) as usize);

        if frame_count == 0 {
            return Ok(Vec::new());
        }

        let byte_count = frame_count
            .checked_mul(self.info.block_align as usize)
            .ok_or_else(|| io::Error::other("PCM chunk size overflow"))?;

        self.raw_buffer.resize(byte_count, 0);
        self.file.read_exact(&mut self.raw_buffer)?;

        self.bytes_read += byte_count as u64;

        let decoded = decode_integer_pcm(&self.raw_buffer, self.info.bits_per_sample)
            .map_err(|e| io::Error::other(e.to_string()))?;

        Ok(decoded)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{fs, path::PathBuf};

    fn make_test_wav() -> Vec<u8> {
        let samples: [i16; 4] = [0, 32767, -32768, 16384];

        let mut wav = Vec::new();
        wav.extend_from_slice(b"RIFF");
        wav.extend_from_slice(&0u32.to_le_bytes());
        wav.extend_from_slice(b"WAVE");

        wav.extend_from_slice(b"fmt ");
        wav.extend_from_slice(&16u32.to_le_bytes());
        wav.extend_from_slice(&1u16.to_le_bytes());
        wav.extend_from_slice(&1u16.to_le_bytes());
        wav.extend_from_slice(&8000u32.to_le_bytes());
        wav.extend_from_slice(&16000u32.to_le_bytes());
        wav.extend_from_slice(&2u16.to_le_bytes());
        wav.extend_from_slice(&16u16.to_le_bytes());

        wav.extend_from_slice(b"data");
        wav.extend_from_slice(&(samples.len() as u32 * 2).to_le_bytes());

        for sample in samples {
            wav.extend_from_slice(&sample.to_le_bytes());
        }

        let riff_size = (wav.len() - 8) as u32;
        wav[4..8].copy_from_slice(&riff_size.to_le_bytes());
        wav
    }

    #[test]
    fn reads_pcm_in_multiple_chunks() {
        let path: PathBuf =
            std::env::temp_dir().join(format!("openwave-stream-test-{}.wav", std::process::id()));

        fs::write(&path, make_test_wav()).unwrap();

        let mut reader = WavStreamReader::open(&path).unwrap();

        assert_eq!(reader.info().channels, 1);
        assert_eq!(reader.info().sample_rate, 8000);
        assert_eq!(reader.total_frames(), 4);

        let first = reader.read_frames(2).unwrap();
        let second = reader.read_frames(2).unwrap();
        let end = reader.read_frames(2).unwrap();

        assert_eq!(first.len(), 2);
        assert_eq!(second.len(), 2);
        assert!(end.is_empty());

        let all_samples: Vec<f32> = first.into_iter().chain(second).collect();

        assert_eq!(all_samples.len(), 4);
        assert!(all_samples[1] > 0.99);
        assert!(all_samples[2] <= -0.99);

        reader.seek_frame(1).unwrap();
        assert_eq!(reader.read_frames(1).unwrap(), [all_samples[1]]);
        assert!(reader.seek_frame(reader.total_frames() + 1).is_err());

        let _ = fs::remove_file(&path);
    }
}

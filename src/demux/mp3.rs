use std::io::{self, Read, Seek, SeekFrom};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MpegVersion {
    Mpeg1,
    Mpeg2,
    Mpeg25,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Layer {
    LayerI = 1,
    LayerII = 2,
    LayerIII = 3,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChannelMode {
    Stereo,
    JointStereo,
    DualChannel,
    Mono,
}

#[derive(Debug, Clone)]
pub struct Mp3FrameHeader {
    pub version: MpegVersion,
    pub layer: Layer,
    pub bitrate: u32,
    pub sample_rate: u32,
    pub padding: bool,
    pub channel_mode: ChannelMode,
    pub mode_extension: u8,
    pub frame_size: usize,
}

impl Mp3FrameHeader {
    pub fn parse(header: u32) -> io::Result<Self> {
        let sync = ((header >> 21) & 0x7FF) as u16;
        if sync != 0x7FF {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("Invalid MP3 sync word: 0x{:X}", sync),
            ));
        }

        let version_bits = (header >> 19) & 0x3;
        let version = match version_bits {
            0 => MpegVersion::Mpeg25,
            2 => MpegVersion::Mpeg2,
            3 => MpegVersion::Mpeg1,
            _ => {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "Reserved MPEG version",
                ));
            }
        };

        let layer_bits = (header >> 17) & 0x3;
        let layer = match layer_bits {
            1 => Layer::LayerIII,
            2 => Layer::LayerII,
            3 => Layer::LayerI,
            _ => return Err(io::Error::new(io::ErrorKind::InvalidData, "Reserved layer")),
        };

        let bitrate_index = ((header >> 12) & 0xF) as usize;
        let bitrate = Self::get_bitrate(version, layer, bitrate_index)?;

        let sample_rate_index = ((header >> 10) & 0x3) as usize;
        let sample_rate = Self::get_sample_rate(version, sample_rate_index)?;

        let padding = ((header >> 9) & 0x1) == 1;

        let channel_mode_bits = (header >> 6) & 0x3;
        let channel_mode = match channel_mode_bits {
            0 => ChannelMode::Stereo,
            1 => ChannelMode::JointStereo,
            2 => ChannelMode::DualChannel,
            3 => ChannelMode::Mono,
            _ => unreachable!(),
        };

        let mode_extension = ((header >> 4) & 0x3) as u8;

        let frame_size = Self::calculate_frame_size(version, layer, bitrate, sample_rate, padding);

        Ok(Mp3FrameHeader {
            version,
            layer,
            bitrate,
            sample_rate,
            padding,
            channel_mode,
            mode_extension,
            frame_size,
        })
    }

    fn get_bitrate(version: MpegVersion, layer: Layer, index: usize) -> io::Result<u32> {
        if index == 0 || index == 15 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "Invalid bitrate index",
            ));
        }

        let bitrates = match (version, layer) {
            (MpegVersion::Mpeg1, Layer::LayerI) => [
                0, 32, 64, 96, 128, 160, 192, 224, 256, 288, 320, 352, 384, 416, 448, 0,
            ],
            (MpegVersion::Mpeg1, Layer::LayerII) => [
                0, 32, 48, 56, 64, 80, 96, 112, 128, 160, 192, 224, 256, 320, 384, 0,
            ],
            (MpegVersion::Mpeg1, Layer::LayerIII) => [
                0, 32, 40, 48, 56, 64, 80, 96, 112, 128, 160, 192, 224, 256, 320, 0,
            ],
            (MpegVersion::Mpeg2 | MpegVersion::Mpeg25, Layer::LayerI) => [
                0, 32, 48, 56, 64, 80, 96, 112, 128, 144, 160, 176, 192, 224, 256, 0,
            ],
            (MpegVersion::Mpeg2 | MpegVersion::Mpeg25, Layer::LayerII | Layer::LayerIII) => [
                0, 8, 16, 24, 32, 40, 48, 56, 64, 80, 96, 112, 128, 144, 160, 0,
            ],
        };

        Ok(bitrates[index] * 1000)
    }

    fn get_sample_rate(version: MpegVersion, index: usize) -> io::Result<u32> {
        if index == 3 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "Reserved sample rate",
            ));
        }

        let sample_rates = match version {
            MpegVersion::Mpeg1 => [44100, 48000, 32000, 0],
            MpegVersion::Mpeg2 => [22050, 24000, 16000, 0],
            MpegVersion::Mpeg25 => [11025, 12000, 8000, 0],
        };

        Ok(sample_rates[index])
    }

    fn calculate_frame_size(
        version: MpegVersion,
        layer: Layer,
        bitrate: u32,
        sample_rate: u32,
        padding: bool,
    ) -> usize {
        let samples_per_frame = match (version, layer) {
            (MpegVersion::Mpeg1, Layer::LayerI) => 384,
            (MpegVersion::Mpeg1, Layer::LayerII | Layer::LayerIII) => 1152,
            (MpegVersion::Mpeg2 | MpegVersion::Mpeg25, Layer::LayerI) => 384,
            (MpegVersion::Mpeg2 | MpegVersion::Mpeg25, Layer::LayerII) => 1152,
            (MpegVersion::Mpeg2 | MpegVersion::Mpeg25, Layer::LayerIII) => 576,
        };

        let slot_size = if layer == Layer::LayerI { 4 } else { 1 };
        let padding_size = if padding { slot_size } else { 0 };

        ((samples_per_frame as u64 * bitrate as u64) / (8 * sample_rate as u64)) as usize
            + padding_size
    }

    pub fn channels(&self) -> usize {
        match self.channel_mode {
            ChannelMode::Mono => 1,
            _ => 2,
        }
    }

    pub fn samples_per_frame(&self) -> usize {
        match (self.version, self.layer) {
            (MpegVersion::Mpeg1, Layer::LayerIII) => 1152,
            (MpegVersion::Mpeg2 | MpegVersion::Mpeg25, Layer::LayerIII) => 576,
            _ => 1152,
        }
    }
}

pub struct Mp3Reader<R: Read + Seek> {
    reader: R,
    first_frame_pos: u64,
}

impl<R: Read + Seek> Mp3Reader<R> {
    pub fn new(mut reader: R) -> io::Result<Self> {
        let first_frame_pos = Self::skip_id3v2(&mut reader)?;

        Ok(Mp3Reader {
            reader,
            first_frame_pos,
        })
    }

    fn skip_id3v2(reader: &mut R) -> io::Result<u64> {
        let mut header = [0u8; 10];
        reader.read_exact(&mut header)?;

        if &header[0..3] == b"ID3" {
            let size = ((header[6] as u32 & 0x7F) << 21)
                | ((header[7] as u32 & 0x7F) << 14)
                | ((header[8] as u32 & 0x7F) << 7)
                | (header[9] as u32 & 0x7F);

            let skip_pos = 10 + size as u64;
            reader.seek(SeekFrom::Start(skip_pos))?;
            Ok(skip_pos)
        } else {
            reader.seek(SeekFrom::Start(0))?;
            Ok(0)
        }
    }

    pub fn find_next_frame(&mut self) -> io::Result<Option<Mp3FrameHeader>> {
        let mut sync_buf = [0u8; 4];
        let mut found = false;

        while !found {
            if self.reader.read_exact(&mut sync_buf[0..1]).is_err() {
                return Ok(None);
            }

            if sync_buf[0] == 0xFF {
                if self.reader.read_exact(&mut sync_buf[1..2]).is_err() {
                    return Ok(None);
                }

                if (sync_buf[1] & 0xE0) == 0xE0 {
                    if self.reader.read_exact(&mut sync_buf[2..4]).is_err() {
                        return Ok(None);
                    }
                    found = true;
                }
            }
        }

        let header_u32 = u32::from_be_bytes(sync_buf);
        let frame_header = Mp3FrameHeader::parse(header_u32)?;

        Ok(Some(frame_header))
    }

    pub fn read_frame_data(&mut self, header: &Mp3FrameHeader) -> io::Result<Vec<u8>> {
        let data_size = header.frame_size.saturating_sub(4);
        let mut data = vec![0u8; data_size];
        self.reader.read_exact(&mut data)?;
        Ok(data)
    }

    pub fn seek_to_start(&mut self) -> io::Result<()> {
        self.reader.seek(SeekFrom::Start(self.first_frame_pos))?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_mpeg1_layer3_header() {
        let header = 0xFFFA_9000u32;
        let parsed = Mp3FrameHeader::parse(header).unwrap();

        assert_eq!(parsed.version, MpegVersion::Mpeg1);
        assert_eq!(parsed.layer, Layer::LayerIII);
    }

    #[test]
    fn calculates_correct_channels() {
        let header = 0xFFFA_9000u32;
        let parsed = Mp3FrameHeader::parse(header).unwrap();

        assert_eq!(parsed.channels(), 2);
    }

    #[test]
    fn calculates_samples_per_frame() {
        let header = 0xFFFA_9000u32;
        let parsed = Mp3FrameHeader::parse(header).unwrap();

        assert_eq!(parsed.samples_per_frame(), 1152);
    }

    #[test]
    fn rejects_invalid_sync() {
        let header = 0x0000_0000u32;
        assert!(Mp3FrameHeader::parse(header).is_err());
    }
}

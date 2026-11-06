use super::types::*;
use super::{Mp4Error, Mp4Result};
use std::io::{Read, Seek, SeekFrom};

pub fn read_u8<R: Read>(reader: &mut R) -> Mp4Result<u8> {
    let mut buf = [0u8; 1];
    reader.read_exact(&mut buf)?;
    Ok(buf[0])
}

pub fn read_u16<R: Read>(reader: &mut R) -> Mp4Result<u16> {
    let mut buf = [0u8; 2];
    reader.read_exact(&mut buf)?;
    Ok(u16::from_be_bytes(buf))
}

pub fn read_u24<R: Read>(reader: &mut R) -> Mp4Result<u32> {
    let mut buf = [0u8; 3];
    reader.read_exact(&mut buf)?;
    Ok(u32::from_be_bytes([0, buf[0], buf[1], buf[2]]))
}

pub fn read_u32<R: Read>(reader: &mut R) -> Mp4Result<u32> {
    let mut buf = [0u8; 4];
    reader.read_exact(&mut buf)?;
    Ok(u32::from_be_bytes(buf))
}

pub fn read_i32<R: Read>(reader: &mut R) -> Mp4Result<i32> {
    let mut buf = [0u8; 4];
    reader.read_exact(&mut buf)?;
    Ok(i32::from_be_bytes(buf))
}

pub fn read_u64<R: Read>(reader: &mut R) -> Mp4Result<u64> {
    let mut buf = [0u8; 8];
    reader.read_exact(&mut buf)?;
    Ok(u64::from_be_bytes(buf))
}

pub fn read_box_type<R: Read>(reader: &mut R) -> Mp4Result<BoxType> {
    let mut buf = [0u8; 4];
    reader.read_exact(&mut buf)?;
    Ok(BoxType::new(&buf))
}

pub fn skip_bytes<R: Read + Seek>(reader: &mut R, count: u64) -> Mp4Result<()> {
    let mut remaining = count;
    let mut buf = [0u8; 8192];

    while remaining > 0 {
        let to_read = remaining.min(buf.len() as u64) as usize;
        reader.read_exact(&mut buf[..to_read])?;
        remaining -= to_read as u64;
    }
    Ok(())
}

pub fn parse_box_header<R: Read + Seek>(reader: &mut R) -> Mp4Result<BoxHeader> {
    parse_box_header_ex(reader, false)
}
/// Parse a box header where a 32-bit size of zero means "extends to end of file".
///
/// This is only valid for top-level boxes.
pub fn parse_box_header_allow_eof<R: Read + Seek>(reader: &mut R) -> Mp4Result<BoxHeader> {
    parse_box_header_ex(reader, true)
}

fn parse_box_header_ex<R: Read + Seek>(
    reader: &mut R,
    allow_size_zero: bool,
) -> Mp4Result<BoxHeader> {
    let box_start = reader.stream_position()?;
    let size32 = read_u32(reader)?;
    let box_type = read_box_type(reader)?;

    let (size, header_size) = if size32 == 1 {
        let size64 = read_u64(reader)?;
        (size64, 16u64)
    } else if size32 == 0 {
        if !allow_size_zero {
            return Err(Mp4Error::UnsupportedFeature(
                "Box with size=0 is only valid at the top level",
            ));
        }

        let current_pos = reader.stream_position()?;
        let file_size = reader.seek(SeekFrom::End(0))?;
        reader.seek(SeekFrom::Start(current_pos))?;

        if file_size < box_start {
            return Err(Mp4Error::InvalidFormat("Box starts beyond end of file"));
        }

        (file_size - box_start, 8u64)
    } else {
        (size32 as u64, 8u64)
    };

    if size < header_size {
        return Err(Mp4Error::InvalidFormat("Box size smaller than header"));
    }

    Ok(BoxHeader {
        box_type,
        size,
        header_size,
    })
}

pub fn ensure_box_size(size: u64, minimum: u64, name: &'static str) -> Mp4Result<()> {
    if size < minimum {
        return Err(Mp4Error::InvalidFormat(name));
    }
    Ok(())
}

pub fn parse_mvhd<R: Read + Seek>(reader: &mut R, size: u64) -> Mp4Result<(u32, u64)> {
    let version = read_u8(reader)?;
    skip_bytes(reader, 3)?;

    let (timescale, duration) = match version {
        0 => {
            ensure_box_size(size, 100, "mvhd box too small")?;
            skip_bytes(reader, 8)?;
            let timescale = read_u32(reader)?;
            let duration = read_u32(reader)? as u64;
            (timescale, duration)
        }
        1 => {
            ensure_box_size(size, 112, "mvhd box too small")?;
            skip_bytes(reader, 16)?;
            let timescale = read_u32(reader)?;
            let duration = read_u64(reader)?;
            (timescale, duration)
        }
        _ => return Err(Mp4Error::UnsupportedFeature("Unsupported mvhd version")),
    };

    let consumed = if version == 1 { 32 } else { 20 };
    skip_bytes(reader, size - consumed)?;

    Ok((timescale, duration))
}

pub fn parse_tkhd<R: Read + Seek>(
    reader: &mut R,
    size: u64,
) -> Mp4Result<(u32, u64, Option<u32>, Option<u32>)> {
    let version = read_u8(reader)?;
    skip_bytes(reader, 3)?;

    let (track_id, duration, consumed) = match version {
        0 => {
            ensure_box_size(size, 84, "tkhd box too small")?;
            skip_bytes(reader, 8)?;
            let track_id = read_u32(reader)?;
            skip_bytes(reader, 4)?;
            let duration = read_u32(reader)? as u64;
            (track_id, duration, 24)
        }
        1 => {
            ensure_box_size(size, 96, "tkhd box too small")?;
            skip_bytes(reader, 16)?;
            let track_id = read_u32(reader)?;
            skip_bytes(reader, 4)?;
            let duration = read_u64(reader)?;
            (track_id, duration, 36)
        }
        _ => return Err(Mp4Error::UnsupportedFeature("Unsupported tkhd version")),
    };

    // reserved[2], layer, alternate_group, volume, reserved, matrix[9]
    skip_bytes(reader, 8 + 2 + 2 + 2 + 2 + 36)?;

    let width = read_u32(reader)?;
    let height = read_u32(reader)?;

    let width = (width != 0).then_some(width >> 16);
    let height = (height != 0).then_some(height >> 16);

    let consumed = consumed + 52 + 8;
    if size > consumed {
        skip_bytes(reader, size - consumed)?;
    }

    Ok((track_id, duration, width, height))
}

pub fn parse_mdhd<R: Read + Seek>(reader: &mut R, size: u64) -> Mp4Result<(u32, u64)> {
    let version = read_u8(reader)?;
    skip_bytes(reader, 3)?;

    let (timescale, duration, consumed) = match version {
        0 => {
            ensure_box_size(size, 24, "mdhd box too small")?;
            skip_bytes(reader, 8)?;
            let timescale = read_u32(reader)?;
            let duration = read_u32(reader)? as u64;
            (timescale, duration, 20)
        }
        1 => {
            ensure_box_size(size, 36, "mdhd box too small")?;
            skip_bytes(reader, 16)?;
            let timescale = read_u32(reader)?;
            let duration = read_u64(reader)?;
            (timescale, duration, 32)
        }
        _ => return Err(Mp4Error::UnsupportedFeature("Unsupported mdhd version")),
    };

    skip_bytes(reader, 4)?;
    let consumed = consumed + 4;
    if size > consumed {
        skip_bytes(reader, size - consumed)?;
    }

    Ok((timescale, duration))
}

pub fn parse_hdlr<R: Read + Seek>(reader: &mut R, size: u64) -> Mp4Result<TrackType> {
    ensure_box_size(size, 12, "hdlr box too small")?;

    skip_bytes(reader, 4)?;
    skip_bytes(reader, 4)?;

    let mut handler_type = [0u8; 4];
    reader.read_exact(&mut handler_type)?;

    let track_type = match &handler_type {
        b"vide" => TrackType::Video,
        b"soun" => TrackType::Audio,
        b"subt" | b"text" => TrackType::Subtitle,
        _ => TrackType::Unknown,
    };

    let remaining = size - 12;
    if remaining > 0 {
        skip_bytes(reader, remaining)?;
    }

    Ok(track_type)
}

pub fn parse_esds<R: Read + Seek>(reader: &mut R, size: u64) -> Mp4Result<Vec<u8>> {
    ensure_box_size(size, 4, "esds box too small")?;
    skip_bytes(reader, 4)?;

    let mut remaining = size - 4;
    let mut audio_specific_config = Vec::new();

    while remaining > 0 && audio_specific_config.is_empty() {
        if remaining < 2 {
            break;
        }

        let tag = read_u8(reader)?;
        let desc_len = read_descriptor_length(reader, &mut remaining)? as u64;
        remaining -= 1;

        if desc_len > remaining {
            return Err(Mp4Error::InvalidFormat(
                "esds descriptor length exceeds box size",
            ));
        }

        match tag {
            0x03 => {
                if desc_len < 3 {
                    skip_bytes(reader, desc_len)?;
                    remaining -= desc_len;
                    continue;
                }
                skip_bytes(reader, 3)?;
                remaining -= 3;
            }
            0x04 => {
                if desc_len < 13 {
                    skip_bytes(reader, desc_len)?;
                    remaining -= desc_len;
                    continue;
                }
                skip_bytes(reader, 13)?;
                remaining -= 13;
            }
            0x05 => {
                audio_specific_config = vec![0u8; desc_len as usize];
                reader.read_exact(&mut audio_specific_config)?;
                remaining -= desc_len;
                break;
            }
            _ => {
                skip_bytes(reader, desc_len)?;
                remaining -= desc_len;
            }
        }
    }

    if remaining > 0 {
        skip_bytes(reader, remaining)?;
    }

    if audio_specific_config.is_empty() {
        return Err(Mp4Error::InvalidFormat(
            "No AudioSpecificConfig found in esds",
        ));
    }

    Ok(audio_specific_config)
}

fn read_descriptor_length<R: Read>(reader: &mut R, remaining: &mut u64) -> Mp4Result<u32> {
    let mut length = 0u32;
    for _ in 0..4 {
        if *remaining == 0 {
            break;
        }
        let byte = read_u8(reader)?;
        *remaining -= 1;
        length = (length << 7) | ((byte & 0x7F) as u32);
        if (byte & 0x80) == 0 {
            break;
        }
    }
    Ok(length)
}

pub fn parse_mp4a<R: Read + Seek>(reader: &mut R, size: u64) -> Mp4Result<(u32, u16, Vec<u8>)> {
    ensure_box_size(size, 28, "mp4a box too small")?;
    skip_bytes(reader, 6)?;
    skip_bytes(reader, 2)?;

    let version = read_u16(reader)?;
    skip_bytes(reader, 6)?;

    let mut channels = read_u16(reader)?;
    let _sample_size = read_u16(reader)?;
    skip_bytes(reader, 4)?; // compression_id and packet_size
    let mut sample_rate = read_u32(reader)? >> 16;
    let header_size = match version {
        0 => 28,
        1 => {
            ensure_box_size(size, 44, "mp4a version 1 box too small")?;
            skip_bytes(reader, 16)?;
            44
        }
        2 => {
            ensure_box_size(size, 64, "mp4a version 2 box too small")?;
            skip_bytes(reader, 4)?; // sizeOfStructOnly
            let rate = f64::from_bits(read_u64(reader)?);
            let channel_count = read_u32(reader)?;
            skip_bytes(reader, 4)?; // always 0x7f000000
            skip_bytes(reader, 4)?; // constBitsPerChannel
            skip_bytes(reader, 4)?; // formatSpecificFlags
            skip_bytes(reader, 4)?; // constBytesPerAudioPacket
            skip_bytes(reader, 4)?; // constLPCMFramesPerAudioPacket

            if rate.is_finite() && rate > 0.0 && rate <= u32::MAX as f64 {
                sample_rate = rate.round() as u32;
            }
            if channel_count > 0 {
                channels = u16::try_from(channel_count).unwrap_or(u16::MAX);
            }

            64
        }
        _ => return Err(Mp4Error::UnsupportedFeature("Unsupported mp4a version")),
    };

    let mut remaining = size - header_size;
    let mut audio_specific_config = Vec::new();

    while remaining >= 8 {
        let box_start = reader.stream_position()?;
        let box_header = parse_box_header(reader)?;

        let box_end = box_start
            .checked_add(box_header.size)
            .ok_or(Mp4Error::InvalidFormat("mp4a child box size overflow"))?;
        let parent_end = box_start
            .checked_add(remaining)
            .ok_or(Mp4Error::InvalidFormat("mp4a box size overflow"))?;
        if box_end > parent_end {
            return Err(Mp4Error::InvalidFormat(
                "mp4a child box extends beyond parent",
            ));
        }

        let content_size = box_header.content_size();
        remaining -= box_header.size;

        if box_header.box_type == BoxType::ESDS {
            audio_specific_config = parse_esds(reader, content_size)?;
            break;
        } else {
            skip_bytes(reader, content_size)?;
        }
    }

    if remaining > 0 {
        skip_bytes(reader, remaining)?;
    }

    Ok((sample_rate, channels, audio_specific_config))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    #[test]
    fn reads_box_header() {
        let data = vec![0x00, 0x00, 0x00, 0x20, b'f', b't', b'y', b'p'];

        let mut cursor = Cursor::new(data);
        let header = parse_box_header(&mut cursor).unwrap();

        assert_eq!(header.box_type, BoxType::FTYP);
        assert_eq!(header.size, 32);
        assert_eq!(header.header_size, 8);
    }

    #[test]
    fn reads_u32_big_endian() {
        let data = vec![0x12, 0x34, 0x56, 0x78];
        let mut cursor = Cursor::new(data);
        assert_eq!(read_u32(&mut cursor).unwrap(), 0x12345678);
    }

    #[test]
    fn parses_minimal_mp4a_without_underflow() {
        let mut data = vec![0u8; 32];
        data[6..8].copy_from_slice(&1u16.to_be_bytes());
        data[16..18].copy_from_slice(&2u16.to_be_bytes());
        data[18..20].copy_from_slice(&16u16.to_be_bytes());
        data[24..28].copy_from_slice(&(44100u32 << 16).to_be_bytes());

        let mut cursor = Cursor::new(data);
        let (rate, channels, config) = parse_mp4a(&mut cursor, 32).unwrap();

        assert_eq!(rate, 44100);
        assert_eq!(channels, 2);
        assert!(config.is_empty());
    }

    #[test]
    fn rejects_invalid_box_size() {
        let data = vec![0x00, 0x00, 0x00, 0x04, b'f', b't', b'y', b'p'];

        let mut cursor = Cursor::new(data);
        assert!(parse_box_header(&mut cursor).is_err());
    }

    #[test]
    fn reads_extended_box_header() {
        let mut data = vec![0x00, 0x00, 0x00, 0x01, b'f', b't', b'y', b'p'];
        data.extend_from_slice(&24u64.to_be_bytes());

        let mut cursor = Cursor::new(data);
        let header = parse_box_header(&mut cursor).unwrap();

        assert_eq!(header.box_type, BoxType::FTYP);
        assert_eq!(header.size, 24);
        assert_eq!(header.header_size, 16);
    }

    #[test]
    fn resolves_zero_size_box_to_end_of_file() {
        let mut data = vec![0x00, 0x00, 0x00, 0x00, b'm', b'd', b'a', b't'];
        data.extend_from_slice(&[1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12]);

        let mut cursor = Cursor::new(data);
        let header = parse_box_header_allow_eof(&mut cursor).unwrap();

        assert_eq!(header.box_type, BoxType::MDAT);
        assert_eq!(header.size, 20);
        assert_eq!(header.content_size(), 12);
    }

    #[test]
    fn parses_tkhd_dimensions_at_correct_offsets() {
        let mut data = vec![0u8; 84];
        data[12..16].copy_from_slice(&7u32.to_be_bytes());
        data[20..24].copy_from_slice(&42u32.to_be_bytes());
        data[76..80].copy_from_slice(&(1920u32 << 16).to_be_bytes());
        data[80..84].copy_from_slice(&(1080u32 << 16).to_be_bytes());

        let mut cursor = Cursor::new(data);
        let (track_id, duration, width, height) = parse_tkhd(&mut cursor, 84).unwrap();

        assert_eq!(track_id, 7);
        assert_eq!(duration, 42);
        assert_eq!(width, Some(1920));
        assert_eq!(height, Some(1080));
    }

    #[test]
    fn rejects_esds_descriptor_beyond_box() {
        let mut data = vec![0u8; 6];
        data[4] = 0x05;
        data[5] = 0x7F;

        let mut cursor = Cursor::new(data);
        assert!(parse_esds(&mut cursor, 6).is_err());
    }

    #[test]
    fn parses_mp4a_version_1_child_boxes_after_extra_fields() {
        let mut data = vec![0u8; 44];
        data[8..10].copy_from_slice(&1u16.to_be_bytes());
        data[16..18].copy_from_slice(&2u16.to_be_bytes());
        data[18..20].copy_from_slice(&16u16.to_be_bytes());
        data[24..28].copy_from_slice(&(44100u32 << 16).to_be_bytes());

        let mut esds_payload = vec![0u8; 4];
        esds_payload.extend_from_slice(&[0x05, 0x02, 0x12, 0x10]);
        data.extend(make_box(b"esds", &esds_payload));

        let mut cursor = Cursor::new(data);
        let (rate, channels, config) = parse_mp4a(&mut cursor, 44 + 16).unwrap();

        assert_eq!(rate, 44100);
        assert_eq!(channels, 2);
        assert_eq!(config, vec![0x12, 0x10]);
    }

    #[test]
    fn parses_mp4a_version_2_sample_rate_and_channels() {
        let mut data = vec![0u8; 64];
        data[8..10].copy_from_slice(&2u16.to_be_bytes());
        data[16..18].copy_from_slice(&1u16.to_be_bytes());
        data[24..28].copy_from_slice(&(1u32 << 16).to_be_bytes());
        data[32..40].copy_from_slice(&48000.0f64.to_bits().to_be_bytes());
        data[40..44].copy_from_slice(&6u32.to_be_bytes());
        data[44..48].copy_from_slice(&0x7f00_0000u32.to_be_bytes());

        let mut esds_payload = vec![0u8; 4];
        esds_payload.extend_from_slice(&[0x05, 0x02, 0x12, 0x10]);
        data.extend(make_box(b"esds", &esds_payload));

        let mut cursor = Cursor::new(data);
        let (rate, channels, config) = parse_mp4a(&mut cursor, 64 + 16).unwrap();

        assert_eq!(rate, 48000);
        assert_eq!(channels, 6);
        assert_eq!(config, vec![0x12, 0x10]);
    }

    #[test]
    fn rejects_unknown_mp4a_version() {
        let mut data = vec![0u8; 28];
        data[8..10].copy_from_slice(&3u16.to_be_bytes());

        let mut cursor = Cursor::new(data);
        assert!(matches!(
            parse_mp4a(&mut cursor, 28),
            Err(Mp4Error::UnsupportedFeature(_))
        ));
    }

    #[test]
    fn parses_mp4a_audio_specific_config() {
        let mut data = vec![0u8; 28];
        data[16..18].copy_from_slice(&2u16.to_be_bytes());
        data[18..20].copy_from_slice(&16u16.to_be_bytes());
        data[24..28].copy_from_slice(&(48000u32 << 16).to_be_bytes());

        let mut esds_payload = vec![0u8; 4];
        esds_payload.extend_from_slice(&[0x05, 0x02, 0x12, 0x10]);
        data.extend(make_box(b"esds", &esds_payload));

        let mut cursor = Cursor::new(data);
        let (rate, channels, config) = parse_mp4a(&mut cursor, 44).unwrap();

        assert_eq!(rate, 48000);
        assert_eq!(channels, 2);
        assert_eq!(config, vec![0x12, 0x10]);
    }

    fn make_box(box_type: &[u8; 4], payload: &[u8]) -> Vec<u8> {
        let size = 8 + payload.len() as u32;
        let mut out = Vec::with_capacity(size as usize);
        out.extend_from_slice(&size.to_be_bytes());
        out.extend_from_slice(box_type);
        out.extend_from_slice(payload);
        out
    }
}

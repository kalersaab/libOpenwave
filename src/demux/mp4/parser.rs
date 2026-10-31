use super::types::*;
use super::{Mp4Error, Mp4Result};
use std::io::Read;

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

pub fn skip_bytes<R: Read>(reader: &mut R, count: u64) -> Mp4Result<()> {
    let mut remaining = count;
    let mut buf = vec![0u8; 8192.min(count as usize)];

    while remaining > 0 {
        let to_read = remaining.min(buf.len() as u64) as usize;
        reader.read_exact(&mut buf[..to_read])?;
        remaining -= to_read as u64;
    }
    Ok(())
}

pub fn parse_box_header<R: Read>(reader: &mut R) -> Mp4Result<BoxHeader> {
    let size32 = read_u32(reader)?;
    let box_type = read_box_type(reader)?;

    let (size, header_size) = if size32 == 1 {
        let size64 = read_u64(reader)?;
        (size64, 16)
    } else if size32 == 0 {
        return Err(Mp4Error::UnsupportedFeature("Box extends to end of file"));
    } else {
        (size32 as u64, 8)
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

pub fn parse_ftyp<R: Read>(reader: &mut R, size: u64) -> Mp4Result<()> {
    let _major_brand = read_u32(reader)?;
    let _minor_version = read_u32(reader)?;

    let mut remaining = size - 8 - 8;
    while remaining >= 4 {
        let _compatible_brand = read_u32(reader)?;
        remaining -= 4;
    }

    if remaining > 0 {
        skip_bytes(reader, remaining)?;
    }

    Ok(())
}

pub fn parse_mvhd<R: Read>(reader: &mut R) -> Mp4Result<(u32, u64)> {
    let version = read_u8(reader)?;
    skip_bytes(reader, 3)?;

    let (timescale, duration) = if version == 1 {
        skip_bytes(reader, 16)?;
        let timescale = read_u32(reader)?;
        let duration = read_u64(reader)?;
        (timescale, duration)
    } else {
        skip_bytes(reader, 8)?;
        let timescale = read_u32(reader)?;
        let duration = read_u32(reader)? as u64;
        (timescale, duration)
    };

    Ok((timescale, duration))
}

pub fn parse_tkhd<R: Read>(reader: &mut R) -> Mp4Result<(u32, u64, Option<u32>, Option<u32>)> {
    let version = read_u8(reader)?;
    skip_bytes(reader, 3)?;

    let (track_id, duration) = if version == 1 {
        skip_bytes(reader, 16)?;
        let track_id = read_u32(reader)?;
        skip_bytes(reader, 4)?;
        let duration = read_u64(reader)?;
        (track_id, duration)
    } else {
        skip_bytes(reader, 8)?;
        let track_id = read_u32(reader)?;
        skip_bytes(reader, 4)?;
        let duration = read_u32(reader)? as u64;
        (track_id, duration)
    };

    skip_bytes(reader, 8)?;
    skip_bytes(reader, 2)?;
    skip_bytes(reader, 2)?;
    skip_bytes(reader, 36)?;

    let width = read_u32(reader)?;
    let height = read_u32(reader)?;

    let width = if width > 0 { Some(width >> 16) } else { None };
    let height = if height > 0 { Some(height >> 16) } else { None };

    Ok((track_id, duration, width, height))
}

pub fn parse_mdhd<R: Read>(reader: &mut R) -> Mp4Result<(u32, u64)> {
    let version = read_u8(reader)?;
    skip_bytes(reader, 3)?;

    let (timescale, duration) = if version == 1 {
        skip_bytes(reader, 16)?;
        let timescale = read_u32(reader)?;
        let duration = read_u64(reader)?;
        (timescale, duration)
    } else {
        skip_bytes(reader, 8)?;
        let timescale = read_u32(reader)?;
        let duration = read_u32(reader)? as u64;
        (timescale, duration)
    };

    Ok((timescale, duration))
}

pub fn parse_hdlr<R: Read>(reader: &mut R, size: u64) -> Mp4Result<TrackType> {
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

    let remaining = size.saturating_sub(8 + 8);
    if remaining > 0 {
        skip_bytes(reader, remaining)?;
    }

    Ok(track_type)
}

pub fn parse_stts<R: Read>(reader: &mut R) -> Mp4Result<Vec<TimeToSample>> {
    skip_bytes(reader, 4)?;

    let entry_count = read_u32(reader)?;
    let mut entries = Vec::with_capacity(entry_count.min(10000) as usize);

    for _ in 0..entry_count {
        let sample_count = read_u32(reader)?;
        let sample_delta = read_u32(reader)?;
        entries.push(TimeToSample {
            sample_count,
            sample_delta,
        });
    }

    Ok(entries)
}

pub fn parse_stsc<R: Read>(reader: &mut R) -> Mp4Result<Vec<SampleToChunk>> {
    skip_bytes(reader, 4)?;

    let entry_count = read_u32(reader)?;
    let mut entries = Vec::with_capacity(entry_count.min(10000) as usize);

    for _ in 0..entry_count {
        let first_chunk = read_u32(reader)?;
        let samples_per_chunk = read_u32(reader)?;
        let sample_description_index = read_u32(reader)?;

        entries.push(SampleToChunk {
            first_chunk,
            samples_per_chunk,
            sample_description_index,
        });
    }

    Ok(entries)
}

pub fn parse_stsz<R: Read>(reader: &mut R) -> Mp4Result<Vec<u32>> {
    skip_bytes(reader, 4)?;

    let sample_size = read_u32(reader)?;
    let sample_count = read_u32(reader)?;

    if sample_size != 0 {
        Ok(vec![sample_size; sample_count.min(1000000) as usize])
    } else {
        let mut sizes = Vec::with_capacity(sample_count.min(1000000) as usize);
        for _ in 0..sample_count {
            sizes.push(read_u32(reader)?);
        }
        Ok(sizes)
    }
}

pub fn parse_stco<R: Read>(reader: &mut R) -> Mp4Result<Vec<u64>> {
    skip_bytes(reader, 4)?;

    let entry_count = read_u32(reader)?;
    let mut offsets = Vec::with_capacity(entry_count.min(1000000) as usize);

    for _ in 0..entry_count {
        offsets.push(read_u32(reader)? as u64);
    }

    Ok(offsets)
}

pub fn parse_co64<R: Read>(reader: &mut R) -> Mp4Result<Vec<u64>> {
    skip_bytes(reader, 4)?;

    let entry_count = read_u32(reader)?;
    let mut offsets = Vec::with_capacity(entry_count.min(1000000) as usize);

    for _ in 0..entry_count {
        offsets.push(read_u64(reader)?);
    }

    Ok(offsets)
}

pub fn parse_stss<R: Read>(reader: &mut R) -> Mp4Result<Vec<u32>> {
    skip_bytes(reader, 4)?;

    let entry_count = read_u32(reader)?;
    let mut sync_samples = Vec::with_capacity(entry_count.min(100000) as usize);

    for _ in 0..entry_count {
        sync_samples.push(read_u32(reader)?);
    }

    Ok(sync_samples)
}

pub fn parse_esds<R: Read>(reader: &mut R, size: u64) -> Mp4Result<Vec<u8>> {
    // ESDS (Elementary Stream Descriptor) contains AAC configuration
    // Format: version/flags (4), ES_DescrTag, ...
    skip_bytes(reader, 4)?; // version + flags

    let mut remaining = size - 4;
    let mut audio_specific_config = Vec::new();

    // Parse ES descriptor tags (simplified)
    while remaining > 0 && audio_specific_config.is_empty() {
        if remaining < 2 {
            break;
        }

        let tag = read_u8(reader)?;
        let desc_len = read_descriptor_length(reader, &mut remaining)?;
        remaining -= 1; // tag byte

        match tag {
            0x03 => {
                // ES_DescrTag
                if desc_len < 3 {
                    skip_bytes(reader, desc_len as u64)?;
                    remaining -= desc_len as u64;
                    continue;
                }
                skip_bytes(reader, 3)?; // ES_ID + flags
                remaining -= desc_len as u64;
            }
            0x04 => {
                // DecoderConfigDescrTag
                if desc_len < 13 {
                    skip_bytes(reader, desc_len as u64)?;
                    remaining -= desc_len as u64;
                    continue;
                }
                skip_bytes(reader, 13)?; // objectTypeIndication, streamType, bufferSize, etc.
                remaining -= desc_len as u64;
            }
            0x05 => {
                // DecSpecificInfoTag - this contains AudioSpecificConfig!
                audio_specific_config = vec![0u8; desc_len as usize];
                reader.read_exact(&mut audio_specific_config)?;
                remaining -= desc_len as u64;
                break;
            }
            _ => {
                // Unknown tag, skip
                if desc_len > 0 && desc_len < remaining as u32 {
                    skip_bytes(reader, desc_len as u64)?;
                    remaining -= desc_len as u64;
                } else {
                    break;
                }
            }
        }
    }

    if remaining > 0 {
        skip_bytes(reader, remaining)?;
    }

    if audio_specific_config.is_empty() {
        return Err(Mp4Error::InvalidFormat("No AudioSpecificConfig found in esds"));
    }

    Ok(audio_specific_config)
}

fn read_descriptor_length<R: Read>(reader: &mut R, remaining: &mut u64) -> Mp4Result<u32> {
    // MP4 descriptor length encoding (variable length)
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

pub fn parse_mp4a<R: Read>(reader: &mut R, size: u64) -> Mp4Result<(u32, u16, Vec<u8>)> {
    // mp4a box (AudioSampleEntry)
    skip_bytes(reader, 6)?; // reserved
    skip_bytes(reader, 2)?; // data_reference_index

    skip_bytes(reader, 8)?; // reserved (audio specific)

    let channels = read_u16(reader)?;
    let _sample_size = read_u16(reader)?;

    skip_bytes(reader, 4)?; // pre_defined + reserved

    let sample_rate = read_u32(reader)?;
    let sample_rate = sample_rate >> 16; // Fixed point 16.16

    let mut remaining = size - 8 - 6 - 2 - 8 - 2 - 2 - 4 - 4;
    let mut audio_specific_config = Vec::new();

    // Look for esds box
    while remaining >= 8 {
        let box_header = parse_box_header(reader)?;
        remaining -= box_header.header_size;

        let content_size = box_header.size - box_header.header_size;

        if box_header.box_type == BoxType::ESDS {
            audio_specific_config = parse_esds(reader, content_size)?;
            remaining -= content_size;
            break;
        } else {
            // Skip unknown box
            skip_bytes(reader, content_size)?;
            remaining -= content_size;
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
    fn rejects_invalid_box_size() {
        let data = vec![0x00, 0x00, 0x00, 0x04, b'f', b't', b'y', b'p'];

        let mut cursor = Cursor::new(data);
        assert!(parse_box_header(&mut cursor).is_err());
    }

}

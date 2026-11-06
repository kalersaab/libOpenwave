use super::super::parser::*;
use super::super::types::*;
use super::super::{Mp4Error, Mp4Result};
use std::io::{Read, Seek};

pub fn parse_ctts<R: Read + Seek>(reader: &mut R, size: u64) -> Mp4Result<Vec<CompositionOffset>> {
    ensure_box_size(size, 8, "ctts box too small")?;

    let version = read_u8(reader)?;
    skip_bytes(reader, 3)?;

    let entry_count = read_u32(reader)?;
    if (entry_count as u64) * 8 > size - 8 {
        return Err(Mp4Error::InvalidFormat("ctts entry count exceeds box size"));
    }

    let mut entries = Vec::with_capacity(entry_count as usize);

    for _ in 0..entry_count {
        let sample_count = read_u32(reader)?;
        let sample_offset = match version {
            0 => read_u32(reader)? as i64,
            1 => read_i32(reader)? as i64,
            _ => return Err(Mp4Error::UnsupportedFeature("Unsupported ctts version")),
        };

        entries.push(CompositionOffset {
            sample_count,
            sample_offset,
        });
    }

    Ok(entries)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    #[test]
    fn parses_ctts_version_0_unsigned_offsets() {
        let mut data = vec![0u8; 16];
        data[4..8].copy_from_slice(&1u32.to_be_bytes());
        data[8..12].copy_from_slice(&2u32.to_be_bytes());
        data[12..16].copy_from_slice(&u32::MAX.to_be_bytes());

        let mut cursor = Cursor::new(data);
        let entries = parse_ctts(&mut cursor, 16).unwrap();

        assert_eq!(
            entries,
            vec![CompositionOffset {
                sample_count: 2,
                sample_offset: u32::MAX as i64,
            }]
        );
    }

    #[test]
    fn parses_ctts_version_1_signed_offsets() {
        let mut data = vec![0u8; 16];
        data[0] = 1;
        data[4..8].copy_from_slice(&1u32.to_be_bytes());
        data[8..12].copy_from_slice(&1u32.to_be_bytes());
        data[12..16].copy_from_slice(&(-40i32).to_be_bytes());

        let mut cursor = Cursor::new(data);
        let entries = parse_ctts(&mut cursor, 16).unwrap();

        assert_eq!(
            entries,
            vec![CompositionOffset {
                sample_count: 1,
                sample_offset: -40,
            }]
        );
    }

    #[test]
    fn rejects_ctts_entry_count_beyond_box() {
        let mut data = vec![0u8; 8];
        data[4..8].copy_from_slice(&1u32.to_be_bytes());

        let mut cursor = Cursor::new(data);
        assert!(parse_ctts(&mut cursor, 8).is_err());
    }
}

use super::super::parser::*;
use super::super::types::*;
use super::super::{Mp4Error, Mp4Result};
use std::io::{Read, Seek};

pub fn parse_stts<R: Read + Seek>(reader: &mut R, size: u64) -> Mp4Result<Vec<TimeToSample>> {
    ensure_box_size(size, 8, "stts box too small")?;
    skip_bytes(reader, 4)?;

    let entry_count = read_u32(reader)?;
    if (entry_count as u64) * 8 > size - 8 {
        return Err(Mp4Error::InvalidFormat("stts entry count exceeds box size"));
    }

    let mut entries = Vec::with_capacity(entry_count as usize);

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

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    #[test]
    fn parses_stts_entries() {
        let mut data = vec![0u8; 20];
        data[4..8].copy_from_slice(&1u32.to_be_bytes());
        data[8..12].copy_from_slice(&3u32.to_be_bytes());
        data[12..16].copy_from_slice(&10u32.to_be_bytes());

        let mut cursor = Cursor::new(data);
        let entries = parse_stts(&mut cursor, 20).unwrap();

        assert_eq!(
            entries,
            vec![TimeToSample {
                sample_count: 3,
                sample_delta: 10,
            }]
        );
    }

    #[test]
    fn rejects_stts_entry_count_beyond_box() {
        let mut data = vec![0u8; 8];
        data[4..8].copy_from_slice(&1u32.to_be_bytes());

        let mut cursor = Cursor::new(data);
        assert!(parse_stts(&mut cursor, 8).is_err());
    }
}

use super::super::parser::*;
use super::super::types::*;
use super::super::{Mp4Error, Mp4Result};
use std::io::{Read, Seek};

pub fn parse_stsc<R: Read + Seek>(reader: &mut R, size: u64) -> Mp4Result<Vec<SampleToChunk>> {
    ensure_box_size(size, 8, "stsc box too small")?;
    skip_bytes(reader, 4)?;

    let entry_count = read_u32(reader)?;
    if (entry_count as u64) * 12 > size - 8 {
        return Err(Mp4Error::InvalidFormat("stsc entry count exceeds box size"));
    }

    let mut entries = Vec::with_capacity(entry_count as usize);
    let mut previous_first_chunk = 0u32;

    for _ in 0..entry_count {
        let first_chunk = read_u32(reader)?;
        let samples_per_chunk = read_u32(reader)?;
        let sample_description_index = read_u32(reader)?;

        if first_chunk <= previous_first_chunk || samples_per_chunk == 0 {
            return Err(Mp4Error::InvalidFormat("invalid stsc entry"));
        }
        if entries.is_empty() && first_chunk != 1 {
            return Err(Mp4Error::InvalidFormat(
                "first stsc entry must start at chunk 1",
            ));
        }
        previous_first_chunk = first_chunk;

        entries.push(SampleToChunk {
            first_chunk,
            samples_per_chunk,
            sample_description_index,
        });
    }

    Ok(entries)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    #[test]
    fn parses_stsc_entries() {
        let mut data = vec![0u8; 20];
        data[4..8].copy_from_slice(&1u32.to_be_bytes());
        data[8..12].copy_from_slice(&1u32.to_be_bytes());
        data[12..16].copy_from_slice(&2u32.to_be_bytes());
        data[16..20].copy_from_slice(&1u32.to_be_bytes());

        let mut cursor = Cursor::new(data);
        let entries = parse_stsc(&mut cursor, 20).unwrap();

        assert_eq!(
            entries,
            vec![SampleToChunk {
                first_chunk: 1,
                samples_per_chunk: 2,
                sample_description_index: 1,
            }]
        );
    }

    #[test]
    fn rejects_stsc_that_does_not_start_at_chunk_one() {
        let mut data = vec![0u8; 20];
        data[4..8].copy_from_slice(&1u32.to_be_bytes());
        data[8..12].copy_from_slice(&2u32.to_be_bytes());
        data[12..16].copy_from_slice(&1u32.to_be_bytes());

        let mut cursor = Cursor::new(data);
        assert!(parse_stsc(&mut cursor, 20).is_err());
    }
}

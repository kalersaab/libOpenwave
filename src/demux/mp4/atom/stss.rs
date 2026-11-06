use super::super::parser::*;
use super::super::{Mp4Error, Mp4Result};
use std::io::{Read, Seek};

pub fn parse_stss<R: Read + Seek>(reader: &mut R, size: u64) -> Mp4Result<Vec<u32>> {
    ensure_box_size(size, 8, "stss box too small")?;
    skip_bytes(reader, 4)?;

    let entry_count = read_u32(reader)?;
    if (entry_count as u64) * 4 > size - 8 {
        return Err(Mp4Error::InvalidFormat("stss entry count exceeds box size"));
    }

    let mut sync_samples = Vec::with_capacity(entry_count as usize);
    let mut previous = 0u32;

    for _ in 0..entry_count {
        let sample_number = read_u32(reader)?;
        if sample_number == 0 || sample_number <= previous {
            return Err(Mp4Error::InvalidFormat("invalid stss entry"));
        }
        previous = sample_number;
        sync_samples.push(sample_number);
    }

    Ok(sync_samples)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    #[test]
    fn parses_stss_sync_samples() {
        let mut data = vec![0u8; 16];
        data[4..8].copy_from_slice(&1u32.to_be_bytes());
        data[8..12].copy_from_slice(&1u32.to_be_bytes());

        let mut cursor = Cursor::new(data);
        let sync_samples = parse_stss(&mut cursor, 16).unwrap();

        assert_eq!(sync_samples, vec![1]);
    }
}

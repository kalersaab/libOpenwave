use super::super::parser::*;
use super::super::{Mp4Error, Mp4Result};
use std::io::{Read, Seek};

pub fn parse_stco<R: Read + Seek>(reader: &mut R, size: u64) -> Mp4Result<Vec<u64>> {
    ensure_box_size(size, 8, "stco box too small")?;
    skip_bytes(reader, 4)?;

    let entry_count = read_u32(reader)?;
    if (entry_count as u64) * 4 > size - 8 {
        return Err(Mp4Error::InvalidFormat("stco entry count exceeds box size"));
    }

    let mut offsets = Vec::with_capacity(entry_count as usize);

    for _ in 0..entry_count {
        offsets.push(read_u32(reader)? as u64);
    }

    Ok(offsets)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    #[test]
    fn parses_stco_offsets() {
        let mut data = vec![0u8; 12];
        data[4..8].copy_from_slice(&1u32.to_be_bytes());
        data[8..12].copy_from_slice(&100u32.to_be_bytes());

        let mut cursor = Cursor::new(data);
        let offsets = parse_stco(&mut cursor, 12).unwrap();

        assert_eq!(offsets, vec![100]);
    }
}

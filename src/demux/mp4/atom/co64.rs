use super::super::parser::*;
use super::super::{Mp4Error, Mp4Result};
use std::io::{Read, Seek};

pub fn parse_co64<R: Read + Seek>(reader: &mut R, size: u64) -> Mp4Result<Vec<u64>> {
    ensure_box_size(size, 8, "co64 box too small")?;
    skip_bytes(reader, 4)?;

    let entry_count = read_u32(reader)?;
    if (entry_count as u64) * 8 > size - 8 {
        return Err(Mp4Error::InvalidFormat("co64 entry count exceeds box size"));
    }

    let mut offsets = Vec::with_capacity(entry_count as usize);

    for _ in 0..entry_count {
        offsets.push(read_u64(reader)?);
    }

    Ok(offsets)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    #[test]
    fn parses_co64_offsets() {
        let base = u32::MAX as u64 + 123;
        let mut data = vec![0u8; 16];
        data[4..8].copy_from_slice(&1u32.to_be_bytes());
        data[8..16].copy_from_slice(&base.to_be_bytes());

        let mut cursor = Cursor::new(data);
        let offsets = parse_co64(&mut cursor, 16).unwrap();

        assert_eq!(offsets, vec![base]);
    }
}

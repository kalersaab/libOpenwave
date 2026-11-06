use super::super::parser::*;
use super::super::{Mp4Error, Mp4Result};
use std::io::{Read, Seek};

pub fn parse_stsz<R: Read + Seek>(reader: &mut R, size: u64) -> Mp4Result<Vec<u32>> {
    ensure_box_size(size, 12, "stsz box too small")?;
    skip_bytes(reader, 4)?;

    let sample_size = read_u32(reader)?;
    let sample_count = read_u32(reader)?;

    if sample_size != 0 {
        let sample_count = sample_count as usize;
        let mut sizes = Vec::new();
        sizes
            .try_reserve_exact(sample_count)
            .map_err(|_| Mp4Error::InvalidFormat("stsz sample count too large"))?;
        sizes.resize(sample_count, sample_size);
        Ok(sizes)
    } else {
        if (sample_count as u64) * 4 > size - 12 {
            return Err(Mp4Error::InvalidFormat("stsz entry count exceeds box size"));
        }

        let sample_count = sample_count as usize;
        let mut sizes = Vec::new();
        sizes
            .try_reserve_exact(sample_count)
            .map_err(|_| Mp4Error::InvalidFormat("stsz sample count too large"))?;

        for _ in 0..sample_count {
            sizes.push(read_u32(reader)?);
        }
        Ok(sizes)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    #[test]
    fn parses_stsz_uniform_sample_sizes() {
        let mut data = vec![0u8; 16];
        data[4..8].copy_from_slice(&4u32.to_be_bytes());
        data[8..12].copy_from_slice(&3u32.to_be_bytes());

        let mut cursor = Cursor::new(data);
        let sizes = parse_stsz(&mut cursor, 16).unwrap();

        assert_eq!(sizes, vec![4, 4, 4]);
    }

    #[test]
    fn parses_stsz_per_sample_sizes() {
        let mut data = vec![0u8; 20];
        data[4..8].copy_from_slice(&0u32.to_be_bytes());
        data[8..12].copy_from_slice(&2u32.to_be_bytes());
        data[12..16].copy_from_slice(&10u32.to_be_bytes());
        data[16..20].copy_from_slice(&20u32.to_be_bytes());

        let mut cursor = Cursor::new(data);
        let sizes = parse_stsz(&mut cursor, 20).unwrap();

        assert_eq!(sizes, vec![10, 20]);
    }
}

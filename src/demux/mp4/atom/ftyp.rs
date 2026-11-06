use super::super::parser::*;
use super::super::types::FileType;
use super::super::{Mp4Error, Mp4Result};
use std::io::{Read, Seek};

pub fn parse_ftyp<R: Read + Seek>(reader: &mut R, size: u64) -> Mp4Result<FileType> {
    if size < 8 {
        return Err(Mp4Error::InvalidFormat("ftyp box too small"));
    }

    let mut major_brand = [0u8; 4];
    reader.read_exact(&mut major_brand)?;
    let minor_version = read_u32(reader)?;

    let mut remaining = size - 8;
    let mut compatible_brands = Vec::with_capacity((remaining / 4) as usize);
    while remaining >= 4 {
        let mut brand = [0u8; 4];
        reader.read_exact(&mut brand)?;
        compatible_brands.push(brand);
        remaining -= 4;
    }

    if remaining > 0 {
        skip_bytes(reader, remaining)?;
    }

    Ok(FileType {
        major_brand,
        minor_version,
        compatible_brands,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    #[test]
    fn parses_file_type_metadata() {
        let mut data = Vec::new();
        data.extend_from_slice(b"isom");
        data.extend_from_slice(&512u32.to_be_bytes());
        data.extend_from_slice(b"iso2");
        data.extend_from_slice(b"avc1");

        let mut cursor = Cursor::new(data);
        let file_type = parse_ftyp(&mut cursor, 16).unwrap();

        assert_eq!(file_type.major_brand, *b"isom");
        assert_eq!(file_type.minor_version, 512);
        assert_eq!(file_type.compatible_brands, vec![*b"iso2", *b"avc1"]);
    }

    #[test]
    fn rejects_short_ftyp() {
        let mut cursor = Cursor::new(vec![0u8; 8]);
        assert!(parse_ftyp(&mut cursor, 4).is_err());
    }
}

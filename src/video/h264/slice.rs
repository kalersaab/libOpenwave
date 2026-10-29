use crate::video::bitstream::{BitReader, BitstreamError};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SliceError {
    InvalidNalType,
    InvalidSliceType,
    InvalidPpsId,
    Bitstream(BitstreamError),
}

impl std::fmt::Display for SliceError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "H.264 slice error: {self:?}")
    }
}

impl std::error::Error for SliceError {}

impl From<BitstreamError> for SliceError {
    fn from(value: BitstreamError) -> Self {
        Self::Bitstream(value)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SliceType {
    P,
    B,
    I,
    Sp,
    Si,
}

impl TryFrom<u32> for SliceType {
    type Error = SliceError;

    fn try_from(value: u32) -> Result<Self, Self::Error> {
        if value > 9 {
            return Err(SliceError::InvalidSliceType);
        }

        match value {
            0 | 5 => Ok(Self::P),
            1 | 6 => Ok(Self::B),
            2 | 7 => Ok(Self::I),
            3 | 8 => Ok(Self::Sp),
            4 | 9 => Ok(Self::Si),
            _ => Err(SliceError::InvalidSliceType),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SliceHeaderPrefix {
    pub first_mb_in_slice: u32,
    pub slice_type: SliceType,
    pub pps_id: u32,
    pub is_idr: bool,
}

fn ebsp_to_rbsp(ebsp: &[u8]) -> Vec<u8> {
    let mut rbsp = Vec::with_capacity(ebsp.len());
    let mut zeros = 0usize;

    for &byte in ebsp {
        if zeros >= 2 && byte == 3 {
            zeros = 0;
            continue;
        }

        rbsp.push(byte);

        if byte == 0 {
            zeros += 1;
        } else {
            zeros = 0;
        }
    }

    rbsp
}

/// Parses only the initial slice-header fields.
/// `nal` includes the one-byte NAL header.
/// This is not yet a complete slice-header parser.
pub fn parse_slice_prefix(nal: &[u8]) -> Result<SliceHeaderPrefix, SliceError> {
    let header = *nal.first().ok_or(SliceError::InvalidNalType)?;

    if header & 0x80 != 0 {
        return Err(SliceError::InvalidNalType);
    }

    let nal_type = header & 0x1f;

    if nal_type != 1 && nal_type != 5 {
        return Err(SliceError::InvalidNalType);
    }

    let rbsp = ebsp_to_rbsp(&nal[1..]);
    let mut reader = BitReader::new(&rbsp);

    let first_mb_in_slice = reader.read_ue()?;
    let raw_slice_type = reader.read_ue()?;

    if raw_slice_type > 9 {
        return Err(SliceError::InvalidSliceType);
    }

    let pps_id = reader.read_ue()?;

    if pps_id > 255 {
        return Err(SliceError::InvalidPpsId);
    }

    Ok(SliceHeaderPrefix {
        first_mb_in_slice,
        slice_type: SliceType::try_from(raw_slice_type)?,
        pps_id,
        is_idr: nal_type == 5,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    // Encode unsigned Exp-Golomb values MSB-first.
    fn ue_bits(value: u32, out: &mut Vec<bool>) {
        let code = u64::from(value) + 1;
        let width = 64 - code.leading_zeros();

        out.extend(std::iter::repeat_n(false, (width - 1) as usize));

        for shift in (0..width).rev() {
            out.push(((code >> shift) & 1) != 0);
        }
    }

    fn make_nal(nal_type: u8, values: &[u32]) -> Vec<u8> {
        let mut bits = Vec::new();

        for &value in values {
            ue_bits(value, &mut bits);
        }

        // Add RBSP trailing bits.
        bits.push(true);
        while !bits.len().is_multiple_of(8) {
            bits.push(false);
        }

        let mut nal = vec![nal_type];
        let (chunks, _) = bits.as_chunks::<8>();
        for chunk in chunks {
            let mut byte = 0u8;
            for &bit in chunk {
                byte = (byte << 1) | u8::from(bit);
            }
            nal.push(byte);
        }

        nal
    }

    #[test]
    fn parses_p_slice_prefix() {
        let nal = make_nal(0x41, &[0, 0, 3]);

        let parsed = parse_slice_prefix(&nal).unwrap();

        assert_eq!(parsed.first_mb_in_slice, 0);
        assert_eq!(parsed.slice_type, SliceType::P);
        assert_eq!(parsed.pps_id, 3);
        assert!(!parsed.is_idr);
    }

    #[test]
    fn parses_idr_slice_prefix() {
        let nal = make_nal(0x65, &[0, 1, 0]);

        let parsed = parse_slice_prefix(&nal).unwrap();

        assert_eq!(parsed.slice_type, SliceType::B);
        assert!(parsed.is_idr);
    }

    #[test]
    fn accepts_all_slice_type_values() {
        for raw in 0..=9 {
            assert!(SliceType::try_from(raw).is_ok());
        }

        assert_eq!(SliceType::try_from(10), Err(SliceError::InvalidSliceType));
    }

    #[test]
    fn rejects_sps_nal() {
        assert_eq!(
            parse_slice_prefix(&[0x67, 0x42, 0, 0]),
            Err(SliceError::InvalidNalType)
        );
    }
}

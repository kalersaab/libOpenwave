#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NalUnitType {
    Unspecified,
    CodedSliceNonIdr,
    CodedSliceDataPartitionA,
    CodedSliceDataPartitionB,
    CodedSliceDataPartitionC,
    Idr,
    Sei,
    Sps,
    Pps,
    Aud,
    EndOfSequence,
    EndOfStream,
    Filler,
    Other(u8),
}

impl From<u8> for NalUnitType {
    fn from(value: u8) -> Self {
        match value {
            1 => Self::CodedSliceNonIdr,
            2 => Self::CodedSliceDataPartitionA,
            3 => Self::CodedSliceDataPartitionB,
            4 => Self::CodedSliceDataPartitionC,
            5 => Self::Idr,
            6 => Self::Sei,
            7 => Self::Sps,
            8 => Self::Pps,
            9 => Self::Aud,
            10 => Self::EndOfSequence,
            11 => Self::EndOfStream,
            12 => Self::Filler,
            0 => Self::Unspecified,
            other => Self::Other(other),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NalHeader {
    pub nal_ref_idc: u8,
    pub unit_type: NalUnitType,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NalError {
    EmptyUnit,
    ForbiddenZeroBit,
}

impl NalHeader {
    pub fn parse(data: &[u8]) -> Result<Self, NalError> {
        let byte = *data.first().ok_or(NalError::EmptyUnit)?;

        if byte & 0x80 != 0 {
            return Err(NalError::ForbiddenZeroBit);
        }

        Ok(Self {
            nal_ref_idc: (byte >> 5) & 0x03,
            unit_type: NalUnitType::from(byte & 0x1f),
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NalUnit<'a> {
    pub header: NalHeader,

    pub data: &'a [u8],
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AnnexBError {
    MissingStartCode,
    EmptyUnit,
    InvalidHeader(NalError),
}

fn start_code_at(data: &[u8], pos: usize) -> Option<usize> {
    let rest = data.get(pos..)?;

    if rest.starts_with(&[0, 0, 0, 1]) {
        Some(4)
    } else if rest.starts_with(&[0, 0, 1]) {
        Some(3)
    } else {
        None
    }
}

pub fn parse_annex_b(data: &[u8]) -> Result<Vec<NalUnit<'_>>, AnnexBError> {
    let mut units = Vec::new();
    let mut pos = 0;

    while pos < data.len() {
        if start_code_at(data, pos).is_some() {
            break;
        }
        pos += 1;
    }

    if pos == data.len() {
        return Err(AnnexBError::MissingStartCode);
    }

    while pos < data.len() {
        let Some(start_len) = start_code_at(data, pos) else {
            pos += 1;
            continue;
        };

        let nal_start = pos + start_len;
        let mut next_start = nal_start;

        while next_start < data.len() && start_code_at(data, next_start).is_none() {
            next_start += 1;
        }

        let mut nal_end = next_start;

        while nal_end > nal_start && data[nal_end - 1] == 0 {
            nal_end -= 1;
        }

        if nal_start == nal_end {
            return Err(AnnexBError::EmptyUnit);
        }

        let nal_data = &data[nal_start..nal_end];
        let header = NalHeader::parse(nal_data).map_err(AnnexBError::InvalidHeader)?;

        units.push(NalUnit {
            header,
            data: nal_data,
        });

        pos = next_start;
    }

    if units.is_empty() {
        return Err(AnnexBError::MissingStartCode);
    }

    Ok(units)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_three_and_four_byte_start_codes() {
        let stream = [0, 0, 0, 1, 0x67, 0x42, 0, 0, 0, 0, 1, 0x68, 0xce, 0x06];

        let units = parse_annex_b(&stream).unwrap();

        assert_eq!(units.len(), 2);
        assert_eq!(units[0].header.unit_type, NalUnitType::Sps);
        assert_eq!(units[1].header.unit_type, NalUnitType::Pps);
    }

    #[test]
    fn rejects_forbidden_zero_bit() {
        assert_eq!(NalHeader::parse(&[0xE7]), Err(NalError::ForbiddenZeroBit));
    }

    #[test]
    fn rejects_missing_start_code() {
        assert_eq!(
            parse_annex_b(&[0x67, 0x42, 0x00]),
            Err(AnnexBError::MissingStartCode)
        );
    }

    #[test]
    fn rejects_empty_nal() {
        assert_eq!(
            parse_annex_b(&[0, 0, 1, 0, 0, 1]),
            Err(AnnexBError::EmptyUnit)
        );
    }
}

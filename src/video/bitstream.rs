#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BitstreamError {
    UnexpectedEof,
    InvalidBitCount,
    ExpGolombOverflow,
}

impl std::fmt::Display for BitstreamError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::UnexpectedEof => write!(f, "unexpected end of bitstream"),
            Self::InvalidBitCount => write!(f, "invalid bit count"),
            Self::ExpGolombOverflow => write!(f, "Exp-Golomb value overflow"),
        }
    }
}

impl std::error::Error for BitstreamError {}

pub struct BitReader<'a> {
    data: &'a [u8],
    bit_pos: usize,
}

impl<'a> BitReader<'a> {
    pub fn new(data: &'a [u8]) -> Self {
        Self { data, bit_pos: 0 }
    }

    pub fn bits_remaining(&self) -> usize {
        self.data
            .len()
            .saturating_mul(8)
            .saturating_sub(self.bit_pos)
    }

    pub fn bit_position(&self) -> usize {
        self.bit_pos
    }

    pub fn skip_bits(&mut self, count: usize) -> Result<(), BitstreamError> {
        if count > self.bits_remaining() {
            return Err(BitstreamError::UnexpectedEof);
        }
        self.bit_pos += count;
        Ok(())
    }

    pub fn read_bit(&mut self) -> Result<bool, BitstreamError> {
        if self.bit_pos >= self.data.len().saturating_mul(8) {
            return Err(BitstreamError::UnexpectedEof);
        }

        let byte = self.data[self.bit_pos / 8];
        let shift = 7 - (self.bit_pos % 8);
        self.bit_pos += 1;

        Ok(((byte >> shift) & 1) != 0)
    }

    pub fn read_bits(&mut self, count: u32) -> Result<u32, BitstreamError> {
        if count > 32 {
            return Err(BitstreamError::InvalidBitCount);
        }

        if count as usize > self.bits_remaining() {
            return Err(BitstreamError::UnexpectedEof);
        }

        let mut value = 0u32;

        for _ in 0..count {
            value = (value << 1) | u32::from(self.read_bit()?);
        }

        Ok(value)
    }

    pub fn read_ue(&mut self) -> Result<u32, BitstreamError> {
        let mut leading_zeros = 0u32;

        loop {
            if self.read_bit()? {
                break;
            }

            leading_zeros += 1;

            if leading_zeros > 31 {
                return Err(BitstreamError::ExpGolombOverflow);
            }
        }

        if leading_zeros == 0 {
            return Ok(0);
        }

        let suffix = self.read_bits(leading_zeros)?;

        let value = (1u64 << leading_zeros) - 1 + u64::from(suffix);

        u32::try_from(value).map_err(|_| BitstreamError::ExpGolombOverflow)
    }

    pub fn read_se(&mut self) -> Result<i32, BitstreamError> {
        let code_num = self.read_ue()?;

        if code_num == 0 {
            return Ok(0);
        }

        let magnitude = i64::from(code_num / 2 + code_num % 2);
        let signed = if code_num % 2 == 0 {
            -magnitude
        } else {
            magnitude
        };

        i32::try_from(signed).map_err(|_| BitstreamError::ExpGolombOverflow)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_bits_most_significant_first() {
        let mut reader = BitReader::new(&[0b1011_0010]);

        assert_eq!(reader.read_bits(4).unwrap(), 0b1011);
        assert_eq!(reader.read_bits(4).unwrap(), 0b0010);
        assert_eq!(reader.bits_remaining(), 0);
    }

    #[test]
    fn reads_unsigned_exp_golomb() {
        let mut reader = BitReader::new(&[0b1010_0110]);

        assert_eq!(reader.read_ue().unwrap(), 0);
        assert_eq!(reader.read_ue().unwrap(), 1);
        assert_eq!(reader.read_ue().unwrap(), 2);
    }

    #[test]
    fn reads_signed_exp_golomb() {
        let mut reader = BitReader::new(&[0b0100_1100, 0b1000_0000]);

        assert_eq!(reader.read_se().unwrap(), 1);
        assert_eq!(reader.read_se().unwrap(), -1);
        assert_eq!(reader.read_se().unwrap(), 2);
    }

    #[test]
    fn rejects_reads_past_end() {
        let mut reader = BitReader::new(&[0]);

        assert_eq!(reader.read_bits(9), Err(BitstreamError::UnexpectedEof));
    }

    #[test]
    fn rejects_more_than_32_bits() {
        let mut reader = BitReader::new(&[0; 8]);

        assert_eq!(reader.read_bits(33), Err(BitstreamError::InvalidBitCount));
    }
}

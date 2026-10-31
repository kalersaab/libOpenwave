use crate::video::{
    bitstream::{BitReader, BitstreamError},
    h264::macroblock::types::MacroblockType,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MacroblockParseError {
    Bitstream(BitstreamError),
    InvalidParameter,
    InvalidMacroblockType,
    InvalidCodedBlockPattern,
    UnsupportedMacroblockType,
    UnsupportedMacroblock,
    UnsupportedResidual,
}

impl std::fmt::Display for MacroblockParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "H.264 macroblock parse error: {self:?}")
    }
}

impl std::error::Error for MacroblockParseError {}

impl From<BitstreamError> for MacroblockParseError {
    fn from(value: BitstreamError) -> Self {
        Self::Bitstream(value)
    }
}

pub fn parse_p_macroblock(
    reader: &mut BitReader<'_>,
    active_reference_count: u32,
) -> Result<MacroblockType, MacroblockParseError> {
    if active_reference_count == 0 || active_reference_count > 32 {
        return Err(MacroblockParseError::InvalidParameter);
    }

    match reader.read_ue()? {
        0 => Ok(MacroblockType::PSkip),
        1 => {
            let reference_index = if active_reference_count > 1 {
                reader.read_ue()?
            } else {
                0
            };
            if reference_index >= active_reference_count {
                return Err(MacroblockParseError::InvalidParameter);
            }

            let motion_vector_difference = [reader.read_se()?, reader.read_se()?];
            let coded_block_pattern_code = reader.read_ue()?;
            if coded_block_pattern_code != 3 {
                return Err(MacroblockParseError::UnsupportedResidual);
            }

            Ok(MacroblockType::P16x16 {
                reference_index,
                motion_vector_difference,
                coded_block_pattern: 0,
            })
        }
        _ => Err(MacroblockParseError::UnsupportedMacroblock),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_p_skip_and_zero_residual_p16x16() {
        let mut skip = BitReader::new(&[0b1000_0000]);
        assert_eq!(parse_p_macroblock(&mut skip, 1), Ok(MacroblockType::PSkip));

        let mut p16x16 = BitReader::new(&[0b0101_1001, 0b0000_0000]);
        assert_eq!(
            parse_p_macroblock(&mut p16x16, 1),
            Ok(MacroblockType::P16x16 {
                reference_index: 0,
                motion_vector_difference: [0, 0],
                coded_block_pattern: 0,
            })
        );
    }

    #[test]
    fn rejects_invalid_reference_indices_and_residuals() {
        let mut invalid_ref = BitReader::new(&[0b0100_0000]);
        assert_eq!(
            parse_p_macroblock(&mut invalid_ref, 0),
            Err(MacroblockParseError::InvalidParameter)
        );

        let mut residual = BitReader::new(&[0b0101_1100]);
        assert_eq!(
            parse_p_macroblock(&mut residual, 1),
            Err(MacroblockParseError::UnsupportedResidual)
        );
    }
}

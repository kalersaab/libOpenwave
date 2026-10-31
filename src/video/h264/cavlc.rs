use crate::video::bitstream::{BitReader, BitstreamError};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CavlcError {
    InvalidParameter,
    InvalidCode,
    UnsupportedBlockSize,
    Bitstream(BitstreamError),
}

impl std::fmt::Display for CavlcError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "H.264 CAVLC error: {self:?}")
    }
}

impl std::error::Error for CavlcError {}

impl From<BitstreamError> for CavlcError {
    fn from(value: BitstreamError) -> Self {
        Self::Bitstream(value)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CoeffToken {
    pub total_coeff: u8,
    pub trailing_ones: u8,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct VlcCode {
    bits: u16,
    len: u8,
}

const fn code(bits: u16, len: u8) -> VlcCode {
    VlcCode { bits, len }
}

const COEFF_TOKEN_0: [&[VlcCode]; 17] = [
    &[code(1, 1)],
    &[code(5, 6), code(1, 2)],
    &[code(7, 8), code(4, 6), code(1, 3)],
    &[code(7, 9), code(6, 8), code(5, 7), code(3, 5)],
    &[code(7, 10), code(6, 9), code(5, 8), code(3, 6)],
    &[code(7, 11), code(6, 10), code(5, 9), code(4, 7)],
    &[code(15, 13), code(6, 11), code(5, 10), code(4, 8)],
    &[code(11, 13), code(14, 13), code(5, 11), code(4, 9)],
    &[code(8, 13), code(10, 13), code(13, 13), code(4, 10)],
    &[code(15, 14), code(14, 14), code(9, 13), code(4, 11)],
    &[code(11, 14), code(10, 14), code(13, 14), code(12, 13)],
    &[code(15, 15), code(14, 15), code(9, 14), code(12, 14)],
    &[code(11, 15), code(10, 15), code(13, 15), code(8, 14)],
    &[code(15, 16), code(1, 15), code(9, 15), code(12, 15)],
    &[code(11, 16), code(14, 16), code(13, 16), code(8, 15)],
    &[code(7, 16), code(10, 16), code(9, 16), code(12, 16)],
    &[code(4, 16), code(6, 16), code(5, 16), code(8, 16)],
];

const COEFF_TOKEN_1: [&[VlcCode]; 17] = [
    &[code(3, 2)],
    &[code(11, 6), code(2, 2)],
    &[code(7, 6), code(7, 5), code(3, 3)],
    &[code(7, 7), code(10, 6), code(9, 6), code(5, 4)],
    &[code(7, 8), code(6, 6), code(5, 6), code(4, 4)],
    &[code(4, 8), code(6, 7), code(5, 7), code(6, 5)],
    &[code(7, 9), code(6, 8), code(5, 8), code(8, 6)],
    &[code(15, 11), code(6, 9), code(5, 9), code(4, 6)],
    &[code(11, 11), code(14, 11), code(13, 11), code(4, 7)],
    &[code(15, 12), code(10, 11), code(9, 11), code(4, 9)],
    &[code(11, 12), code(14, 12), code(13, 12), code(12, 11)],
    &[code(8, 12), code(10, 12), code(9, 12), code(8, 11)],
    &[code(15, 13), code(14, 13), code(13, 13), code(12, 12)],
    &[code(11, 13), code(10, 13), code(9, 13), code(12, 13)],
    &[code(7, 13), code(11, 14), code(6, 13), code(8, 13)],
    &[code(9, 14), code(8, 14), code(10, 14), code(1, 13)],
    &[code(7, 14), code(6, 14), code(5, 14), code(4, 14)],
];

const COEFF_TOKEN_2: [&[VlcCode]; 17] = [
    &[code(15, 4)],
    &[code(15, 6), code(14, 4)],
    &[code(11, 6), code(15, 5), code(13, 4)],
    &[code(8, 6), code(12, 5), code(14, 5), code(12, 4)],
    &[code(15, 7), code(10, 5), code(11, 5), code(11, 4)],
    &[code(11, 7), code(8, 5), code(9, 5), code(10, 4)],
    &[code(9, 7), code(14, 6), code(13, 6), code(9, 4)],
    &[code(8, 7), code(10, 6), code(9, 6), code(8, 4)],
    &[code(15, 8), code(14, 7), code(13, 7), code(13, 5)],
    &[code(11, 8), code(14, 8), code(10, 7), code(12, 6)],
    &[code(15, 9), code(10, 8), code(13, 8), code(12, 7)],
    &[code(11, 9), code(14, 9), code(9, 8), code(12, 8)],
    &[code(8, 9), code(10, 9), code(13, 9), code(8, 8)],
    &[code(13, 10), code(7, 9), code(9, 9), code(12, 9)],
    &[code(9, 10), code(12, 10), code(11, 10), code(10, 10)],
    &[code(5, 10), code(8, 10), code(7, 10), code(6, 10)],
    &[code(1, 10), code(4, 10), code(3, 10), code(2, 10)],
];

const CHROMA_DC_COEFF_TOKEN: [&[VlcCode]; 5] = [
    &[code(1, 2)],
    &[code(7, 6), code(1, 1)],
    &[code(4, 6), code(6, 6), code(1, 3)],
    &[code(3, 6), code(3, 7), code(2, 7), code(5, 6)],
    &[code(2, 6), code(3, 8), code(2, 8), code(0, 7)],
];

const CHROMA422_DC_COEFF_TOKEN: [&[VlcCode]; 9] = [
    &[code(1, 1)],
    &[code(15, 7), code(1, 2)],
    &[code(14, 7), code(13, 7), code(1, 3)],
    &[code(7, 9), code(12, 7), code(11, 7), code(1, 5)],
    &[code(6, 9), code(5, 9), code(10, 7), code(1, 6)],
    &[code(7, 10), code(6, 10), code(4, 9), code(9, 7)],
    &[code(7, 11), code(6, 11), code(5, 10), code(8, 7)],
    &[code(7, 12), code(6, 12), code(5, 11), code(4, 10)],
    &[code(7, 13), code(5, 12), code(4, 12), code(4, 11)],
];

const TOTAL_ZEROS_16: [&[VlcCode]; 15] = [
    &[
        code(1, 1),
        code(3, 3),
        code(2, 3),
        code(3, 4),
        code(2, 4),
        code(3, 5),
        code(2, 5),
        code(3, 6),
        code(2, 6),
        code(3, 7),
        code(2, 7),
        code(3, 8),
        code(2, 8),
        code(3, 9),
        code(2, 9),
        code(1, 9),
    ],
    &[
        code(7, 3),
        code(6, 3),
        code(5, 3),
        code(4, 3),
        code(3, 3),
        code(5, 4),
        code(4, 4),
        code(3, 4),
        code(2, 4),
        code(3, 5),
        code(2, 5),
        code(3, 6),
        code(2, 6),
        code(1, 6),
        code(0, 6),
    ],
    &[
        code(5, 4),
        code(7, 3),
        code(6, 3),
        code(5, 3),
        code(4, 4),
        code(3, 4),
        code(4, 3),
        code(3, 3),
        code(2, 4),
        code(3, 5),
        code(2, 5),
        code(1, 6),
        code(1, 5),
        code(0, 6),
    ],
    &[
        code(3, 5),
        code(7, 3),
        code(5, 4),
        code(4, 4),
        code(6, 3),
        code(5, 3),
        code(4, 3),
        code(3, 4),
        code(3, 3),
        code(2, 4),
        code(2, 5),
        code(1, 5),
        code(0, 5),
    ],
    &[
        code(5, 4),
        code(4, 4),
        code(3, 4),
        code(7, 3),
        code(6, 3),
        code(5, 3),
        code(4, 3),
        code(3, 3),
        code(2, 4),
        code(1, 5),
        code(1, 4),
        code(0, 5),
    ],
    &[
        code(1, 6),
        code(1, 5),
        code(7, 3),
        code(6, 3),
        code(5, 3),
        code(4, 3),
        code(3, 3),
        code(2, 3),
        code(1, 4),
        code(1, 3),
        code(0, 6),
    ],
    &[
        code(1, 6),
        code(1, 5),
        code(5, 3),
        code(4, 3),
        code(3, 3),
        code(3, 2),
        code(2, 3),
        code(1, 4),
        code(1, 3),
        code(0, 6),
    ],
    &[
        code(1, 6),
        code(1, 4),
        code(1, 5),
        code(3, 3),
        code(3, 2),
        code(2, 2),
        code(2, 3),
        code(1, 3),
        code(0, 6),
    ],
    &[
        code(1, 6),
        code(0, 6),
        code(1, 4),
        code(3, 2),
        code(2, 2),
        code(1, 3),
        code(1, 2),
        code(1, 5),
    ],
    &[
        code(1, 5),
        code(0, 5),
        code(1, 3),
        code(3, 2),
        code(2, 2),
        code(1, 2),
        code(1, 4),
    ],
    &[
        code(0, 4),
        code(1, 4),
        code(1, 3),
        code(2, 3),
        code(1, 1),
        code(3, 3),
    ],
    &[code(0, 4), code(1, 4), code(1, 2), code(1, 1), code(1, 3)],
    &[code(0, 3), code(1, 3), code(1, 1), code(1, 2)],
    &[code(0, 2), code(1, 2), code(1, 1)],
    &[code(0, 1), code(1, 1)],
];

const CHROMA_DC_TOTAL_ZEROS_4: [&[VlcCode]; 3] = [
    &[code(1, 1), code(1, 2), code(1, 3), code(0, 3)],
    &[code(1, 1), code(1, 2), code(0, 2)],
    &[code(1, 1), code(0, 1)],
];

const CHROMA_DC_TOTAL_ZEROS_8: [&[VlcCode]; 7] = [
    &[
        code(1, 1),
        code(2, 3),
        code(3, 3),
        code(2, 4),
        code(3, 4),
        code(1, 4),
        code(1, 5),
        code(0, 5),
    ],
    &[
        code(0, 3),
        code(1, 2),
        code(1, 3),
        code(4, 3),
        code(5, 3),
        code(6, 3),
        code(7, 3),
    ],
    &[
        code(0, 3),
        code(1, 3),
        code(1, 2),
        code(2, 2),
        code(6, 3),
        code(7, 3),
    ],
    &[code(6, 3), code(0, 2), code(1, 2), code(2, 2), code(7, 3)],
    &[code(0, 2), code(1, 2), code(2, 2), code(3, 2)],
    &[code(0, 2), code(1, 2), code(1, 1)],
    &[code(0, 1), code(1, 1)],
];

const RUN_BEFORE: [&[VlcCode]; 7] = [
    &[code(1, 1), code(0, 1)],
    &[code(1, 1), code(1, 2), code(0, 2)],
    &[code(3, 2), code(2, 2), code(1, 2), code(0, 2)],
    &[code(3, 2), code(2, 2), code(1, 2), code(1, 3), code(0, 3)],
    &[
        code(3, 2),
        code(2, 2),
        code(3, 3),
        code(2, 3),
        code(1, 3),
        code(0, 3),
    ],
    &[
        code(3, 2),
        code(0, 3),
        code(1, 3),
        code(3, 3),
        code(2, 3),
        code(5, 3),
        code(4, 3),
    ],
    &[
        code(7, 3),
        code(6, 3),
        code(5, 3),
        code(4, 3),
        code(3, 3),
        code(2, 3),
        code(1, 3),
        code(1, 4),
        code(1, 5),
        code(1, 6),
        code(1, 7),
        code(1, 8),
        code(1, 9),
        code(1, 10),
        code(1, 11),
    ],
];

fn decode_vlc(reader: &mut BitReader<'_>, codes: &[VlcCode]) -> Result<usize, CavlcError> {
    let max_len = codes.iter().map(|entry| entry.len).max().unwrap_or(0);
    let mut bits = 0u16;

    for len in 1..=max_len {
        bits = (bits << 1) | u16::from(reader.read_bit()?);
        if let Some(index) = codes
            .iter()
            .position(|entry| entry.len == len && entry.bits == bits)
        {
            return Ok(index);
        }
    }

    Err(CavlcError::InvalidCode)
}

fn decode_coeff_token_from_rows(
    reader: &mut BitReader<'_>,
    rows: &[&[VlcCode]],
    max_num_coeff: u8,
) -> Result<CoeffToken, CavlcError> {
    let max_len = rows
        .iter()
        .flat_map(|row| row.iter())
        .map(|entry| entry.len)
        .max()
        .unwrap_or(0);
    let mut bits = 0u16;

    for len in 1..=max_len {
        bits = (bits << 1) | u16::from(reader.read_bit()?);
        for (total_coeff, row) in rows.iter().enumerate().take(usize::from(max_num_coeff) + 1) {
            if let Some(trailing_ones) = row
                .iter()
                .position(|entry| entry.len == len && entry.bits == bits)
            {
                return Ok(CoeffToken {
                    total_coeff: total_coeff as u8,
                    trailing_ones: trailing_ones as u8,
                });
            }
        }
    }

    Err(CavlcError::InvalidCode)
}

fn decode_coeff_token_fixed(
    reader: &mut BitReader<'_>,
    max_num_coeff: u8,
) -> Result<CoeffToken, CavlcError> {
    let bits = reader.read_bits(6)? as u16;
    if bits == 3 {
        return Ok(CoeffToken {
            total_coeff: 0,
            trailing_ones: 0,
        });
    }

    for total_coeff in 1..=max_num_coeff {
        for trailing_ones in 0..=total_coeff.min(3) {
            if 4 * u16::from(total_coeff - 1) + u16::from(trailing_ones) == bits {
                return Ok(CoeffToken {
                    total_coeff,
                    trailing_ones,
                });
            }
        }
    }

    Err(CavlcError::InvalidCode)
}

/// Decodes `coeff_token`; `None` represents an unavailable neighbor count.
/// `max_num_coeff` selects the special 4- or 8-coefficient chroma DC table.
pub fn decode_coeff_token(
    reader: &mut BitReader<'_>,
    n_c: Option<u32>,
    max_num_coeff: u8,
) -> Result<CoeffToken, CavlcError> {
    if max_num_coeff == 0 || max_num_coeff > 16 {
        return Err(CavlcError::UnsupportedBlockSize);
    }
    if max_num_coeff == 4 {
        return decode_coeff_token_from_rows(reader, &CHROMA_DC_COEFF_TOKEN, max_num_coeff);
    }
    if max_num_coeff == 8 {
        return decode_coeff_token_from_rows(reader, &CHROMA422_DC_COEFF_TOKEN, max_num_coeff);
    }
    if max_num_coeff != 15 && max_num_coeff != 16 {
        return Err(CavlcError::UnsupportedBlockSize);
    }

    let n_c = n_c.unwrap_or(0);
    let table = if n_c < 2 {
        &COEFF_TOKEN_0
    } else if n_c < 4 {
        &COEFF_TOKEN_1
    } else if n_c < 8 {
        &COEFF_TOKEN_2
    } else {
        return decode_coeff_token_fixed(reader, max_num_coeff);
    };

    decode_coeff_token_from_rows(reader, table, max_num_coeff)
}

/// Decodes `total_zeros` for a block with the given coefficient capacity.
pub fn decode_total_zeros(
    reader: &mut BitReader<'_>,
    total_coeff: u8,
    max_num_coeff: u8,
) -> Result<u8, CavlcError> {
    if total_coeff == 0 || total_coeff > max_num_coeff || max_num_coeff > 16 {
        return Err(CavlcError::InvalidParameter);
    }
    if total_coeff == max_num_coeff {
        return Ok(0);
    }

    let table = match max_num_coeff {
        4 => &CHROMA_DC_TOTAL_ZEROS_4[..],
        8 => &CHROMA_DC_TOTAL_ZEROS_8[..],
        1..=16 => &TOTAL_ZEROS_16[..],
        _ => return Err(CavlcError::UnsupportedBlockSize),
    };
    let row = table
        .get(usize::from(total_coeff - 1))
        .ok_or(CavlcError::InvalidParameter)?;
    let total_zeros = decode_vlc(reader, row)? as u8;
    if total_zeros > max_num_coeff - total_coeff {
        return Err(CavlcError::InvalidCode);
    }
    Ok(total_zeros)
}

/// Decodes `run_before` using the current number of zero coefficients left.
pub fn decode_run_before(reader: &mut BitReader<'_>, zeros_left: u8) -> Result<u8, CavlcError> {
    if zeros_left == 0 {
        return Ok(0);
    }
    if zeros_left > 15 {
        return Err(CavlcError::InvalidParameter);
    }

    let row = usize::from(zeros_left.min(7) - 1);
    let run_before = decode_vlc(reader, RUN_BEFORE[row])? as u8;
    if run_before > zeros_left {
        return Err(CavlcError::InvalidCode);
    }
    Ok(run_before)
}

fn decode_level(reader: &mut BitReader<'_>, suffix_length: u8) -> Result<u32, CavlcError> {
    let mut level_prefix = 0u8;
    while !reader.read_bit()? {
        level_prefix = level_prefix.checked_add(1).ok_or(CavlcError::InvalidCode)?;
        if level_prefix > 31 {
            return Err(CavlcError::InvalidCode);
        }
    }

    let suffix_size = if level_prefix == 14 && suffix_length == 0 {
        4
    } else if level_prefix >= 15 {
        level_prefix - 3
    } else {
        suffix_length
    };
    let suffix = u64::from(reader.read_bits(u32::from(suffix_size))?);
    let mut level_code = u64::from(level_prefix.min(15)) << u32::from(suffix_length);
    level_code += suffix;
    if level_prefix >= 15 && suffix_length == 0 {
        level_code += 15;
    }
    if level_prefix >= 16 {
        level_code += (1u64 << (level_prefix - 3)) - 4096;
    }

    u32::try_from(level_code).map_err(|_| CavlcError::InvalidCode)
}

/// Decodes coefficient levels in bitstream order (highest scan position first).
pub fn decode_levels(
    reader: &mut BitReader<'_>,
    token: CoeffToken,
) -> Result<Vec<i32>, CavlcError> {
    if token.total_coeff > 16 || token.trailing_ones > token.total_coeff.min(3) {
        return Err(CavlcError::InvalidParameter);
    }

    let mut levels = Vec::with_capacity(usize::from(token.total_coeff));
    for _ in 0..token.trailing_ones {
        levels.push(if reader.read_bit()? { -1 } else { 1 });
    }

    let mut suffix_length: u8 = u8::from(token.total_coeff > 10 && token.trailing_ones < 3);
    for index in token.trailing_ones..token.total_coeff {
        let mut level_code = decode_level(reader, suffix_length)?;
        if index == token.trailing_ones && token.trailing_ones < 3 {
            level_code = level_code.checked_add(2).ok_or(CavlcError::InvalidCode)?;
        }
        let magnitude = i64::from((level_code + 2 - (level_code & 1)) >> 1);
        let level = if level_code & 1 == 0 {
            magnitude
        } else {
            -magnitude
        };
        let level = i32::try_from(level).map_err(|_| CavlcError::InvalidCode)?;
        levels.push(level);

        if suffix_length == 0 {
            suffix_length = 1;
        }
        if level.unsigned_abs() > (3u32 << (suffix_length - 1)) && suffix_length < 6 {
            suffix_length += 1;
        }
    }

    Ok(levels)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn reader(bits: &str) -> Vec<u8> {
        let mut bytes = vec![0u8; bits.len().div_ceil(8)];
        for (index, bit) in bits.bytes().enumerate() {
            if bit == b'1' {
                bytes[index / 8] |= 1 << (7 - index % 8);
            }
        }
        bytes
    }

    fn code_bits(code: VlcCode) -> String {
        (0..code.len)
            .rev()
            .map(|shift| {
                if (code.bits >> shift) & 1 == 1 {
                    '1'
                } else {
                    '0'
                }
            })
            .collect()
    }

    fn assert_coeff_table(rows: &[&[VlcCode]], max_num_coeff: u8, n_c: Option<u32>) {
        for (total_coeff, row) in rows.iter().enumerate() {
            for (trailing_ones, &entry) in row.iter().enumerate() {
                let bits = code_bits(entry);
                let bytes = reader(&bits);
                let decoded =
                    decode_coeff_token(&mut BitReader::new(&bytes), n_c, max_num_coeff).unwrap();
                assert_eq!(
                    decoded,
                    CoeffToken {
                        total_coeff: total_coeff as u8,
                        trailing_ones: trailing_ones as u8
                    },
                    "total_coeff={total_coeff}, trailing_ones={trailing_ones}"
                );
            }
        }
    }

    #[test]
    fn selects_coeff_token_tables_from_neighbor_context() {
        let bytes = reader("000101");
        assert_eq!(
            decode_coeff_token(&mut BitReader::new(&bytes), Some(0), 16),
            Ok(CoeffToken {
                total_coeff: 1,
                trailing_ones: 0
            })
        );

        let bytes = reader("01");
        assert_eq!(
            decode_coeff_token(&mut BitReader::new(&bytes), None, 16),
            Ok(CoeffToken {
                total_coeff: 1,
                trailing_ones: 1
            })
        );

        for (n_c, expected) in [(2, "11"), (4, "1111")] {
            let bytes = reader(expected);
            assert_eq!(
                decode_coeff_token(&mut BitReader::new(&bytes), Some(n_c), 16),
                Ok(CoeffToken {
                    total_coeff: 0,
                    trailing_ones: 0
                })
            );
        }

        let bytes = reader("000000");
        assert_eq!(
            decode_coeff_token(&mut BitReader::new(&bytes), Some(8), 16),
            Ok(CoeffToken {
                total_coeff: 1,
                trailing_ones: 0
            })
        );
    }

    #[test]
    fn decodes_chroma_dc_coeff_tokens() {
        let bytes = reader("01");
        assert_eq!(
            decode_coeff_token(&mut BitReader::new(&bytes), None, 4),
            Ok(CoeffToken {
                total_coeff: 0,
                trailing_ones: 0
            })
        );

        let bytes = reader("1");
        assert_eq!(
            decode_coeff_token(&mut BitReader::new(&bytes), None, 8),
            Ok(CoeffToken {
                total_coeff: 0,
                trailing_ones: 0
            })
        );
    }

    #[test]
    fn decodes_total_zeros_tables() {
        let bytes = reader("1");
        assert_eq!(
            decode_total_zeros(&mut BitReader::new(&bytes), 1, 16),
            Ok(0)
        );

        let bytes = reader("000");
        assert_eq!(decode_total_zeros(&mut BitReader::new(&bytes), 1, 4), Ok(3));

        let bytes = reader("010");
        assert_eq!(decode_total_zeros(&mut BitReader::new(&bytes), 1, 8), Ok(1));
    }

    #[test]
    fn decodes_run_before_and_skips_zero_syntax() {
        let bytes = reader("1");
        assert_eq!(decode_run_before(&mut BitReader::new(&bytes), 1), Ok(0));

        let bytes = reader("111");
        assert_eq!(decode_run_before(&mut BitReader::new(&bytes), 7), Ok(0));

        let bytes = reader("");
        let mut bit_reader = BitReader::new(&bytes);
        assert_eq!(decode_run_before(&mut bit_reader, 0), Ok(0));
        assert_eq!(bit_reader.bits_remaining(), 0);
    }

    #[test]
    fn decodes_trailing_and_non_trailing_levels() {
        let bytes = reader("010");
        assert_eq!(
            decode_levels(
                &mut BitReader::new(&bytes),
                CoeffToken {
                    total_coeff: 3,
                    trailing_ones: 3
                }
            ),
            Ok(vec![1, -1, 1])
        );

        let bytes = reader("1");
        assert_eq!(
            decode_levels(
                &mut BitReader::new(&bytes),
                CoeffToken {
                    total_coeff: 1,
                    trailing_ones: 0
                }
            ),
            Ok(vec![2])
        );

        let bytes = reader("010");
        assert_eq!(
            decode_levels(
                &mut BitReader::new(&bytes),
                CoeffToken {
                    total_coeff: 1,
                    trailing_ones: 0
                }
            ),
            Ok(vec![-2])
        );
    }

    #[test]
    fn decodes_level_prefix_and_suffix_adjustments() {
        let bytes = reader("001");
        assert_eq!(
            decode_levels(
                &mut BitReader::new(&bytes),
                CoeffToken {
                    total_coeff: 1,
                    trailing_ones: 0
                }
            ),
            Ok(vec![3])
        );

        let bytes = reader("0001");
        assert_eq!(
            decode_levels(
                &mut BitReader::new(&bytes),
                CoeffToken {
                    total_coeff: 1,
                    trailing_ones: 0
                }
            ),
            Ok(vec![-3])
        );

        let bytes = reader("0001");
        assert_eq!(
            decode_levels(
                &mut BitReader::new(&bytes),
                CoeffToken {
                    total_coeff: 4,
                    trailing_ones: 3
                }
            ),
            Ok(vec![1, 1, 1, 1])
        );
    }

    #[test]
    fn updates_level_suffix_length_after_large_levels() {
        let bits = format!("0010{}", "100".repeat(10));
        let bytes = reader(&bits);
        assert_eq!(
            decode_levels(
                &mut BitReader::new(&bytes),
                CoeffToken {
                    total_coeff: 11,
                    trailing_ones: 0
                }
            ),
            Ok(vec![4, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1])
        );
    }

    #[test]
    fn all_coeff_token_table_entries_decode_to_their_symbols() {
        assert_coeff_table(&COEFF_TOKEN_0, 16, Some(0));
        assert_coeff_table(&COEFF_TOKEN_1, 16, Some(2));
        assert_coeff_table(&COEFF_TOKEN_2, 16, Some(4));
        assert_coeff_table(&CHROMA_DC_COEFF_TOKEN, 4, None);
        assert_coeff_table(&CHROMA422_DC_COEFF_TOKEN, 8, None);
    }

    #[test]
    fn all_fixed_length_coeff_token_values_decode() {
        for total_coeff in 0..=16 {
            for trailing_ones in 0..=total_coeff.min(3) {
                let value = if total_coeff == 0 {
                    3
                } else {
                    4 * (total_coeff - 1) + trailing_ones
                };
                let bytes = reader(&format!("{value:06b}"));
                assert_eq!(
                    decode_coeff_token(&mut BitReader::new(&bytes), Some(8), 16),
                    Ok(CoeffToken {
                        total_coeff: total_coeff as u8,
                        trailing_ones: trailing_ones as u8
                    })
                );
            }
        }
    }

    #[test]
    fn all_total_zeros_table_entries_decode_to_their_symbols() {
        for (total_coeff, row) in TOTAL_ZEROS_16.iter().enumerate() {
            for (zeros, &entry) in row.iter().enumerate() {
                let bytes = reader(&code_bits(entry));
                assert_eq!(
                    decode_total_zeros(&mut BitReader::new(&bytes), total_coeff as u8 + 1, 16),
                    Ok(zeros as u8),
                    "table=16, total_coeff={}, zeros={zeros}, code={}",
                    total_coeff + 1,
                    code_bits(entry)
                );
            }
        }

        for (max_num_coeff, rows) in [
            (4, &CHROMA_DC_TOTAL_ZEROS_4[..]),
            (8, &CHROMA_DC_TOTAL_ZEROS_8[..]),
        ] {
            for (total_coeff, row) in rows.iter().enumerate() {
                for (zeros, &entry) in row.iter().enumerate() {
                    let bytes = reader(&code_bits(entry));
                    assert_eq!(
                        decode_total_zeros(
                            &mut BitReader::new(&bytes),
                            total_coeff as u8 + 1,
                            max_num_coeff
                        ),
                        Ok(zeros as u8),
                        "table={max_num_coeff}, total_coeff={}, zeros={zeros}, code={}",
                        total_coeff + 1,
                        code_bits(entry)
                    );
                }
            }
        }
    }

    #[test]
    fn all_run_before_table_entries_decode_to_their_symbols() {
        for (row_index, row) in RUN_BEFORE.iter().enumerate() {
            let zeros_left = if row_index == 6 {
                15
            } else {
                row_index as u8 + 1
            };
            for (run_before, &entry) in row.iter().enumerate() {
                if run_before as u8 > zeros_left {
                    continue;
                }
                let bytes = reader(&code_bits(entry));
                assert_eq!(
                    decode_run_before(&mut BitReader::new(&bytes), zeros_left),
                    Ok(run_before as u8)
                );
            }
        }
    }

    #[test]
    fn rejects_invalid_block_size_and_truncated_codes() {
        let bytes = reader("");
        assert_eq!(
            decode_coeff_token(&mut BitReader::new(&bytes), None, 0),
            Err(CavlcError::UnsupportedBlockSize)
        );
        assert_eq!(
            decode_coeff_token(&mut BitReader::new(&bytes), None, 16),
            Err(CavlcError::Bitstream(BitstreamError::UnexpectedEof))
        );
    }
}

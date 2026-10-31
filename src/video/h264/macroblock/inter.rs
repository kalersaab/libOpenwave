use crate::video::{
    bitstream::BitReader,
    h264::macroblock::{MacroblockParseError, MacroblockType},
};

pub fn parse_p_macroblock(
    reader: &mut BitReader<'_>,
    num_ref_idx_l0_active: u32,
) -> Result<MacroblockType, MacroblockParseError> {
    let mb_type = reader.read_ue()?;

    match mb_type {
        0 => {
            let reference_index = if num_ref_idx_l0_active > 1 {
                parse_te(reader, num_ref_idx_l0_active)?
            } else {
                0
            };

            let mvd_x = reader.read_se()?;
            let mvd_y = reader.read_se()?;
            let coded_block_pattern = parse_coded_block_pattern(reader, false)?;

            Ok(MacroblockType::P16x16 {
                reference_index,
                motion_vector_difference: [mvd_x, mvd_y],
                coded_block_pattern,
            })
        }

        1 => {
            let mut partitions = [[0i32; 2]; 2];
            let mut ref_indices = [0u32; 2];

            for i in 0..2 {
                ref_indices[i] = if num_ref_idx_l0_active > 1 {
                    parse_te(reader, num_ref_idx_l0_active)?
                } else {
                    0
                };
                partitions[i][0] = reader.read_se()?;
                partitions[i][1] = reader.read_se()?;
            }

            let coded_block_pattern = parse_coded_block_pattern(reader, false)?;

            Ok(MacroblockType::P16x8 {
                reference_indices: ref_indices,
                motion_vector_differences: partitions,
                coded_block_pattern,
            })
        }

        2 => {
            let mut partitions = [[0i32; 2]; 2];
            let mut ref_indices = [0u32; 2];

            for i in 0..2 {
                ref_indices[i] = if num_ref_idx_l0_active > 1 {
                    parse_te(reader, num_ref_idx_l0_active)?
                } else {
                    0
                };
                partitions[i][0] = reader.read_se()?;
                partitions[i][1] = reader.read_se()?;
            }

            let coded_block_pattern = parse_coded_block_pattern(reader, false)?;

            Ok(MacroblockType::P8x16 {
                reference_indices: ref_indices,
                motion_vector_differences: partitions,
                coded_block_pattern,
            })
        }

        3 | 4 => Err(MacroblockParseError::UnsupportedMacroblockType),

        5..=24 => Err(MacroblockParseError::UnsupportedMacroblockType),

        _ => Err(MacroblockParseError::InvalidMacroblockType),
    }
}

pub fn parse_b_macroblock(
    reader: &mut BitReader<'_>,
    num_ref_idx_l0_active: u32,
    num_ref_idx_l1_active: u32,
) -> Result<MacroblockType, MacroblockParseError> {
    let mb_type = reader.read_ue()?;

    match mb_type {
        0 => Ok(MacroblockType::BDirect16x16),

        1 => {
            let reference_index = if num_ref_idx_l0_active > 1 {
                parse_te(reader, num_ref_idx_l0_active)?
            } else {
                0
            };

            let mvd_x = reader.read_se()?;
            let mvd_y = reader.read_se()?;
            let coded_block_pattern = parse_coded_block_pattern(reader, false)?;

            Ok(MacroblockType::BL0_16x16 {
                reference_index,
                motion_vector_difference: [mvd_x, mvd_y],
                coded_block_pattern,
            })
        }

        2 => {
            let reference_index = if num_ref_idx_l1_active > 1 {
                parse_te(reader, num_ref_idx_l1_active)?
            } else {
                0
            };

            let mvd_x = reader.read_se()?;
            let mvd_y = reader.read_se()?;
            let coded_block_pattern = parse_coded_block_pattern(reader, false)?;

            Ok(MacroblockType::BL1_16x16 {
                reference_index,
                motion_vector_difference: [mvd_x, mvd_y],
                coded_block_pattern,
            })
        }

        3 => {
            let ref_idx_l0 = if num_ref_idx_l0_active > 1 {
                parse_te(reader, num_ref_idx_l0_active)?
            } else {
                0
            };

            let mvd_l0_x = reader.read_se()?;
            let mvd_l0_y = reader.read_se()?;

            let ref_idx_l1 = if num_ref_idx_l1_active > 1 {
                parse_te(reader, num_ref_idx_l1_active)?
            } else {
                0
            };

            let mvd_l1_x = reader.read_se()?;
            let mvd_l1_y = reader.read_se()?;

            let coded_block_pattern = parse_coded_block_pattern(reader, false)?;

            Ok(MacroblockType::BBi16x16 {
                reference_index_l0: ref_idx_l0,
                motion_vector_difference_l0: [mvd_l0_x, mvd_l0_y],
                reference_index_l1: ref_idx_l1,
                motion_vector_difference_l1: [mvd_l1_x, mvd_l1_y],
                coded_block_pattern,
            })
        }

        4.. => Err(MacroblockParseError::UnsupportedMacroblockType),
    }
}

fn parse_te(reader: &mut BitReader<'_>, range: u32) -> Result<u32, MacroblockParseError> {
    if range == 1 {
        Ok(if reader.read_bit()? { 1 } else { 0 })
    } else {
        Ok(reader.read_ue()?)
    }
}

fn parse_coded_block_pattern(
    reader: &mut BitReader<'_>,
    is_intra: bool,
) -> Result<u32, MacroblockParseError> {
    let cbp = reader.read_ue()?;

    let mapped = if is_intra {
        cbp
    } else {
        match cbp {
            0 => 0,
            1 => 16,
            2 => 1,
            3 => 2,
            4 => 4,
            5 => 8,
            6 => 32,
            7 => 3,
            8 => 5,
            9 => 10,
            10 => 12,
            11 => 15,
            12 => 47,
            13 => 7,
            14 => 11,
            15 => 13,
            16 => 14,
            17 => 6,
            18 => 9,
            19 => 31,
            20 => 35,
            21 => 37,
            22 => 42,
            23 => 44,
            24 => 33,
            25 => 34,
            26 => 36,
            27 => 40,
            28 => 39,
            29 => 43,
            30 => 45,
            31 => 46,
            32 => 17,
            33 => 18,
            34 => 20,
            35 => 24,
            36 => 19,
            37 => 21,
            38 => 26,
            39 => 28,
            40 => 23,
            41 => 27,
            42 => 29,
            43 => 30,
            44 => 22,
            45 => 25,
            46 => 38,
            47 => 41,
            _ => return Err(MacroblockParseError::InvalidCodedBlockPattern),
        }
    };

    Ok(mapped)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::video::bitstream::BitReader;

    fn create_test_reader(data: &[u8]) -> BitReader<'_> {
        BitReader::new(data)
    }

    #[test]
    fn parses_p16x16_macroblock() {
        let mut bits = vec![];

        bits.push(true);

        bits.extend([false, false, true, false, false]);

        bits.extend([false, true, true]);

        bits.push(true);

        bits.push(true);
        while bits.len() % 8 != 0 {
            bits.push(false);
        }

        let mut bytes = vec![];
        for chunk in bits.chunks(8) {
            let mut byte = 0u8;
            for (i, &bit) in chunk.iter().enumerate() {
                if bit {
                    byte |= 1 << (7 - i);
                }
            }
            bytes.push(byte);
        }

        let mut reader = create_test_reader(&bytes);
        let mb = parse_p_macroblock(&mut reader, 1).unwrap();

        if let MacroblockType::P16x16 {
            reference_index,
            motion_vector_difference,
            coded_block_pattern,
        } = mb
        {
            assert_eq!(reference_index, 0);
            assert_eq!(motion_vector_difference, [2, -1]);
            assert_eq!(coded_block_pattern, 0);
        } else {
            panic!("Expected P16x16 macroblock, got: {:?}", mb);
        }
    }

    #[test]
    fn parses_b_direct_macroblock() {
        let data = [0b10000000];

        let mut reader = create_test_reader(&data);
        let mb = parse_b_macroblock(&mut reader, 1, 1).unwrap();

        assert_eq!(mb, MacroblockType::BDirect16x16);
    }

    #[test]
    fn maps_coded_block_pattern() {
        let data = [0b01000000];
        let mut reader = create_test_reader(&data);

        let cbp = parse_coded_block_pattern(&mut reader, false).unwrap();
        assert_eq!(cbp, 16);
    }
}

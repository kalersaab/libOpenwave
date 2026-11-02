use crate::audio::aac::bitstream::BitReader;
use std::io;

#[derive(Debug, Clone, Copy)]
pub struct HuffmanEntry {
    pub bits: u16,
    pub len: u8,
    pub signed: bool,
}

pub struct HuffmanCodebook {
    entries: &'static [(u16, u8, i8, i8)],
    max_len: u8,
    signed: bool,
}

impl HuffmanCodebook {
    const fn new(entries: &'static [(u16, u8, i8, i8)], max_len: u8, signed: bool) -> Self {
        HuffmanCodebook {
            entries,
            max_len,
            signed,
        }
    }

    pub fn decode(&self, br: &mut BitReader) -> io::Result<(i16, i16)> {
        let mut code = 0u16;

        for len in 1..=self.max_len {
            let bit = br.read_bit()? as u16;
            code = (code << 1) | bit;

            for &(pattern, pattern_len, w, x) in self.entries {
                if pattern_len == len && pattern == code {
                    let mut w_val = w as i16;
                    let mut x_val = x as i16;

                    if self.signed {
                        if w_val != 0 && br.read_bit()? {
                            w_val = -w_val;
                        }
                        if x_val != 0 && br.read_bit()? {
                            x_val = -x_val;
                        }
                    }

                    if w_val == 16 || w_val == -16 {
                        let escape = self.read_escape(br)?;
                        w_val = if w_val > 0 { escape } else { -escape };
                    }
                    if x_val == 16 || x_val == -16 {
                        let escape = self.read_escape(br)?;
                        x_val = if x_val > 0 { escape } else { -escape };
                    }

                    return Ok((w_val, x_val));
                }
            }
        }

        Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "Invalid Huffman code",
        ))
    }

    fn read_escape(&self, br: &mut BitReader) -> io::Result<i16> {
        let mut n = 4u16;
        while br.read_bit()? {
            n += 1;
            if n > 13 {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "Escape value too large",
                ));
            }
        }

        let value = br.read_bits(n as u8)? as i16;
        Ok((1 << n) + value)
    }
}

pub static HUFFMAN_CODEBOOK_1: HuffmanCodebook = HuffmanCodebook::new(
    &[
        (0b00, 2, 0, 0),
        (0b01, 2, 1, 1),
        (0b10, 2, -1, -1),
        (0b110, 3, -1, 0),
        (0b111, 3, 0, -1),
        (0b1000, 4, 1, 0),
        (0b1001, 4, 0, 1),
        (0b1010, 4, -1, 1),
        (0b1011, 4, 1, -1),
    ],
    4,
    false,
);

pub static HUFFMAN_CODEBOOK_2: HuffmanCodebook = HuffmanCodebook::new(
    &[
        (0b00, 2, 0, 0),
        (0b010, 3, 1, 1),
        (0b011, 3, -1, -1),
        (0b100, 3, 1, 0),
        (0b101, 3, 0, 1),
        (0b1100, 4, -1, 0),
        (0b1101, 4, 0, -1),
        (0b1110, 4, -1, 1),
        (0b1111, 4, 1, -1),
    ],
    4,
    false,
);

pub static HUFFMAN_CODEBOOK_3: HuffmanCodebook = HuffmanCodebook::new(
    &[
        (0b0, 1, 0, 0),
        (0b100, 3, 1, 0),
        (0b101, 3, 0, 1),
        (0b110, 3, 1, 1),
        (0b111, 3, -1, -1),
    ],
    3,
    true,
);

pub static HUFFMAN_CODEBOOK_4: HuffmanCodebook = HuffmanCodebook::new(
    &[
        (0b00, 2, 0, 0),
        (0b010, 3, 1, 0),
        (0b011, 3, 0, 1),
        (0b10, 2, 1, 1),
        (0b110, 3, -1, -1),
        (0b1110, 4, -1, 0),
        (0b1111, 4, 0, -1),
    ],
    4,
    true,
);

pub static HUFFMAN_CODEBOOK_5: HuffmanCodebook = HuffmanCodebook::new(
    &[
        (0b0, 1, 0, 0),
        (0b100, 3, 1, 1),
        (0b101, 3, -1, 1),
        (0b110, 3, 1, -1),
        (0b111, 3, -1, -1),
        (0b1000, 4, 2, 2),
        (0b1001, 4, -2, 2),
        (0b1010, 4, 2, -2),
        (0b1011, 4, -2, -2),
    ],
    4,
    true,
);

pub static HUFFMAN_CODEBOOK_6: HuffmanCodebook = HuffmanCodebook::new(
    &[
        (0b00, 2, 0, 0),
        (0b010, 3, 1, 1),
        (0b011, 3, -1, 1),
        (0b100, 3, 1, -1),
        (0b101, 3, -1, -1),
        (0b110, 3, 2, 2),
        (0b1110, 4, -2, 2),
        (0b1111, 4, 2, -2),
        (0b10000, 5, -2, -2),
    ],
    5,
    true,
);

pub static HUFFMAN_CODEBOOK_7: HuffmanCodebook = HuffmanCodebook::new(
    &[(0b0, 1, 0, 0), (0b100, 3, 1, 0), (0b101, 3, 0, 1)],
    3,
    true,
);

pub static HUFFMAN_CODEBOOK_8: HuffmanCodebook = HuffmanCodebook::new(
    &[(0b00, 2, 0, 0), (0b010, 3, 1, 0), (0b011, 3, 0, 1)],
    3,
    true,
);

pub static HUFFMAN_CODEBOOK_9: HuffmanCodebook = HuffmanCodebook::new(
    &[
        (0b0, 1, 0, 0),
        (0b100, 3, 1, 1),
        (0b101, 3, -1, 1),
        (0b110, 3, 1, -1),
        (0b111, 3, -1, -1),
    ],
    3,
    true,
);

pub static HUFFMAN_CODEBOOK_10: HuffmanCodebook = HuffmanCodebook::new(
    &[
        (0b00, 2, 0, 0),
        (0b010, 3, 1, 1),
        (0b011, 3, -1, 1),
        (0b100, 3, 1, -1),
        (0b101, 3, -1, -1),
    ],
    3,
    true,
);

pub static HUFFMAN_CODEBOOK_11: HuffmanCodebook = HuffmanCodebook::new(
    &[
        (0b0000, 4, 0, 0),
        (0b0001, 4, 1, 1),
        (0b0010, 4, 1, 2),
        (0b0011, 4, 2, 1),
        (0b0100, 4, 2, 2),
        (0b0101, 4, 1, 3),
        (0b0110, 4, 3, 1),
        (0b0111, 4, 3, 2),
        (0b1000, 4, 2, 3),
        (0b1001, 4, 3, 3),
        (0b1010, 4, 4, 4),
        (0b10110, 5, 16, 16),
    ],
    5,
    true,
);

pub fn get_codebook(index: u8) -> Option<&'static HuffmanCodebook> {
    match index {
        1 => Some(&HUFFMAN_CODEBOOK_1),
        2 => Some(&HUFFMAN_CODEBOOK_2),
        3 => Some(&HUFFMAN_CODEBOOK_3),
        4 => Some(&HUFFMAN_CODEBOOK_4),
        5 => Some(&HUFFMAN_CODEBOOK_5),
        6 => Some(&HUFFMAN_CODEBOOK_6),
        7 => Some(&HUFFMAN_CODEBOOK_7),
        8 => Some(&HUFFMAN_CODEBOOK_8),
        9 => Some(&HUFFMAN_CODEBOOK_9),
        10 => Some(&HUFFMAN_CODEBOOK_10),
        11 => Some(&HUFFMAN_CODEBOOK_11),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_codebook_1() {
        let data = [0b00_000000];
        let mut br = BitReader::new(&data);

        let (w, x) = HUFFMAN_CODEBOOK_1.decode(&mut br).unwrap();
        assert_eq!((w, x), (0, 0));
    }

    #[test]
    fn decodes_signed_values() {
        let data = [0b1001_0000];
        let mut br = BitReader::new(&data);

        let (w, x) = HUFFMAN_CODEBOOK_3.decode(&mut br).unwrap();
        assert_eq!((w, x), (-1, 0));
    }

    #[test]
    fn gets_codebook_by_index() {
        assert!(get_codebook(1).is_some());
        assert!(get_codebook(11).is_some());
        assert!(get_codebook(0).is_none());
        assert!(get_codebook(12).is_none());
    }
}

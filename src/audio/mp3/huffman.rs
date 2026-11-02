use crate::audio::mp3::bitstream::Mp3BitReader;
use std::io;

pub struct HuffmanTable {
    pub linbits: u8,
    pub max_len: u8,
}

impl HuffmanTable {
    pub const fn new(linbits: u8, max_len: u8) -> Self {
        HuffmanTable { linbits, max_len }
    }

    pub fn decode(&self, br: &mut Mp3BitReader) -> io::Result<(i16, i16)> {
        let mut code = 0u32;

        for len in 1..=self.max_len {
            let bit = br.read_bit()? as u32;
            code = (code << 1) | bit;

            if let Some((x, y)) = self.lookup(code, len) {
                let mut x_val = x;
                let mut y_val = y;

                if self.linbits > 0 {
                    if x_val == 15 {
                        let linbits_val = br.read_bits(self.linbits)? as i16;
                        x_val += linbits_val;
                    }
                    if y_val == 15 {
                        let linbits_val = br.read_bits(self.linbits)? as i16;
                        y_val += linbits_val;
                    }
                }

                if x_val != 0 && br.read_bit()? {
                    x_val = -x_val;
                }
                if y_val != 0 && br.read_bit()? {
                    y_val = -y_val;
                }

                return Ok((x_val, y_val));
            }
        }

        Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "Invalid MP3 Huffman code",
        ))
    }

    fn lookup(&self, code: u32, len: u8) -> Option<(i16, i16)> {
        match (code, len) {
            (0b0, 1) => Some((0, 0)),
            (0b01, 2) => Some((0, 1)),
            (0b10, 2) => Some((1, 0)),
            (0b11, 2) => Some((1, 1)),
            _ => None,
        }
    }
}

pub fn get_huffman_table(index: u8) -> Option<&'static HuffmanTable> {
    static TABLES: [HuffmanTable; 33] = [
        HuffmanTable::new(0, 0),
        HuffmanTable::new(0, 4),
        HuffmanTable::new(0, 4),
        HuffmanTable::new(0, 4),
        HuffmanTable::new(0, 4),
        HuffmanTable::new(0, 4),
        HuffmanTable::new(0, 6),
        HuffmanTable::new(0, 6),
        HuffmanTable::new(0, 6),
        HuffmanTable::new(0, 6),
        HuffmanTable::new(0, 6),
        HuffmanTable::new(0, 8),
        HuffmanTable::new(0, 8),
        HuffmanTable::new(0, 12),
        HuffmanTable::new(0, 12),
        HuffmanTable::new(0, 12),
        HuffmanTable::new(1, 13),
        HuffmanTable::new(2, 13),
        HuffmanTable::new(3, 13),
        HuffmanTable::new(4, 13),
        HuffmanTable::new(6, 13),
        HuffmanTable::new(8, 13),
        HuffmanTable::new(10, 13),
        HuffmanTable::new(13, 13),
        HuffmanTable::new(4, 12),
        HuffmanTable::new(5, 12),
        HuffmanTable::new(6, 12),
        HuffmanTable::new(7, 12),
        HuffmanTable::new(8, 12),
        HuffmanTable::new(9, 12),
        HuffmanTable::new(11, 12),
        HuffmanTable::new(13, 12),
        HuffmanTable::new(0, 4),
    ];

    if (index as usize) < TABLES.len() {
        Some(&TABLES[index as usize])
    } else {
        None
    }
}

pub fn decode_quad(br: &mut Mp3BitReader, table_select: bool) -> io::Result<[i16; 4]> {
    let code = br.read_bits(4)? as u8;

    let values = if table_select {
        match code {
            0b0000 => [0, 0, 0, 0],
            0b0001 => [0, 0, 0, 1],
            0b0010 => [0, 0, 1, 0],
            0b0011 => [0, 0, 1, 1],
            0b0100 => [0, 1, 0, 0],
            0b0101 => [0, 1, 0, 1],
            0b0110 => [0, 1, 1, 0],
            0b0111 => [0, 1, 1, 1],
            0b1000 => [1, 0, 0, 0],
            0b1001 => [1, 0, 0, 1],
            0b1010 => [1, 0, 1, 0],
            0b1011 => [1, 0, 1, 1],
            0b1100 => [1, 1, 0, 0],
            0b1101 => [1, 1, 0, 1],
            0b1110 => [1, 1, 1, 0],
            0b1111 => [1, 1, 1, 1],
            _ => unreachable!(),
        }
    } else {
        match code {
            0b0000 => [0, 0, 0, 0],
            0b0001 => [0, 0, 0, 1],
            0b0010 => [0, 0, 1, 0],
            0b0011 => [0, 1, 0, 0],
            0b0100 => [1, 0, 0, 0],
            0b0101 => [0, 0, 1, 1],
            0b0110 => [0, 1, 0, 1],
            0b0111 => [1, 0, 0, 1],
            0b1000 => [0, 1, 1, 0],
            0b1001 => [1, 0, 1, 0],
            0b1010 => [1, 1, 0, 0],
            0b1011 => [0, 1, 1, 1],
            0b1100 => [1, 0, 1, 1],
            0b1101 => [1, 1, 0, 1],
            0b1110 => [1, 1, 1, 0],
            0b1111 => [1, 1, 1, 1],
            _ => unreachable!(),
        }
    };

    let mut result = [0i16; 4];
    for (i, &val) in values.iter().enumerate() {
        result[i] = if val == 0 {
            0
        } else if br.read_bit()? {
            -1
        } else {
            1
        };
    }

    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gets_huffman_table() {
        assert!(get_huffman_table(0).is_some());
        assert!(get_huffman_table(32).is_some());
        assert!(get_huffman_table(33).is_none());
    }

    #[test]
    fn table_has_linbits() {
        let table = get_huffman_table(16).unwrap();
        assert_eq!(table.linbits, 1);
    }

    #[test]
    fn decodes_simple_huffman() {
        let data = [0b0_0000000];
        let mut br = Mp3BitReader::new(&data);

        let table = get_huffman_table(1).unwrap();
        let (x, y) = table.decode(&mut br).unwrap();
        assert_eq!((x, y), (0, 0));
    }
}

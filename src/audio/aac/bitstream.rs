use std::io;

pub struct BitReader<'a> {
    data: &'a [u8],
    byte_pos: usize,
    bit_pos: u8,
}

impl<'a> BitReader<'a> {
    pub fn new(data: &'a [u8]) -> Self {
        BitReader {
            data,
            byte_pos: 0,
            bit_pos: 0,
        }
    }

    pub fn read_bit(&mut self) -> io::Result<bool> {
        if self.byte_pos >= self.data.len() {
            return Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "BitReader: EOF",
            ));
        }

        let byte = self.data[self.byte_pos];
        let bit = (byte >> (7 - self.bit_pos)) & 1;

        self.bit_pos += 1;
        if self.bit_pos == 8 {
            self.bit_pos = 0;
            self.byte_pos += 1;
        }

        Ok(bit != 0)
    }

    pub fn read_bits(&mut self, n: u8) -> io::Result<u32> {
        if n > 32 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "Cannot read more than 32 bits",
            ));
        }

        let mut value = 0u32;
        for _ in 0..n {
            value = (value << 1) | (self.read_bit()? as u32);
        }
        Ok(value)
    }

    pub fn ignore_bits(&mut self, n: usize) -> io::Result<()> {
        let mut remaining = n;
        while remaining > 0 {
            let avail = 8 - self.bit_pos as usize;
            let take = avail.min(remaining);
            self.bit_pos += take as u8;
            remaining -= take;
            if self.bit_pos == 8 {
                self.bit_pos = 0;
                self.byte_pos += 1;
            }
        }
        if self.byte_pos > self.data.len() {
            return Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "BitReader: EOF",
            ));
        }
        Ok(())
    }

    pub fn byte_align(&mut self) {
        if self.bit_pos != 0 {
            self.bit_pos = 0;
            self.byte_pos += 1;
        }
    }

    pub fn bits_available(&self) -> usize {
        (self.data.len() - self.byte_pos) * 8 - self.bit_pos as usize
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_single_bit() {
        let data = [0b1010_0101];
        let mut reader = BitReader::new(&data);

        assert!(reader.read_bit().unwrap());
        assert!(!reader.read_bit().unwrap());
        assert!(reader.read_bit().unwrap());
        assert!(!reader.read_bit().unwrap());
    }

    #[test]
    fn reads_multiple_bits() {
        let data = [0b1010_0101, 0b1100_1100];
        let mut reader = BitReader::new(&data);

        assert_eq!(reader.read_bits(4).unwrap(), 0b1010);
        assert_eq!(reader.read_bits(8).unwrap(), 0b0101_1100);
    }

    #[test]
    fn byte_aligns_correctly() {
        let data = [0xFF, 0xAA];
        let mut reader = BitReader::new(&data);

        reader.read_bits(3).unwrap();
        reader.byte_align();

        assert_eq!(reader.byte_pos, 1);
        assert_eq!(reader.bit_pos, 0);
    }
}

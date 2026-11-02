use std::io;

pub struct Mp3BitReader<'a> {
    data: &'a [u8],
    byte_pos: usize,
    bit_pos: u8,
}

impl<'a> Mp3BitReader<'a> {
    pub fn new(data: &'a [u8]) -> Self {
        Mp3BitReader {
            data,
            byte_pos: 0,
            bit_pos: 0,
        }
    }

    pub fn read_bit(&mut self) -> io::Result<bool> {
        if self.byte_pos >= self.data.len() {
            return Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "MP3 bitstream EOF",
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

    pub fn skip_bits(&mut self, n: usize) -> io::Result<()> {
        for _ in 0..n {
            self.read_bit()?;
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

    pub fn position(&self) -> usize {
        self.byte_pos * 8 + self.bit_pos as usize
    }
}

#[derive(Debug, Clone, Copy)]
pub struct Granule {
    pub part2_3_length: u16,
    pub big_values: u16,
    pub global_gain: u8,
    pub scalefac_compress: u8,
    pub window_switching_flag: bool,
    pub block_type: u8,
    pub mixed_block_flag: bool,
    pub table_select: [u8; 3],
    pub subblock_gain: [u8; 3],
    pub region0_count: u8,
    pub region1_count: u8,
    pub preflag: bool,
    pub scalefac_scale: bool,
    pub count1table_select: bool,
}

impl Granule {
    pub fn decode(br: &mut Mp3BitReader) -> io::Result<Self> {
        let part2_3_length = br.read_bits(12)? as u16;
        let big_values = br.read_bits(9)? as u16;
        let global_gain = br.read_bits(8)? as u8;
        let scalefac_compress = br.read_bits(4)? as u8;
        let window_switching_flag = br.read_bit()?;

        let (
            block_type,
            mixed_block_flag,
            table_select,
            subblock_gain,
            region0_count,
            region1_count,
        ) = if window_switching_flag {
            let block_type = br.read_bits(2)? as u8;
            let mixed_block_flag = br.read_bit()?;
            let table_select = [br.read_bits(5)? as u8, br.read_bits(5)? as u8, 0];
            let subblock_gain = [
                br.read_bits(3)? as u8,
                br.read_bits(3)? as u8,
                br.read_bits(3)? as u8,
            ];

            #[allow(clippy::if_same_then_else)]
            let (region0_count, region1_count) = (8, 36);

            (
                block_type,
                mixed_block_flag,
                table_select,
                subblock_gain,
                region0_count,
                region1_count,
            )
        } else {
            let table_select = [
                br.read_bits(5)? as u8,
                br.read_bits(5)? as u8,
                br.read_bits(5)? as u8,
            ];
            let region0_count = br.read_bits(4)? as u8;
            let region1_count = br.read_bits(3)? as u8;

            (
                0,
                false,
                table_select,
                [0, 0, 0],
                region0_count,
                region1_count,
            )
        };

        let preflag = br.read_bit()?;
        let scalefac_scale = br.read_bit()?;
        let count1table_select = br.read_bit()?;

        Ok(Granule {
            part2_3_length,
            big_values,
            global_gain,
            scalefac_compress,
            window_switching_flag,
            block_type,
            mixed_block_flag,
            table_select,
            subblock_gain,
            region0_count,
            region1_count,
            preflag,
            scalefac_scale,
            count1table_select,
        })
    }
}

#[derive(Debug, Clone)]
pub struct SideInfo {
    pub main_data_begin: u16,
    pub private_bits: u8,
    pub scfsi: [[bool; 4]; 2],
    pub granules: [[Granule; 2]; 2],
}

impl SideInfo {
    pub fn decode(br: &mut Mp3BitReader, channels: usize) -> io::Result<Self> {
        let main_data_begin = br.read_bits(9)? as u16;

        let private_bits = if channels == 1 {
            br.read_bits(5)? as u8
        } else {
            br.read_bits(3)? as u8
        };

        let mut scfsi = [[false; 4]; 2];
        for scfsi_ch in scfsi.iter_mut().take(channels) {
            for scfsi_band in scfsi_ch.iter_mut() {
                *scfsi_band = br.read_bit()?;
            }
        }

        let mut granules = [[Granule {
            part2_3_length: 0,
            big_values: 0,
            global_gain: 0,
            scalefac_compress: 0,
            window_switching_flag: false,
            block_type: 0,
            mixed_block_flag: false,
            table_select: [0, 0, 0],
            subblock_gain: [0, 0, 0],
            region0_count: 0,
            region1_count: 0,
            preflag: false,
            scalefac_scale: false,
            count1table_select: false,
        }; 2]; 2];

        for granule_gr in &mut granules {
            for granule_ch in granule_gr.iter_mut().take(channels) {
                *granule_ch = Granule::decode(br)?;
            }
        }

        Ok(SideInfo {
            main_data_begin,
            private_bits,
            scfsi,
            granules,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_single_bit() {
        let data = [0b1010_0101];
        let mut br = Mp3BitReader::new(&data);

        assert!(br.read_bit().unwrap());
        assert!(!br.read_bit().unwrap());
        assert!(br.read_bit().unwrap());
        assert!(!br.read_bit().unwrap());
    }

    #[test]
    fn reads_multiple_bits() {
        let data = [0b1010_0101, 0b1100_1100];
        let mut br = Mp3BitReader::new(&data);

        assert_eq!(br.read_bits(4).unwrap(), 0b1010);
        assert_eq!(br.read_bits(8).unwrap(), 0b0101_1100);
    }

    #[test]
    fn byte_aligns_correctly() {
        let data = [0xFF, 0xAA];
        let mut br = Mp3BitReader::new(&data);

        br.read_bits(3).unwrap();
        br.byte_align();

        assert_eq!(br.byte_pos, 1);
        assert_eq!(br.bit_pos, 0);
    }

    #[test]
    fn tracks_position() {
        let data = [0xFF, 0xAA];
        let mut br = Mp3BitReader::new(&data);

        assert_eq!(br.position(), 0);
        br.read_bits(5).unwrap();
        assert_eq!(br.position(), 5);
    }
}

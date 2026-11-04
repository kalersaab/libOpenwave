use std::io;

pub struct Mp3BitReader<'a> {
    data: &'a [u8],
    byte_pos: usize,
    cache: u32,
    cache_bits: i32,
    bits_read: usize,
}

impl<'a> Mp3BitReader<'a> {
    pub fn new(data: &'a [u8]) -> Self {
        let mut reader = Mp3BitReader {
            data,
            byte_pos: 0,
            cache: 0,
            cache_bits: 0,
            bits_read: 0,
        };
        reader.fill_cache();
        reader
    }

    pub fn new_at_bit_offset(data: &'a [u8], bit_offset: usize) -> Self {
        let byte_pos = bit_offset / 8;
        let _bit_pos = (bit_offset % 8) as u8;
        let mut reader = Mp3BitReader {
            data,
            byte_pos,
            cache: 0,
            cache_bits: 0,
            bits_read: bit_offset,
        };
        reader.fill_cache();
        reader
    }

    fn fill_cache(&mut self) {
        while self.cache_bits <= 24 && self.byte_pos < self.data.len() {
            let byte = self.data[self.byte_pos];
            self.byte_pos += 1;
            self.cache = (self.cache << 8) | (byte as u32);
            self.cache_bits += 8;
        }
    }

    pub fn read_bit(&mut self) -> io::Result<bool> {
        if self.cache_bits <= 0 {
            self.fill_cache();
            if self.cache_bits <= 0 {
                return Err(io::Error::new(
                    io::ErrorKind::UnexpectedEof,
                    "MP3 bitstream EOF",
                ));
            }
        }

        self.cache_bits -= 1;
        self.bits_read += 1;
        let bit = (self.cache >> self.cache_bits) & 1;
        Ok(bit != 0)
    }

    pub fn read_bits(&mut self, n: u8) -> io::Result<u32> {
        if n > 32 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "Cannot read more than 32 bits",
            ));
        }

        if n == 0 {
            return Ok(0);
        }

        while self.cache_bits < n as i32 {
            self.fill_cache();
            if self.cache_bits < n as i32 {
                return Err(io::Error::new(
                    io::ErrorKind::UnexpectedEof,
                    "Not enough bits available",
                ));
            }
        }

        self.cache_bits -= n as i32;
        self.bits_read += n as usize;
        let value = (self.cache >> self.cache_bits) & ((1 << n) - 1);
        Ok(value)
    }

    pub fn peek_bits(&self, n: u8) -> u32 {
        if self.cache_bits < n as i32 {
            return 0;
        }
        (self.cache >> (self.cache_bits - n as i32)) & ((1 << n) - 1)
    }

    pub fn skip_bits(&mut self, n: usize) -> io::Result<()> {
        for _ in 0..n {
            self.read_bit()?;
        }
        Ok(())
    }

    pub fn byte_align(&mut self) {
        let bits_to_skip = (self.cache_bits % 8) as usize;
        if bits_to_skip > 0 {
            let _ = self.skip_bits(bits_to_skip);
        }
    }

    pub fn bits_available(&self) -> usize {
        let remaining_bytes = self.data.len().saturating_sub(self.byte_pos);
        (remaining_bytes * 8) + self.cache_bits.max(0) as usize
    }

    pub fn position_bits(&self) -> usize {
        self.bits_read
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

pub const SFB_LONG_44100: [u16; 23] = [
    0, 4, 8, 12, 16, 20, 24, 30, 36, 44, 52, 62, 74, 90, 110, 134, 162, 196, 238, 288, 342, 418,
    576,
];

pub const SFB_SHORT_44100: [u16; 14] = [0, 4, 8, 12, 16, 22, 30, 40, 52, 66, 84, 106, 136, 192];

pub fn get_sfb_long(sample_rate: u32) -> &'static [u16] {
    match sample_rate {
        44100 => &SFB_LONG_44100,
        _ => &SFB_LONG_44100,
    }
}

pub fn get_sfb_short(sample_rate: u32) -> &'static [u16] {
    match sample_rate {
        44100 => &SFB_SHORT_44100,
        _ => &SFB_SHORT_44100,
    }
}

pub fn get_slen(scalefac_compress: u8) -> (u8, u8) {
    let _slen1 = scalefac_compress >> 4;
    let _slen2 = scalefac_compress & 0xF;

    match scalefac_compress {
        0..=15 => ((scalefac_compress / 5), (scalefac_compress % 5)),
        _ => (0, 0),
    }
}

pub struct BitReservoir {
    buffer: Vec<u8>,
    max_size: usize,
}

impl BitReservoir {
    pub fn new() -> Self {
        BitReservoir {
            buffer: Vec::with_capacity(4096),
            max_size: 4096,
        }
    }

    pub fn append(&mut self, data: &[u8]) {
        self.buffer.extend_from_slice(data);

        if self.buffer.len() > self.max_size {
            let excess = self.buffer.len() - self.max_size;
            self.buffer.drain(0..excess);
        }
    }

    pub fn get_main_data(&self, main_data_begin: usize, length: usize) -> Option<&[u8]> {
        if main_data_begin > self.buffer.len() {
            return None;
        }

        let start = self.buffer.len() - main_data_begin;
        let end = (start + length).min(self.buffer.len());

        if start >= end {
            return None;
        }

        Some(&self.buffer[start..end])
    }

    pub fn len(&self) -> usize {
        self.buffer.len()
    }

    pub fn is_empty(&self) -> bool {
        self.buffer.is_empty()
    }
}

impl Default for BitReservoir {
    fn default() -> Self {
        Self::new()
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
        let data = [0xFF, 0xAA, 0xBB, 0xCC];
        let mut br = Mp3BitReader::new(&data);

        br.read_bits(3).unwrap();
        br.byte_align();

        assert_eq!(br.position_bits(), 8);
    }

    #[test]
    fn tracks_position() {
        let data = [0xFF, 0xAA];
        let mut br = Mp3BitReader::new(&data);

        assert_eq!(br.position_bits(), 0);
        br.read_bits(5).unwrap();
        assert_eq!(br.position_bits(), 5);
    }
}

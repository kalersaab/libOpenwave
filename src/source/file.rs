use super::MediaSource;
use std::fs::File;
use std::io::{self, Read, Seek, SeekFrom};
use std::path::Path;

pub struct FileSource {
    file: File,
    length: u64,
}

impl FileSource {
    pub fn open(path: impl AsRef<Path>) -> io::Result<Self> {
        let file = File::open(path)?;
        let length = file.metadata()?.len();

        Ok(Self { file, length })
    }
}

impl MediaSource for FileSource {
    fn len(&self) -> u64 {
        self.length
    }

    fn read_at(&mut self, offset: u64, buffer: &mut [u8]) -> io::Result<usize> {
        if offset >= self.length || buffer.is_empty() {
            return Ok(0);
        }

        self.file.seek(SeekFrom::Start(offset))?;
        let remaining = self.length - offset;
        let count = buffer.len().min(remaining as usize);

        self.file.read(&mut buffer[..count])
    }
}

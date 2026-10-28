pub mod file;

use std::io;
use std::path::Path;

pub trait MediaSource: Send {
    fn len(&self) -> u64;
    fn is_empty(&self) -> bool {
        self.len() == 0
    }

    fn read_at(&mut self, offset: u64, buffer: &mut [u8]) -> io::Result<usize>;
}

pub fn open_file(path: impl AsRef<Path>) -> io::Result<file::FileSource> {
    file::FileSource::open(path)
}

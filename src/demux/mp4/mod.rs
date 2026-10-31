pub mod parser;
pub mod reader;
pub mod types;

pub use reader::Mp4Reader;
pub use types::{Mp4Sample, Mp4Track, TrackType};

use std::io;

#[derive(Debug)]
pub enum Mp4Error {
    Io(io::Error),
    InvalidFormat(&'static str),
    UnsupportedFeature(&'static str),
    MissingBox(&'static str),
}

impl std::fmt::Display for Mp4Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Mp4Error::Io(e) => write!(f, "MP4 I/O error: {}", e),
            Mp4Error::InvalidFormat(msg) => write!(f, "MP4 invalid format: {}", msg),
            Mp4Error::UnsupportedFeature(msg) => write!(f, "MP4 unsupported feature: {}", msg),
            Mp4Error::MissingBox(box_type) => write!(f, "MP4 missing required box: {}", box_type),
        }
    }
}

impl std::error::Error for Mp4Error {}

impl From<io::Error> for Mp4Error {
    fn from(e: io::Error) -> Self {
        Mp4Error::Io(e)
    }
}

pub type Mp4Result<T> = Result<T, Mp4Error>;

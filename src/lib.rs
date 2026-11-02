#![allow(clippy::needless_range_loop)]
#![allow(clippy::type_complexity)]
#![allow(clippy::if_same_then_else)]
pub mod audio;
pub mod core;
pub mod demux;
pub mod source;
pub mod sync;
pub mod video;

pub use demux::wav::{WavError, WavFile, WavInfo};
pub use source::{MediaSource, open_file};

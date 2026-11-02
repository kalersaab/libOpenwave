#![allow(clippy::needless_range_loop)]
#![allow(clippy::type_complexity)]
#![allow(clippy::if_same_then_else)]
pub mod audio;
pub mod core;
pub mod demux;
pub mod source;
pub mod sync;
pub mod video;

pub use audio::engine::AudioEngine;
pub use audio::mp3::Mp3Decoder;
pub use demux::mp3::{Mp3FrameHeader, Mp3Reader};
pub use demux::wav::{WavError, WavFile, WavInfo};
pub use source::{MediaSource, open_file};

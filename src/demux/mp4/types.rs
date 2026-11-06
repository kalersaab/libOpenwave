use std::fmt;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BoxType([u8; 4]);

impl BoxType {
    pub const FTYP: BoxType = BoxType(*b"ftyp");
    pub const MOOV: BoxType = BoxType(*b"moov");
    pub const MDAT: BoxType = BoxType(*b"mdat");
    pub const MVHD: BoxType = BoxType(*b"mvhd");
    pub const TRAK: BoxType = BoxType(*b"trak");
    pub const TKHD: BoxType = BoxType(*b"tkhd");
    pub const MDIA: BoxType = BoxType(*b"mdia");
    pub const MDHD: BoxType = BoxType(*b"mdhd");
    pub const HDLR: BoxType = BoxType(*b"hdlr");
    pub const MINF: BoxType = BoxType(*b"minf");
    pub const STBL: BoxType = BoxType(*b"stbl");
    pub const STSD: BoxType = BoxType(*b"stsd");
    pub const STTS: BoxType = BoxType(*b"stts");
    pub const CTTS: BoxType = BoxType(*b"ctts");
    pub const STSC: BoxType = BoxType(*b"stsc");
    pub const STSZ: BoxType = BoxType(*b"stsz");
    pub const STCO: BoxType = BoxType(*b"stco");
    pub const CO64: BoxType = BoxType(*b"co64");
    pub const STSS: BoxType = BoxType(*b"stss");
    pub const AVC1: BoxType = BoxType(*b"avc1");
    pub const AVCC: BoxType = BoxType(*b"avcC");
    pub const MP4A: BoxType = BoxType(*b"mp4a");
    pub const ESDS: BoxType = BoxType(*b"esds");
    pub const MVEX: BoxType = BoxType(*b"mvex");
    pub const TREX: BoxType = BoxType(*b"trex");
    pub const MOOF: BoxType = BoxType(*b"moof");
    pub const MFHD: BoxType = BoxType(*b"mfhd");
    pub const TRAF: BoxType = BoxType(*b"traf");
    pub const TFHD: BoxType = BoxType(*b"tfhd");
    pub const TFDT: BoxType = BoxType(*b"tfdt");
    pub const TRUN: BoxType = BoxType(*b"trun");

    pub fn new(bytes: &[u8; 4]) -> Self {
        BoxType(*bytes)
    }

    pub fn as_bytes(&self) -> &[u8; 4] {
        &self.0
    }

    pub fn as_str(&self) -> String {
        String::from_utf8_lossy(&self.0).to_string()
    }
}

impl fmt::Display for BoxType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

#[derive(Debug, Clone)]
pub struct BoxHeader {
    pub box_type: BoxType,
    pub size: u64,
    pub header_size: u64,
}

impl BoxHeader {
    /// Size of the box payload, excluding its header.
    pub fn content_size(&self) -> u64 {
        self.size - self.header_size
    }
}

/// Metadata from the `ftyp` box.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileType {
    pub major_brand: [u8; 4],
    pub minor_version: u32,
    pub compatible_brands: Vec<[u8; 4]>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrackType {
    Video,
    Audio,
    Subtitle,
    Unknown,
}

#[derive(Debug, Clone)]
pub struct Mp4Track {
    pub track_id: u32,
    pub track_type: TrackType,
    pub timescale: u32,
    pub duration: u64,
    pub codec_private_data: Vec<u8>,
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub sample_rate: Option<u32>,
    pub channels: Option<u16>,
}

#[derive(Debug, Clone)]
pub struct Mp4Sample {
    pub data: Vec<u8>,
    /// Legacy alias for [`Mp4Sample::dts`], kept for compatibility.
    pub timestamp: u64,
    /// Decode timestamp (DTS) in track timescale units.
    pub dts: u64,
    /// Presentation timestamp (PTS) in track timescale units.
    ///
    /// This is `dts + composition_offset` and may be negative for version 1
    /// `ctts` boxes containing negative offsets.
    pub pts: i64,
    /// Sample duration in track timescale units.
    pub duration: u64,
    pub is_keyframe: bool,
    pub track_id: u32,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct TrexDefaults {
    pub sample_description_index: u32,
    pub sample_duration: u32,
    pub sample_size: u32,
    pub sample_flags: u32,
}

#[derive(Debug, Default)]
pub struct SampleTableBox {
    pub chunk_offsets: Vec<u64>,
    pub sample_sizes: Vec<u32>,
    pub sample_to_chunk: Vec<SampleToChunk>,
    pub time_to_sample: Vec<TimeToSample>,
    pub composition_offsets: Option<Vec<CompositionOffset>>,
    pub sync_samples: Option<Vec<u32>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SampleToChunk {
    pub first_chunk: u32,
    pub samples_per_chunk: u32,
    pub sample_description_index: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TimeToSample {
    pub sample_count: u32,
    pub sample_delta: u32,
}

/// Composition time offset (`ctts`) entry.
///
/// The offset is stored as `i64` so that both version 0 (unsigned `u32`) and
/// version 1 (signed `i32`) boxes can be represented without loss.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CompositionOffset {
    pub sample_count: u32,
    pub sample_offset: i64,
}

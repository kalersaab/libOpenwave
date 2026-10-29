#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VideoTimestamp {
    pub pts_us: u64,
}
pub mod bitstream;
pub mod h264;

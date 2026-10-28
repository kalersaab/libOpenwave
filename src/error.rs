
use thiserror::Error;

#[derive(Debug, Error)]
pub enum OpenWaveError {
    #[error("LibVLC could not be loaded: {0}")]
    BackendUnavailable(String),

    #[error("Failed to initialize the media engine")]
    InitializationFailed,

    #[error("Failed to create the media player")]
    PlayerCreationFailed,

    #[error("Invalid media path: {0}")]
    InvalidPath(String),

    #[error("Failed to open media")]
    MediaOpenFailed,

    #[error("Playback operation failed: {0}")]
    PlaybackFailed(&'static str),
}

pub type Result<T> = std::result::Result<T, OpenWaveError>;

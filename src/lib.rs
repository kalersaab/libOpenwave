
mod error;
mod player;
mod backend;

pub use error::{OpenWaveError, Result};
pub use player::{MediaInfo, Player, PlayerState};

/// Version of the libopenwave public API.
pub const API_VERSION: &str = env!("CARGO_PKG_VERSION");

/// Returns the libopenwave version.
pub fn version() -> &'static str {
    API_VERSION
}

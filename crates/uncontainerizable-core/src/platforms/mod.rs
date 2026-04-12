//! Platform dispatch.
//!
//! Currently a stub implementation that fails with `UnsupportedPlatform` on
//! every target. Real per-platform spawning lands with the Darwin, Linux,
//! and Windows modules as they come online.

use crate::app::{App, ContainOptions};
use crate::container::Container;
use crate::error::{Error, Result};

pub async fn spawn(
    _app: &App,
    _command: &str,
    _opts: ContainOptions,
) -> Result<Box<dyn Container>> {
    Err(Error::UnsupportedPlatform(format!(
        "{} (spawning not yet implemented)",
        std::env::consts::OS
    )))
}

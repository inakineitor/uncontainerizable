//! Platform dispatch.
//!
//! `spawn` cfg-dispatches to the implementation module for the current
//! target. Targets without an implementation fall through to
//! `UnsupportedPlatform`.

use crate::app::{App, ContainOptions};
use crate::container::Container;
#[cfg(not(any(target_os = "macos", target_os = "linux", windows)))]
use crate::error::Error;
use crate::error::Result;

#[cfg(target_os = "macos")]
pub mod darwin;

#[cfg(target_os = "linux")]
pub mod linux;

#[cfg(windows)]
pub mod win32;

#[cfg(target_os = "macos")]
pub async fn spawn(app: &App, command: &str, opts: ContainOptions) -> Result<Box<dyn Container>> {
    darwin::spawn(app, command, opts).await
}

#[cfg(target_os = "linux")]
pub async fn spawn(app: &App, command: &str, opts: ContainOptions) -> Result<Box<dyn Container>> {
    linux::spawn(app, command, opts).await
}

#[cfg(windows)]
pub async fn spawn(app: &App, command: &str, opts: ContainOptions) -> Result<Box<dyn Container>> {
    win32::spawn(app, command, opts).await
}

#[cfg(not(any(target_os = "macos", target_os = "linux", windows)))]
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

//! Graceful process lifecycle for programs that can't be containerized.
//!
//! See `documents/development-plan.md` in the repo root for design.

pub const VERSION: &str = env!("CARGO_PKG_VERSION");

pub mod adapter;
pub mod app;
pub mod container;
pub mod error;
pub mod identity;
pub mod platforms;
pub mod probe;

pub use adapter::Adapter;
pub use app::{App, ContainOptions};
pub use container::{
    Container, DestroyOptions, DestroyResult, QuitOptions, QuitResult, StageResult,
};
pub use error::{AdapterError, Error, PlatformError, ProbeError, Result, StageError};
pub use probe::{Probe, SupportedPlatform};

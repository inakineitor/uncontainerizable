//! Graceful process lifecycle for programs that can't be containerized.
//!
//! See the development plan in `documents/development-plan.md` for design.

pub const VERSION: &str = env!("CARGO_PKG_VERSION");

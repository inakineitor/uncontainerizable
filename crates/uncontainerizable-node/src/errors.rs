//! Conversion from core errors to `napi::Error` so the thrown JS error
//! carries a meaningful message and a stable `code` string.

use napi::{Error, Status};
use uncontainerizable_core::Error as CoreError;

pub fn to_napi(err: CoreError) -> Error {
    let (status, code) = classify(&err);
    let reason = err.to_string();
    let mut napi_err = Error::new(status, reason);
    napi_err.reason = format!("{code}: {}", napi_err.reason);
    napi_err
}

fn classify(err: &CoreError) -> (Status, &'static str) {
    match err {
        CoreError::UnsupportedPlatform(_) => (Status::GenericFailure, "UNSUPPORTED_PLATFORM"),
        CoreError::Spawn { .. } => (Status::InvalidArg, "SPAWN_FAILED"),
        CoreError::Preempt { .. } => (Status::GenericFailure, "PREEMPT_FAILED"),
        CoreError::InvalidIdentity(_) => (Status::InvalidArg, "INVALID_IDENTITY"),
        CoreError::Probe(_) => (Status::GenericFailure, "PROBE_FAILED"),
        CoreError::AlreadyDestroyed => (Status::InvalidArg, "ALREADY_DESTROYED"),
        CoreError::Platform(_) => (Status::GenericFailure, "PLATFORM_ERROR"),
        CoreError::Stage(_) => (Status::GenericFailure, "STAGE_ERROR"),
        #[cfg(target_os = "macos")]
        CoreError::Bundle(_) => (Status::InvalidArg, "BUNDLE_ERROR"),
    }
}

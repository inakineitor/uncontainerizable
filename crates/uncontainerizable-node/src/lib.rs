#![deny(clippy::all)]

use napi_derive::napi;

#[napi]
pub fn core_version() -> String {
    uncontainerizable_core::VERSION.to_string()
}

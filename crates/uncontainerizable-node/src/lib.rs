#![deny(clippy::all)]

use napi_derive::napi;

mod adapter_bridge;
mod app;
mod container;
mod errors;
mod types;

pub use app::NodeApp;
pub use container::NodeContainer;
pub use types::{
    JsContainOptions, JsDestroyOptions, JsDestroyResult, JsProbe, JsQuitOptions, JsQuitResult,
    JsStageResult,
};

#[napi]
pub fn core_version() -> String {
    uncontainerizable_core::VERSION.to_string()
}

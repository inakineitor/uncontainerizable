//! `#[napi(object)]` shapes that mirror the core types across the napi
//! boundary. Kept separate from `app.rs` and `container.rs` so both can
//! import without a circular dependency.

use std::collections::HashMap;

use napi_derive::napi;
use uncontainerizable_core::{
    DestroyResult as CoreDestroyResult, Probe as CoreProbe, QuitResult as CoreQuitResult,
    StageResult as CoreStageResult, SupportedPlatform,
};

#[napi(object)]
pub struct JsProbe {
    pub pid: u32,
    pub bundle_id: Option<String>,
    pub executable_path: Option<String>,
    pub platform: String,
    pub captured_at_ms: f64,
}

impl From<&CoreProbe> for JsProbe {
    fn from(p: &CoreProbe) -> Self {
        Self {
            pid: p.pid,
            bundle_id: p.bundle_id.clone(),
            executable_path: p
                .executable_path
                .as_ref()
                .map(|p| p.to_string_lossy().into_owned()),
            platform: match p.platform {
                SupportedPlatform::Linux => "linux".into(),
                SupportedPlatform::Darwin => "darwin".into(),
                SupportedPlatform::Windows => "win32".into(),
            },
            captured_at_ms: p.captured_at_ms as f64,
        }
    }
}

// `object_to_js = false` because this is used as input only and the
// `adapters` field holds TSFN references that have no ToNapiValue impl.
#[napi(object, object_to_js = false)]
pub struct JsContainOptions {
    pub args: Option<Vec<String>>,
    pub env: Option<HashMap<String, String>>,
    pub cwd: Option<String>,
    pub identity: Option<String>,
    pub darwin_tag_argv0: Option<bool>,
    pub adapters: Option<Vec<crate::adapter_bridge::JsAdapter>>,
}

#[napi(object)]
pub struct JsQuitOptions {
    /// Map of stage name to timeout in milliseconds.
    pub stage_timeouts_ms: Option<HashMap<String, u32>>,
    pub skip_stages: Option<Vec<String>>,
}

#[napi(object)]
pub struct JsDestroyOptions {
    pub quit: Option<JsQuitOptions>,
}

#[napi(object)]
pub struct JsStageResult {
    pub stage_name: String,
    pub index: u32,
    pub exited: bool,
    pub is_terminal: bool,
}

impl From<&CoreStageResult> for JsStageResult {
    fn from(r: &CoreStageResult) -> Self {
        Self {
            stage_name: r.stage_name.clone(),
            index: r.index as u32,
            exited: r.exited,
            is_terminal: r.is_terminal,
        }
    }
}

#[napi(object)]
pub struct JsQuitResult {
    pub exited_at_stage: Option<String>,
    pub reached_terminal_stage: bool,
    pub stage_results: Vec<JsStageResult>,
    pub adapter_errors: Vec<String>,
}

impl From<&CoreQuitResult> for JsQuitResult {
    fn from(r: &CoreQuitResult) -> Self {
        Self {
            exited_at_stage: r.exited_at_stage.clone(),
            reached_terminal_stage: r.reached_terminal_stage,
            stage_results: r.stage_results.iter().map(Into::into).collect(),
            adapter_errors: r.adapter_errors.clone(),
        }
    }
}

#[napi(object)]
pub struct JsDestroyResult {
    pub quit: JsQuitResult,
    /// Error messages collected during teardown. Empty on a clean destroy.
    pub errors: Vec<String>,
}

impl From<&CoreDestroyResult> for JsDestroyResult {
    fn from(r: &CoreDestroyResult) -> Self {
        Self {
            quit: JsQuitResult::from(&r.quit),
            errors: r.errors.iter().map(|e| e.to_string()).collect(),
        }
    }
}

//! `NodeContainer`: napi-exposed handle for a spawned container.
//!
//! Holds a `Box<dyn uncontainerizable_core::Container>` behind an async
//! mutex so repeated `.quit()` / `.destroy()` calls from JS serialize
//! safely across the napi boundary.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use napi::{Error, Status};
use napi_derive::napi;
use tokio::sync::Mutex;
use uncontainerizable_core::{
    Container, DestroyOptions as CoreDestroyOptions, QuitOptions as CoreQuitOptions,
};

use crate::errors::to_napi;
use crate::types::{JsDestroyOptions, JsDestroyResult, JsProbe, JsQuitOptions, JsQuitResult};

#[napi]
pub struct NodeContainer {
    inner: Arc<Mutex<Box<dyn Container>>>,
}

impl NodeContainer {
    pub(crate) fn wrap(container: Box<dyn Container>) -> Self {
        Self {
            inner: Arc::new(Mutex::new(container)),
        }
    }
}

#[napi]
impl NodeContainer {
    #[napi(getter)]
    pub async fn pid(&self) -> u32 {
        let guard = self.inner.lock().await;
        guard.pid()
    }

    #[napi(getter)]
    pub async fn probe(&self) -> JsProbe {
        let guard = self.inner.lock().await;
        JsProbe::from(guard.probe())
    }

    /// Run the staged quit ladder. Adapter errors surface inside
    /// `adapter_errors`; stage errors propagate as a thrown napi error.
    #[napi]
    pub async fn quit(&self, opts: Option<JsQuitOptions>) -> napi::Result<JsQuitResult> {
        let core_opts = to_core_quit_opts(opts);
        let mut guard = self.inner.lock().await;
        let result = guard.quit(core_opts).await.map_err(to_napi)?;
        Ok(JsQuitResult::from(&result))
    }

    /// `quit` plus release platform resources. Does not throw: every
    /// failure is collected into `errors`.
    #[napi]
    pub async fn destroy(&self, opts: Option<JsDestroyOptions>) -> JsDestroyResult {
        let core_opts = to_core_destroy_opts(opts);
        let mut guard = self.inner.lock().await;
        let result = guard.destroy(core_opts).await;
        JsDestroyResult::from(&result)
    }

    #[napi]
    pub async fn members(&self) -> Vec<u32> {
        let guard = self.inner.lock().await;
        guard.members().await
    }

    #[napi(js_name = "isEmpty")]
    pub async fn is_empty(&self) -> napi::Result<bool> {
        let guard = self.inner.lock().await;
        guard
            .is_empty()
            .await
            .map_err(|e| Error::new(Status::GenericFailure, format!("is_empty failed: {e}")))
    }
}

fn to_core_quit_opts(opts: Option<JsQuitOptions>) -> CoreQuitOptions {
    let Some(opts) = opts else {
        return CoreQuitOptions::default();
    };
    let stage_timeouts: HashMap<String, Duration> = opts
        .stage_timeouts_ms
        .unwrap_or_default()
        .into_iter()
        .map(|(k, v)| (k, Duration::from_millis(u64::from(v))))
        .collect();
    CoreQuitOptions {
        stage_timeouts,
        skip_stages: opts.skip_stages.unwrap_or_default(),
    }
}

fn to_core_destroy_opts(opts: Option<JsDestroyOptions>) -> CoreDestroyOptions {
    let Some(opts) = opts else {
        return CoreDestroyOptions::default();
    };
    CoreDestroyOptions {
        quit: to_core_quit_opts(opts.quit),
    }
}

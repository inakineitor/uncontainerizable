#![allow(
    clippy::type_complexity,
    reason = "TSFN generics are inlined (not aliased) so napi's .d.ts emitter resolves them to proper function signatures on the TypeScript side"
)]

//! JS-to-Rust adapter bridge.
//!
//! Takes a `JsAdapter` (a `#[napi(object)]` whose function fields are
//! thread-safe function references into V8) and builds a `DynamicAdapter`
//! that implements `uncontainerizable_core::Adapter`. Each Rust hook
//! marshals its args across the napi boundary via `call_async`, awaits
//! the returned Promise, and maps failures to `AdapterError::Callback`.
//!
//! Every field except `name` and `matches` is optional on the JS side.
//! The TS wrapper normalizes user adapters (which may declare either
//! sync or async methods) to always return Promises before handing them
//! here, so the hook signatures below can hard-code `Promise<_>` returns.
//!
//! `CalleeHandled = false` gives us the `call_async(T) -> Result<Return>`
//! form (the simpler one without explicit JS-side error forwarding);
//! `Weak = false` keeps the JS function alive as long as the TSFN handle
//! is alive; `MaxQueueSize = 0` is the unlimited queue.

use async_trait::async_trait;
use napi::Status;
use napi::bindgen_prelude::Promise;
use napi::threadsafe_function::ThreadsafeFunction;
use napi_derive::napi;
use uncontainerizable_core::{
    Adapter, AdapterError, Container, Probe as CoreProbe, QuitResult as CoreQuitResult,
    StageResult as CoreStageResult,
};

use crate::types::{JsProbe, JsQuitResult, JsStageResult};

#[napi(object, object_to_js = false)]
pub struct JsAdapter {
    pub name: String,
    pub matches: ThreadsafeFunction<JsProbe, Promise<bool>, JsProbe, Status, false, false, 0>,
    pub before_quit:
        Option<ThreadsafeFunction<JsProbe, Promise<()>, JsProbe, Status, false, false, 0>>,
    pub before_stage: Option<
        ThreadsafeFunction<
            (JsProbe, String),
            Promise<()>,
            (JsProbe, String),
            Status,
            false,
            false,
            0,
        >,
    >,
    pub after_stage: Option<
        ThreadsafeFunction<
            (JsProbe, JsStageResult),
            Promise<()>,
            (JsProbe, JsStageResult),
            Status,
            false,
            false,
            0,
        >,
    >,
    pub after_quit: Option<
        ThreadsafeFunction<
            (JsProbe, JsQuitResult),
            Promise<()>,
            (JsProbe, JsQuitResult),
            Status,
            false,
            false,
            0,
        >,
    >,
    pub clear_crash_state:
        Option<ThreadsafeFunction<JsProbe, Promise<()>, JsProbe, Status, false, false, 0>>,
}

pub struct DynamicAdapter {
    name: String,
    matches: ThreadsafeFunction<JsProbe, Promise<bool>, JsProbe, Status, false, false, 0>,
    before_quit: Option<ThreadsafeFunction<JsProbe, Promise<()>, JsProbe, Status, false, false, 0>>,
    before_stage: Option<
        ThreadsafeFunction<
            (JsProbe, String),
            Promise<()>,
            (JsProbe, String),
            Status,
            false,
            false,
            0,
        >,
    >,
    after_stage: Option<
        ThreadsafeFunction<
            (JsProbe, JsStageResult),
            Promise<()>,
            (JsProbe, JsStageResult),
            Status,
            false,
            false,
            0,
        >,
    >,
    after_quit: Option<
        ThreadsafeFunction<
            (JsProbe, JsQuitResult),
            Promise<()>,
            (JsProbe, JsQuitResult),
            Status,
            false,
            false,
            0,
        >,
    >,
    clear_crash_state:
        Option<ThreadsafeFunction<JsProbe, Promise<()>, JsProbe, Status, false, false, 0>>,
}

impl From<JsAdapter> for DynamicAdapter {
    fn from(js: JsAdapter) -> Self {
        Self {
            name: js.name,
            matches: js.matches,
            before_quit: js.before_quit,
            before_stage: js.before_stage,
            after_stage: js.after_stage,
            after_quit: js.after_quit,
            clear_crash_state: js.clear_crash_state,
        }
    }
}

#[async_trait]
impl Adapter for DynamicAdapter {
    fn name(&self) -> &str {
        &self.name
    }

    async fn matches(&self, probe: &CoreProbe) -> bool {
        // Any failure crossing the boundary is treated as "does not
        // match": silently dropping the adapter is safer than letting a
        // misbehaving JS callback block the quit ladder.
        match self.matches.call_async(JsProbe::from(probe)).await {
            Ok(promise) => promise.await.unwrap_or(false),
            Err(_) => false,
        }
    }

    async fn before_quit(
        &self,
        probe: &CoreProbe,
        _container: &dyn Container,
    ) -> Result<(), AdapterError> {
        let Some(tsfn) = self.before_quit.as_ref() else {
            return Ok(());
        };
        let promise = tsfn
            .call_async(JsProbe::from(probe))
            .await
            .map_err(adapter_err)?;
        promise.await.map_err(adapter_err).map(|_: ()| ())
    }

    async fn before_stage(
        &self,
        probe: &CoreProbe,
        stage_name: &str,
        _container: &dyn Container,
    ) -> Result<(), AdapterError> {
        let Some(tsfn) = self.before_stage.as_ref() else {
            return Ok(());
        };
        let args = (JsProbe::from(probe), stage_name.to_string());
        let promise = tsfn.call_async(args).await.map_err(adapter_err)?;
        promise.await.map_err(adapter_err).map(|_: ()| ())
    }

    async fn after_stage(
        &self,
        probe: &CoreProbe,
        stage_result: &CoreStageResult,
        _container: &dyn Container,
    ) -> Result<(), AdapterError> {
        let Some(tsfn) = self.after_stage.as_ref() else {
            return Ok(());
        };
        let args = (JsProbe::from(probe), JsStageResult::from(stage_result));
        let promise = tsfn.call_async(args).await.map_err(adapter_err)?;
        promise.await.map_err(adapter_err).map(|_: ()| ())
    }

    async fn after_quit(
        &self,
        probe: &CoreProbe,
        quit_result: &CoreQuitResult,
        _container: &dyn Container,
    ) -> Result<(), AdapterError> {
        let Some(tsfn) = self.after_quit.as_ref() else {
            return Ok(());
        };
        let args = (JsProbe::from(probe), JsQuitResult::from(quit_result));
        let promise = tsfn.call_async(args).await.map_err(adapter_err)?;
        promise.await.map_err(adapter_err).map(|_: ()| ())
    }

    async fn clear_crash_state(&self, probe: &CoreProbe) -> Result<(), AdapterError> {
        let Some(tsfn) = self.clear_crash_state.as_ref() else {
            return Ok(());
        };
        let promise = tsfn
            .call_async(JsProbe::from(probe))
            .await
            .map_err(adapter_err)?;
        promise.await.map_err(adapter_err).map(|_: ()| ())
    }
}

fn adapter_err<E: std::fmt::Display>(err: E) -> AdapterError {
    AdapterError::Callback(err.to_string())
}

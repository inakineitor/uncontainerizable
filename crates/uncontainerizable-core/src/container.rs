//! `Container` trait + result/options types.
//!
//! Only the trait declaration and the shared result/options types live here
//! currently. Concrete implementations (`BasicContainer`, the shared
//! quit/destroy loop, and per-platform subclasses) land in follow-up work.

use std::collections::HashMap;
use std::time::Duration;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::error::{Error, StageError};
use crate::probe::Probe;

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct QuitOptions {
    /// Override the default `max_wait` for specific stages by name.
    pub stage_timeouts: HashMap<String, Duration>,
    /// Stages to skip entirely (by name).
    pub skip_stages: Vec<String>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct DestroyOptions {
    pub quit: QuitOptions,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct StageResult {
    pub stage_name: String,
    pub index: usize,
    /// `true` if `is_empty()` returned true after this stage.
    pub exited: bool,
    pub is_terminal: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct QuitResult {
    pub exited_at_stage: Option<String>,
    pub reached_terminal_stage: bool,
    pub stage_results: Vec<StageResult>,
    pub adapter_errors: Vec<String>,
}

#[derive(Debug)]
pub struct DestroyResult {
    pub quit: QuitResult,
    /// Errors collected from `destroy_resources` and non-fatal teardown steps.
    /// `destroy()` is infallible by design: surfacing errors means the caller
    /// can log/metric but never has to handle a thrown exception in a `finally`.
    pub errors: Vec<Error>,
}

/// Handle to a spawned, contained process. Implementations are platform-specific.
#[async_trait]
pub trait Container: Send + Sync {
    fn pid(&self) -> u32;
    fn probe(&self) -> &Probe;

    /// Enumerate every PID the container considers a member, including the root.
    async fn members(&self) -> Vec<u32>;

    /// `true` iff the container has no running members.
    /// Authoritative over observing the root PID alone: a Linux cgroup can be
    /// populated by helper processes long after the root exits.
    async fn is_empty(&self) -> std::result::Result<bool, StageError>;

    /// Run the staged quit ladder against this container. Returns a structured
    /// `QuitResult`; collected adapter errors are inside.
    async fn quit(&mut self, opts: QuitOptions) -> std::result::Result<QuitResult, Error>;

    /// `quit` + release platform resources. Infallible-ish: never returns `Err`,
    /// errors are collected into `DestroyResult.errors`.
    async fn destroy(&mut self, opts: DestroyOptions) -> DestroyResult;

    /// Platform hook. Subclasses override to drop their cgroup/job-object handle
    /// etc. `Vec<Error>` so multiple independent failures can surface together.
    async fn destroy_resources(&mut self) -> Vec<Error>;
}

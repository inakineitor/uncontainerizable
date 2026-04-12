//! `Adapter` trait: per-app lifecycle hooks.
//!
//! Adapters run around the staged quit ladder. Hook errors are *collected*
//! into the final `QuitResult`/`DestroyResult` and never abort escalation:
//! a misbehaving adapter can't prevent a container from being destroyed.

use async_trait::async_trait;

use crate::container::{Container, QuitResult, StageResult};
use crate::error::AdapterError;
use crate::probe::Probe;

#[async_trait]
pub trait Adapter: Send + Sync {
    /// Human-readable name. Used for logging and error attribution.
    fn name(&self) -> &str;

    /// Decide whether this adapter applies to the spawned process. Called once
    /// per (adapter, probe) pair; the result is cached by the container.
    fn matches(&self, probe: &Probe) -> bool;

    /// Runs once before any stage executes.
    async fn before_quit(
        &self,
        _probe: &Probe,
        _container: &dyn Container,
    ) -> Result<(), AdapterError> {
        Ok(())
    }

    /// Runs before each stage.
    async fn before_stage(
        &self,
        _probe: &Probe,
        _stage_name: &str,
        _container: &dyn Container,
    ) -> Result<(), AdapterError> {
        Ok(())
    }

    /// Runs after each stage, with its result.
    async fn after_stage(
        &self,
        _probe: &Probe,
        _stage_result: &StageResult,
        _container: &dyn Container,
    ) -> Result<(), AdapterError> {
        Ok(())
    }

    /// Runs once after the final stage.
    async fn after_quit(
        &self,
        _probe: &Probe,
        _quit_result: &QuitResult,
        _container: &dyn Container,
    ) -> Result<(), AdapterError> {
        Ok(())
    }

    /// Clear any crash state the OS or the managed program will use on the
    /// next launch to complain about the forced quit. Only runs if the
    /// terminal stage was reached.
    async fn clear_crash_state(&self, _probe: &Probe) -> Result<(), AdapterError> {
        Ok(())
    }
}

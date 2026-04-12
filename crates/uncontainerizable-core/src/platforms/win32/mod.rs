//! Windows platform implementation.
//!
//! Identity preemption is a Named Job Object at `Local\\uncontainerizable-<identity>`.
//! Opening the object by name is atomic; if a prior instance exists we
//! terminate every process in it, close the handle to release the name, then
//! create a fresh job. Dropping the supervisor's handle triggers
//! `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE`, so orphan prevention survives crashes
//! of the supervisor itself.
//!
//! The quit ladder is two stages: `WM_CLOSE` to top-level windows of the root
//! PID, then `TerminateJobObject`.

use std::sync::Arc;

use async_trait::async_trait;
use tokio::process::Command;

use crate::app::{App, ContainOptions};
use crate::container::{
    Container, ContainerCore, DestroyOptions, DestroyResult, QuitOptions, QuitResult, run_destroy,
    run_quit,
};
use crate::error::{Error, Result, StageError};
use crate::identity;

pub mod job_object;
pub mod probe;
pub mod stages;

use self::job_object::JobObject;

pub async fn spawn(app: &App, command: &str, opts: ContainOptions) -> Result<Box<dyn Container>> {
    let job = if let Some(ident) = &opts.identity {
        identity::validate(ident)?;
        let combined = identity::combine(app.prefix(), ident);
        JobObject::open_or_replace(&combined).map_err(|e| Error::Preempt {
            identity: combined,
            source: Box::new(e),
        })?
    } else {
        JobObject::anonymous().map_err(crate::error::PlatformError::JobObject)?
    };
    let job = Arc::new(job);

    let mut cmd = Command::new(command);
    cmd.args(&opts.args).envs(opts.env.iter().cloned());
    if let Some(cwd) = &opts.cwd {
        cmd.current_dir(cwd);
    }
    let child = cmd.spawn().map_err(|e| Error::Spawn {
        command: command.into(),
        source: e,
    })?;
    let pid = child.id().ok_or_else(|| Error::Spawn {
        command: command.into(),
        source: std::io::Error::other("child has no pid"),
    })?;

    job.assign_pid(pid)
        .map_err(crate::error::PlatformError::JobObject)?;

    let probe = probe::capture_probe(pid).await?;
    let ladder = stages::win32_stages(job.clone());
    let core = ContainerCore::new(pid, probe, opts.adapters, ladder);
    Ok(Box::new(WindowsContainer::new(core, job)))
}

/// Windows container. Membership is drawn from the Job Object's process
/// list; teardown closes the job handle, which (via
/// `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE`) kills any remaining survivors.
pub struct WindowsContainer {
    core: ContainerCore,
    job: Arc<JobObject>,
}

impl WindowsContainer {
    pub fn new(core: ContainerCore, job: Arc<JobObject>) -> Self {
        Self { core, job }
    }

    pub fn job(&self) -> &Arc<JobObject> {
        &self.job
    }
}

#[async_trait]
impl Container for WindowsContainer {
    fn core(&self) -> &ContainerCore {
        &self.core
    }

    fn core_mut(&mut self) -> &mut ContainerCore {
        &mut self.core
    }

    async fn members(&self) -> Vec<u32> {
        self.job.members().unwrap_or_default()
    }

    async fn is_empty(&self) -> std::result::Result<bool, StageError> {
        Ok(self.job.members().map(|v| v.is_empty()).unwrap_or(true))
    }

    async fn destroy_resources(&mut self) -> Vec<Error> {
        // Closing the last handle to a named job releases the name and,
        // via `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE`, terminates any member
        // that somehow survived the terminal stage. The Arc-owned handle
        // drops when both `self.job` and the terminal stage's clone go.
        Vec::new()
    }

    async fn quit(&mut self, opts: QuitOptions) -> Result<QuitResult> {
        run_quit(self, opts).await
    }

    async fn destroy(&mut self, opts: DestroyOptions) -> DestroyResult {
        run_destroy(self, opts).await
    }
}

//! Windows platform implementation.
//!
//! Identity preemption uses a per-identity mutex plus a registry of unique
//! named Job Objects. Each generation gets its own job, so stale containers
//! cannot act on successors. Dropping the supervisor's handle still triggers
//! `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE`, so orphan prevention survives crashes
//! of the supervisor itself.
//!
//! The quit ladder is two stages: `WM_CLOSE` to top-level windows of the root
//! PID, then `TerminateJobObject`.

use std::mem::size_of;
use std::os::windows::process::CommandExt;
use std::process::{Child, Command as StdCommand};
use std::sync::Arc;

use async_trait::async_trait;
use windows::Win32::Foundation::CloseHandle;
use windows::Win32::System::Diagnostics::ToolHelp::{
    CreateToolhelp32Snapshot, TH32CS_SNAPTHREAD, THREADENTRY32, Thread32First, Thread32Next,
};
use windows::Win32::System::Threading::{
    CREATE_SUSPENDED, OpenThread, ResumeThread, THREAD_SUSPEND_RESUME,
};

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

use self::job_object::{IdentityClaim, JobObject};

pub async fn spawn(app: &App, command: &str, opts: ContainOptions) -> Result<Box<dyn Container>> {
    let identity = if let Some(ident) = opts.identity.as_deref() {
        identity::validate(ident)?;
        Some(identity::combine(app.prefix(), ident))
    } else {
        None
    };

    let (job, claim, job_name) = if let Some(combined) = &identity {
        let claim = IdentityClaim::acquire(combined).map_err(|e| Error::Preempt {
            identity: combined.clone(),
            source: Box::new(e),
        })?;
        claim.terminate_predecessor().map_err(|e| Error::Preempt {
            identity: combined.clone(),
            source: Box::new(e),
        })?;
        let (job, job_name) = claim.create_successor_job().map_err(|e| Error::Preempt {
            identity: combined.clone(),
            source: Box::new(e),
        })?;
        (job, Some(claim), Some(job_name))
    } else {
        (
            JobObject::anonymous().map_err(crate::error::PlatformError::JobObject)?,
            None,
            None,
        )
    };

    let mut child = spawn_suspended_child(command, &opts)?;
    let pid = child.id();
    if let Err(error) = job.assign_pid(pid) {
        let _ = child.kill();
        return Err(crate::error::PlatformError::JobObject(error).into());
    }
    if let (Some(claim), Some(job_name), Some(combined)) =
        (claim, job_name.as_deref(), identity.as_ref())
    {
        if let Err(error) = claim.commit(job_name) {
            let _ = child.kill();
            return Err(Error::Preempt {
                identity: combined.clone(),
                source: Box::new(error),
            });
        }
    }
    if let Err(error) = resume_main_thread(pid) {
        let _ = child.kill();
        return Err(crate::error::PlatformError::JobObject(error).into());
    }

    let probe = probe::capture_probe(pid).await?;
    drop(child);

    let job = Arc::new(job);
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

fn spawn_suspended_child(command: &str, opts: &ContainOptions) -> Result<Child> {
    let mut cmd = StdCommand::new(command);
    cmd.args(&opts.args).envs(opts.env.iter().cloned());
    if let Some(cwd) = &opts.cwd {
        cmd.current_dir(cwd);
    }
    cmd.creation_flags(CREATE_SUSPENDED.0);
    cmd.spawn().map_err(|source| Error::Spawn {
        command: command.into(),
        source,
    })
}

fn resume_main_thread(pid: u32) -> std::result::Result<(), crate::error::JobObjectError> {
    let thread_id = suspended_main_thread_id(pid)?;
    unsafe {
        let thread = OpenThread(THREAD_SUSPEND_RESUME, false, thread_id)
            .map_err(|source| crate::error::JobObjectError::ResumeProcess { pid, source })?;
        let resume_result = ResumeThread(thread);
        let close_result = CloseHandle(thread);
        if resume_result == u32::MAX {
            return Err(crate::error::JobObjectError::ResumeProcess {
                pid,
                source: windows::core::Error::from_thread(),
            });
        }
        let _ = close_result;
        Ok(())
    }
}

fn suspended_main_thread_id(pid: u32) -> std::result::Result<u32, crate::error::JobObjectError> {
    unsafe {
        let snapshot = CreateToolhelp32Snapshot(TH32CS_SNAPTHREAD, 0)
            .map_err(|source| crate::error::JobObjectError::ResumeProcess { pid, source })?;
        let mut entry: THREADENTRY32 = std::mem::zeroed();
        entry.dwSize = size_of::<THREADENTRY32>() as u32;

        let first = Thread32First(snapshot, &mut entry);
        if let Err(source) = first {
            let _ = CloseHandle(snapshot);
            return Err(crate::error::JobObjectError::ResumeProcess { pid, source });
        }

        loop {
            if entry.th32OwnerProcessID == pid {
                let _ = CloseHandle(snapshot);
                return Ok(entry.th32ThreadID);
            }
            if Thread32Next(snapshot, &mut entry).is_err() {
                break;
            }
        }

        let _ = CloseHandle(snapshot);
        Err(crate::error::JobObjectError::MissingMainThread { pid })
    }
}

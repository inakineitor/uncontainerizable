//! Linux quit-ladder stages.
//!
//! Two stages. Both rely on cgroup freeze to deliver signals race-free:
//! we freeze the group, enumerate members, send the signal to each, thaw,
//! and let `is_empty()` drive the poll.
//!
//! * `sigterm_frozen`: SIGTERM everything in the cgroup. Non-terminal.
//! * `sigkill_frozen`: SIGKILL everything. Terminal.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use nix::sys::signal::{Signal, kill};
use nix::unistd::Pid;
use tokio::fs;

use crate::container::{Container, Stage};
use crate::error::StageError;

pub fn linux_stages(cgroup_path: PathBuf) -> Vec<Arc<dyn Stage>> {
    vec![
        Arc::new(SigTermFrozen::new(cgroup_path.clone())),
        Arc::new(SigKillFrozen::new(cgroup_path)),
    ]
}

pub struct SigTermFrozen {
    cgroup_path: PathBuf,
}

impl SigTermFrozen {
    pub fn new(cgroup_path: PathBuf) -> Self {
        Self { cgroup_path }
    }
}

#[async_trait]
impl Stage for SigTermFrozen {
    fn name(&self) -> &str {
        "sigterm_frozen"
    }
    fn is_terminal(&self) -> bool {
        false
    }
    fn max_wait(&self) -> Duration {
        Duration::from_secs(2)
    }
    async fn execute(&self, _c: &dyn Container) -> Result<(), StageError> {
        signal_frozen_members(&self.cgroup_path, Signal::SIGTERM).await?;
        Ok(())
    }
}

pub struct SigKillFrozen {
    cgroup_path: PathBuf,
}

impl SigKillFrozen {
    pub fn new(cgroup_path: PathBuf) -> Self {
        Self { cgroup_path }
    }
}

#[async_trait]
impl Stage for SigKillFrozen {
    fn name(&self) -> &str {
        "sigkill_frozen"
    }
    fn is_terminal(&self) -> bool {
        true
    }
    fn max_wait(&self) -> Duration {
        Duration::from_millis(500)
    }
    async fn execute(&self, _c: &dyn Container) -> Result<(), StageError> {
        // Prefer `cgroup.kill` on kernels that support it (5.14+). Writing
        // `1` SIGKILLs every process in the subtree atomically.
        let kill_file = self.cgroup_path.join("cgroup.kill");
        if fs::metadata(&kill_file).await.is_ok() {
            fs::write(&kill_file, "1").await?;
            return Ok(());
        }
        signal_frozen_members(&self.cgroup_path, Signal::SIGKILL).await?;
        Ok(())
    }
}

async fn signal_frozen_members(path: &Path, signal: Signal) -> Result<(), StageError> {
    fs::write(path.join("cgroup.freeze"), "1").await?;
    let procs = fs::read_to_string(path.join("cgroup.procs"))
        .await
        .unwrap_or_default();
    for line in procs.lines() {
        if let Ok(pid) = line.trim().parse::<u32>() {
            let _ = kill(Pid::from_raw(pid as i32), signal);
        }
    }
    fs::write(path.join("cgroup.freeze"), "0").await?;
    Ok(())
}

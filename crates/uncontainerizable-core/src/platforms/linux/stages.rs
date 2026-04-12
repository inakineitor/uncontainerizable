//! Linux quit-ladder stages.
//!
//! Two stages. Both rely on cgroup freeze to deliver signals race-free:
//! we freeze the group, enumerate members, send the signal to each, thaw,
//! and let `is_empty()` drive the poll.
//!
//! * `sigterm_frozen`: SIGTERM everything in the cgroup. Non-terminal.
//! * `sigkill_frozen`: SIGKILL everything. Terminal.

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use nix::sys::signal::{Signal, kill};
use nix::unistd::Pid;
use tokio::fs;

use crate::container::{Container, Stage};
use crate::error::StageError;

use super::cgroup::Cgroup;

pub fn linux_stages() -> Vec<Arc<dyn Stage>> {
    vec![Arc::new(SigTermFrozen), Arc::new(SigKillFrozen)]
}

pub struct SigTermFrozen;

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
    async fn execute(&self, c: &dyn Container) -> Result<(), StageError> {
        let path = linux_cgroup_path(c)?;
        signal_frozen_members(&path, Signal::SIGTERM).await?;
        Ok(())
    }
}

pub struct SigKillFrozen;

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
    async fn execute(&self, c: &dyn Container) -> Result<(), StageError> {
        let path = linux_cgroup_path(c)?;
        // Prefer `cgroup.kill` on kernels that support it (5.14+). Writing
        // `1` SIGKILLs every process in the subtree atomically.
        let kill_file = path.join("cgroup.kill");
        if fs::metadata(&kill_file).await.is_ok() {
            fs::write(&kill_file, "1").await?;
            return Ok(());
        }
        signal_frozen_members(&path, Signal::SIGKILL).await?;
        Ok(())
    }
}

async fn signal_frozen_members(path: &std::path::Path, signal: Signal) -> Result<(), StageError> {
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

/// Helper so stages can reach the cgroup path without knowing about
/// `LinuxContainer` directly (the stages run before the container impl is
/// fully in scope).
fn linux_cgroup_path(c: &dyn Container) -> Result<std::path::PathBuf, StageError> {
    // Downcast through a platform-specific accessor baked into `Container`
    // via `cgroup_path_hint` metadata stored in the probe's executable_path
    // is brittle; instead, each stage looks it up by walking `/proc/<pid>/cgroup`
    // for the root PID, which always reflects the cgroup we placed it in.
    let pid = c.pid();
    let raw = std::fs::read_to_string(format!("/proc/{pid}/cgroup"))?;
    let rel = raw
        .lines()
        .find_map(|l| l.strip_prefix("0::"))
        .ok_or(StageError::MissingProbe("cgroup path"))?;
    Ok(std::path::Path::new("/sys/fs/cgroup").join(rel.trim_start_matches('/')))
}

// Re-export for docs.
#[allow(dead_code)]
pub(crate) fn _use_cgroup_type(_: &Cgroup) {}

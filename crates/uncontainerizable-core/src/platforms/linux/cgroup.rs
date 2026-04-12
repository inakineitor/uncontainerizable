//! Thin wrapper over cgroup v2 files.
//!
//! Uses direct file I/O on `cgroup.procs`, `cgroup.freeze`, `cgroup.events`,
//! and `cgroup.kill` rather than the `cgroups-rs` crate's higher-level
//! machinery. That crate targets v1 + v2 and its 0.4 API changed between
//! minor releases; we only need v2 so a ~120 LOC direct implementation
//! avoids the moving target and one transitive dep.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use nix::errno::Errno;
use tokio::fs;
use tokio::time::sleep;

use crate::error::CgroupError;

const POLL_INTERVAL: Duration = Duration::from_millis(50);
const FREEZE_TIMEOUT: Duration = Duration::from_millis(1000);
const REAP_TIMEOUT: Duration = Duration::from_millis(500);

const UNIFIED_ROOT: &str = "/sys/fs/cgroup";
const SUBGROUP_SEGMENT: &str = "uncontainerizable";

/// A cgroup v2 directory owned by this supervisor. Dropping the value does
/// not rmdir; call `destroy()` after the container drains.
pub struct Cgroup {
    path: PathBuf,
}

impl Cgroup {
    /// Verify cgroup v2 is mounted in unified mode and the current session
    /// has write access under its own subtree. Fails fast so the error
    /// surfaces before we spawn a child we can't manage.
    pub async fn assert_available() -> Result<(), CgroupError> {
        if !Path::new(UNIFIED_ROOT).join("cgroup.controllers").exists() {
            return Err(CgroupError::NotV2);
        }
        let current = current_cgroup_path().await?;
        let probe = current.join("cgroup.procs");
        let meta = fs::metadata(&probe).await?;
        if meta.permissions().readonly() {
            return Err(CgroupError::NotDelegated {
                path: current.display().to_string(),
            });
        }
        Ok(())
    }

    /// Create a cgroup at a deterministic path derived from the identity.
    /// If a cgroup already exists there, kill its members and replace it.
    pub async fn open_or_replace(identity: &str) -> Result<Self, CgroupError> {
        let sanitized = sanitize_for_cgroup(identity)?;
        let base = current_cgroup_path().await?;
        let path = base.join(SUBGROUP_SEGMENT).join(&sanitized);

        if fs::metadata(&path).await.is_ok() {
            kill_and_remove_cgroup(&path).await?;
        }

        fs::create_dir_all(&path).await?;
        Ok(Self { path })
    }

    /// Create an anonymous cgroup (no identity means no preemption, just a
    /// scratch group for this one-shot spawn).
    pub async fn create_anonymous() -> Result<Self, CgroupError> {
        let base = current_cgroup_path().await?;
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let name = format!("anon-{}-{}", std::process::id(), nanos);
        let path = base.join(SUBGROUP_SEGMENT).join(name);
        fs::create_dir_all(&path).await?;
        Ok(Self { path })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub async fn add(&self, pid: u32) -> Result<(), CgroupError> {
        fs::write(self.path.join("cgroup.procs"), pid.to_string()).await?;
        Ok(())
    }

    pub async fn members(&self) -> Vec<u32> {
        fs::read_to_string(self.path.join("cgroup.procs"))
            .await
            .unwrap_or_default()
            .lines()
            .filter_map(|l| l.trim().parse().ok())
            .collect()
    }

    pub async fn is_empty(&self) -> Result<bool, CgroupError> {
        let raw = fs::read_to_string(self.path.join("cgroup.events")).await?;
        Ok(raw.lines().any(|l| l == "populated 0"))
    }

    pub async fn freeze(&self) -> Result<(), CgroupError> {
        fs::write(self.path.join("cgroup.freeze"), "1").await?;
        self.wait_frozen(true).await
    }

    pub async fn thaw(&self) -> Result<(), CgroupError> {
        fs::write(self.path.join("cgroup.freeze"), "0").await?;
        self.wait_frozen(false).await
    }

    /// Remove the cgroup directory. Retries for a short window because an
    /// rmdir can race with the last pid being reaped.
    pub async fn destroy(&self) -> Result<(), CgroupError> {
        let deadline = Instant::now() + REAP_TIMEOUT;
        loop {
            match fs::remove_dir(&self.path).await {
                Ok(()) => return Ok(()),
                Err(e) if Instant::now() >= deadline => {
                    return Err(CgroupError::Io(e));
                }
                Err(_) => {
                    sleep(POLL_INTERVAL).await;
                }
            }
        }
    }

    async fn wait_frozen(&self, target: bool) -> Result<(), CgroupError> {
        let events = self.path.join("cgroup.events");
        let deadline = Instant::now() + FREEZE_TIMEOUT;
        while Instant::now() < deadline {
            let raw = fs::read_to_string(&events).await?;
            let frozen = raw
                .lines()
                .find_map(|l| l.strip_prefix("frozen "))
                .map(|v| v == "1")
                .unwrap_or(false);
            if frozen == target {
                return Ok(());
            }
            sleep(POLL_INTERVAL).await;
        }
        Err(CgroupError::FreezeTimeout {
            path: self.path.display().to_string(),
            target,
            timeout_ms: FREEZE_TIMEOUT.as_millis() as u64,
        })
    }
}

async fn kill_and_remove_cgroup(path: &Path) -> Result<(), CgroupError> {
    // v2 kernels (5.14+) have `cgroup.kill`; one write SIGKILLs every
    // process in the subtree. Fall back to manual SIGKILL when the file
    // isn't present (older kernels or restricted hosts).
    let kill_file = path.join("cgroup.kill");
    if fs::metadata(&kill_file).await.is_ok() {
        write_if_present(&kill_file, "1").await?;
    } else {
        write_if_present(&path.join("cgroup.freeze"), "1").await?;
        let procs = match fs::read_to_string(path.join("cgroup.procs")).await {
            Ok(procs) => procs,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => String::new(),
            Err(error) => return Err(CgroupError::Io(error)),
        };
        for line in procs.lines() {
            if let Ok(pid) = line.trim().parse::<u32>() {
                send_sigkill(pid)?;
            }
        }
        write_if_present(&path.join("cgroup.freeze"), "0").await?;
    }

    wait_for_drain(path).await?;
    match fs::remove_dir(path).await {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(CgroupError::Io(error)),
    }
    Ok(())
}

fn send_sigkill(pid: u32) -> Result<(), CgroupError> {
    use nix::sys::signal::{Signal, kill};
    use nix::unistd::Pid;

    match kill(Pid::from_raw(pid as i32), Signal::SIGKILL) {
        Ok(()) | Err(Errno::ESRCH) => Ok(()),
        Err(error) => Err(CgroupError::Other(format!(
            "failed to send SIGKILL to pid {pid}: {error}"
        ))),
    }
}

async fn wait_for_drain(path: &Path) -> Result<(), CgroupError> {
    let deadline = Instant::now() + REAP_TIMEOUT;
    while Instant::now() < deadline {
        match fs::read_to_string(path.join("cgroup.events")).await {
            Ok(events) if events.lines().any(|line| line == "populated 0") => return Ok(()),
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(error) => return Err(CgroupError::Io(error)),
        }
        sleep(POLL_INTERVAL).await;
    }

    Err(CgroupError::Other(format!(
        "cgroup {} did not drain within {}ms",
        path.display(),
        REAP_TIMEOUT.as_millis()
    )))
}

async fn write_if_present(path: &Path, contents: &str) -> Result<(), CgroupError> {
    match fs::write(path, contents).await {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(CgroupError::Io(error)),
    }
}

async fn current_cgroup_path() -> Result<PathBuf, CgroupError> {
    let raw = fs::read_to_string("/proc/self/cgroup").await?;
    let rel = raw
        .lines()
        .find_map(|l| l.strip_prefix("0::"))
        .ok_or(CgroupError::NotV2)?;
    Ok(Path::new(UNIFIED_ROOT).join(rel.trim_start_matches('/')))
}

fn sanitize_for_cgroup(identity: &str) -> Result<String, CgroupError> {
    let s: String = identity
        .chars()
        .map(|c| if c == ':' { '.' } else { c })
        .collect();
    if s.is_empty() || s == "." || s == ".." {
        return Err(CgroupError::InvalidName(identity.into()));
    }
    Ok(s)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sanitize_replaces_colon_with_dot() {
        assert_eq!(
            sanitize_for_cgroup("com.example:browser-main").unwrap(),
            "com.example.browser-main"
        );
    }

    #[test]
    fn sanitize_rejects_empty_dot_dotdot() {
        assert!(sanitize_for_cgroup(".").is_err());
        assert!(sanitize_for_cgroup("..").is_err());
        assert!(sanitize_for_cgroup("").is_err());
    }
}

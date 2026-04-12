//! macOS probe capture.
//!
//! Best-effort: any field that can't be resolved is left as `None` so
//! downstream adapter matching degrades gracefully.

use std::path::PathBuf;

use tokio::process::Command;

use crate::error::ProbeError;
use crate::probe::{Probe, SupportedPlatform};

use super::lsappinfo;

pub async fn capture_probe(pid: u32) -> Result<Probe, ProbeError> {
    let mut probe = Probe::new(pid, SupportedPlatform::Darwin);
    probe.executable_path = exe_path_from_ps(pid).await.ok();
    probe.bundle_id = lsappinfo::bundle_id(pid).await;
    Ok(probe)
}

async fn exe_path_from_ps(pid: u32) -> Result<PathBuf, ProbeError> {
    let output = Command::new("ps")
        .args(["-p", &pid.to_string(), "-o", "comm="])
        .output()
        .await?;
    if !output.status.success() {
        return Err(ProbeError::Subprocess {
            command: "ps".into(),
            message: format!("ps exited with {}", output.status),
        });
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    let line = stdout.lines().next().unwrap_or("").trim();
    if line.is_empty() {
        return Err(ProbeError::Subprocess {
            command: "ps".into(),
            message: "empty output".into(),
        });
    }
    Ok(PathBuf::from(line))
}

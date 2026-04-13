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
    capture_probe_with_bundle(pid, None).await
}

/// Like `capture_probe`, but skips the `lsappinfo` shell-out when the
/// caller has already resolved the bundle ID through a more reliable
/// source (e.g. parsing `Info.plist` on the Launch Services spawn path).
/// The LS route hits this race-free because the plist read happens
/// before we even ask LS to launch the app.
pub async fn capture_probe_with_bundle(
    pid: u32,
    bundle_id: Option<String>,
) -> Result<Probe, ProbeError> {
    let mut probe = Probe::new(pid, SupportedPlatform::Darwin);
    probe.executable_path = exe_path_from_ps(pid).await.ok();
    probe.bundle_id = if bundle_id.is_some() {
        bundle_id
    } else {
        lsappinfo::bundle_id(pid).await
    };
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

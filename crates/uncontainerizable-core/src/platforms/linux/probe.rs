//! Linux probe capture. Reads the executable path from `/proc/<pid>/exe`;
//! bundle-id has no Linux analogue so it's always `None`.

use std::path::PathBuf;

use tokio::fs;

use crate::error::ProbeError;
use crate::probe::{Probe, SupportedPlatform};

pub async fn capture_probe(pid: u32) -> Result<Probe, ProbeError> {
    let mut probe = Probe::new(pid, SupportedPlatform::Linux);
    probe.executable_path = exe_path(pid).await.ok();
    Ok(probe)
}

async fn exe_path(pid: u32) -> Result<PathBuf, ProbeError> {
    let link = format!("/proc/{pid}/exe");
    let target = fs::read_link(&link).await?;
    Ok(target)
}

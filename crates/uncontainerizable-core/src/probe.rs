//! Probe: identity info captured at spawn time.
//!
//! Captured once when the container is created, so adapter matching survives
//! post-mortem (after the root PID has exited). Platform probe modules fill
//! in the optional fields; all of them may be `None` in v0.1.

use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SupportedPlatform {
    Linux,
    Darwin,
    #[serde(rename = "win32")]
    Windows,
}

impl SupportedPlatform {
    /// Current platform, or `None` on anything uncontainerizable doesn't support.
    pub const fn current() -> Option<Self> {
        #[cfg(target_os = "linux")]
        {
            Some(Self::Linux)
        }
        #[cfg(target_os = "macos")]
        {
            Some(Self::Darwin)
        }
        #[cfg(windows)]
        {
            Some(Self::Windows)
        }
        #[cfg(not(any(target_os = "linux", target_os = "macos", windows)))]
        {
            None
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Probe {
    pub pid: u32,
    pub bundle_id: Option<String>,
    pub executable_path: Option<PathBuf>,
    pub platform: SupportedPlatform,
    /// Unix timestamp (milliseconds) at spawn.
    pub captured_at_ms: u64,
}

impl Probe {
    pub fn new(pid: u32, platform: SupportedPlatform) -> Self {
        let captured_at_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0);
        Self {
            pid,
            bundle_id: None,
            executable_path: None,
            platform,
            captured_at_ms,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn supported_platform_current_is_some_on_supported_targets() {
        let current = SupportedPlatform::current();
        #[cfg(any(target_os = "linux", target_os = "macos", windows))]
        assert!(current.is_some());
        #[cfg(not(any(target_os = "linux", target_os = "macos", windows)))]
        assert!(current.is_none());
    }

    #[test]
    fn probe_new_captures_pid_and_platform() {
        let platform = SupportedPlatform::current().unwrap_or(SupportedPlatform::Linux);
        let probe = Probe::new(42, platform);
        assert_eq!(probe.pid, 42);
        assert_eq!(probe.platform, platform);
        assert!(probe.bundle_id.is_none());
        assert!(probe.executable_path.is_none());
    }

    #[test]
    fn supported_platform_serializes_as_lowercase_with_win32_alias() {
        let linux = serde_json::to_string(&SupportedPlatform::Linux).unwrap();
        let darwin = serde_json::to_string(&SupportedPlatform::Darwin).unwrap();
        let windows = serde_json::to_string(&SupportedPlatform::Windows).unwrap();
        assert_eq!(linux, "\"linux\"");
        assert_eq!(darwin, "\"darwin\"");
        assert_eq!(windows, "\"win32\"");
    }
}

//! Error types for uncontainerizable-core.
//!
//! Layered:
//! - `Error` is the crate's public error; every fallible operation surfaces this.
//! - `ProbeError`, `PlatformError`, `StageError`, `AdapterError` are leaf variants
//!   that convert into `Error` via `#[from]`.
//! - `CgroupError` and `JobObjectError` are platform-specific and only exist
//!   under their respective `cfg` gates.

use thiserror::Error;

#[derive(Debug, Error)]
pub enum Error {
    #[error("platform {0} is not supported")]
    UnsupportedPlatform(String),

    #[error("failed to spawn {command}")]
    Spawn {
        command: String,
        #[source]
        source: std::io::Error,
    },

    #[error("failed to preempt prior instance of identity {identity:?}")]
    Preempt {
        identity: String,
        #[source]
        source: Box<dyn std::error::Error + Send + Sync>,
    },

    #[error("invalid identity: {0}")]
    InvalidIdentity(String),

    #[error(transparent)]
    Probe(#[from] ProbeError),

    #[error("container has already been destroyed")]
    AlreadyDestroyed,

    #[error(transparent)]
    Platform(#[from] PlatformError),

    #[error(transparent)]
    Stage(#[from] StageError),
}

#[derive(Debug, Error)]
pub enum ProbeError {
    #[error(transparent)]
    Io(#[from] std::io::Error),

    #[error("subprocess {command} failed: {message}")]
    Subprocess { command: String, message: String },
}

#[derive(Debug, Error)]
pub enum PlatformError {
    #[cfg(target_os = "linux")]
    #[error(transparent)]
    Cgroup(#[from] CgroupError),

    #[cfg(windows)]
    #[error(transparent)]
    JobObject(#[from] JobObjectError),

    #[error(transparent)]
    Io(#[from] std::io::Error),

    #[error("platform subsystem error: {0}")]
    Other(String),
}

#[cfg(target_os = "linux")]
#[derive(Debug, Error)]
pub enum CgroupError {
    #[error("cgroup v2 not mounted at /sys/fs/cgroup")]
    NotV2,

    #[error("no write access to {path}: session likely missing Delegate=yes")]
    NotDelegated { path: String },

    #[error("invalid cgroup name: {0:?}")]
    InvalidName(String),

    #[error("cgroup {path} did not reach frozen={target} within {timeout_ms}ms")]
    FreezeTimeout {
        path: String,
        target: bool,
        timeout_ms: u64,
    },

    #[error(transparent)]
    Io(#[from] std::io::Error),

    /// Passthrough for arbitrary upstream `cgroups-rs` errors. A typed
    /// `From<cgroups_rs::...::Error>` impl will be added alongside the Linux
    /// cgroup integration so the error path (which differs between
    /// `cgroups-rs` versions) can be pinned next to the real call sites.
    #[error("cgroups-rs error: {0}")]
    Upstream(String),
}

#[cfg(windows)]
#[derive(Debug, Error)]
pub enum JobObjectError {
    #[error("failed to open or create job object {name:?}")]
    OpenOrCreate {
        name: String,
        #[source]
        source: windows::core::Error,
    },

    #[error("failed to terminate predecessor job")]
    TerminatePredecessor {
        #[source]
        source: windows::core::Error,
    },

    #[error("failed to assign process to job")]
    AssignProcess {
        #[source]
        source: windows::core::Error,
    },

    #[error("failed to query job information")]
    Query {
        #[source]
        source: windows::core::Error,
    },
}

#[derive(Debug, Error)]
pub enum StageError {
    #[error("missing probe field: {0}")]
    MissingProbe(&'static str),

    #[error(transparent)]
    KillTree(#[from] kill_tree::Error),

    #[cfg(target_os = "linux")]
    #[error(transparent)]
    Cgroup(#[from] CgroupError),

    #[cfg(windows)]
    #[error(transparent)]
    JobObject(#[from] JobObjectError),

    #[error(transparent)]
    Io(#[from] std::io::Error),

    #[cfg(unix)]
    #[error("signal delivery failed: {0}")]
    Signal(#[from] nix::errno::Errno),

    #[error("stage {stage} timed out after {timeout_ms}ms")]
    Timeout { stage: String, timeout_ms: u64 },
}

#[derive(Debug, Error)]
pub enum AdapterError {
    #[error(transparent)]
    Io(#[from] std::io::Error),

    #[error(transparent)]
    Serde(#[from] serde_json::Error),

    #[error("adapter callback failed: {0}")]
    Callback(String),
}

pub type Result<T> = std::result::Result<T, Error>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unsupported_platform_renders_message() {
        let e = Error::UnsupportedPlatform("amiga".into());
        assert_eq!(e.to_string(), "platform amiga is not supported");
    }

    #[test]
    fn invalid_identity_renders_message() {
        let e = Error::InvalidIdentity("empty".into());
        assert_eq!(e.to_string(), "invalid identity: empty");
    }

    #[test]
    fn already_destroyed_renders_message() {
        let e = Error::AlreadyDestroyed;
        assert_eq!(e.to_string(), "container has already been destroyed");
    }

    #[test]
    fn probe_error_converts_into_error() {
        let inner = ProbeError::Subprocess {
            command: "ps".into(),
            message: "boom".into(),
        };
        let wrapped: Error = inner.into();
        assert!(matches!(wrapped, Error::Probe(_)));
    }
}

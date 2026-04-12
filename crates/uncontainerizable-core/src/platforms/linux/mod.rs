//! Linux platform implementation.
//!
//! Identity preemption is cgroup v2: the cgroup directory itself is the
//! kernel-backed source of truth. Replacing an identity tears down the
//! predecessor cgroup before the new child starts.
//!
//! The quit ladder is two stages (SIGTERM then SIGKILL), both delivered
//! race-free through freeze / signal / thaw (see `stages`).

use std::ffi::CString;
use std::os::unix::ffi::OsStrExt;

use async_trait::async_trait;
use nix::libc;
use tokio::process::Command;

use crate::app::{App, ContainOptions};
use crate::container::{
    Container, ContainerCore, DestroyOptions, DestroyResult, QuitOptions, QuitResult, run_destroy,
    run_quit,
};
use crate::error::{Error, Result, StageError};
use crate::identity;

pub mod cgroup;
pub mod probe;
pub mod stages;

use self::cgroup::Cgroup;

pub async fn spawn(app: &App, command: &str, opts: ContainOptions) -> Result<Box<dyn Container>> {
    Cgroup::assert_available()
        .await
        .map_err(|e| Error::Platform(crate::error::PlatformError::Cgroup(e)))?;

    let cg = if let Some(ident) = &opts.identity {
        identity::validate(ident)?;
        let combined = identity::combine(app.prefix(), ident);
        Cgroup::open_or_replace(&combined)
            .await
            .map_err(|e| Error::Preempt {
                identity: combined,
                source: Box::new(e),
            })?
    } else {
        Cgroup::create_anonymous()
            .await
            .map_err(crate::error::PlatformError::Cgroup)?
    };
    let cgroup_procs = cgroup_procs_cstring(&cg).map_err(|source| Error::Spawn {
        command: command.into(),
        source,
    })?;

    let mut cmd = Command::new(command);
    cmd.args(&opts.args).envs(opts.env.iter().cloned());
    if let Some(cwd) = &opts.cwd {
        cmd.current_dir(cwd);
    }
    unsafe {
        // `pre_exec` runs after fork and before exec, so the child joins
        // the cgroup before any user code can spawn descendants.
        cmd.pre_exec(move || write_self_to_cgroup(&cgroup_procs));
    }
    let child = cmd.spawn().map_err(|e| Error::Spawn {
        command: command.into(),
        source: e,
    })?;
    let pid = child.id().ok_or_else(|| Error::Spawn {
        command: command.into(),
        source: std::io::Error::other("child has no pid"),
    })?;

    let probe = probe::capture_probe(pid).await?;
    let stages = stages::linux_stages(cg.path().to_path_buf());
    let core = ContainerCore::new(pid, probe, opts.adapters, stages);
    Ok(Box::new(LinuxContainer::new(core, cg)))
}

/// Linux container. Membership comes from `cgroup.procs`; teardown
/// consists of the staged signal delivery plus rmdir of the cgroup
/// directory.
pub struct LinuxContainer {
    core: ContainerCore,
    cg: Cgroup,
}

impl LinuxContainer {
    pub fn new(core: ContainerCore, cg: Cgroup) -> Self {
        Self { core, cg }
    }
}

fn cgroup_procs_cstring(cg: &Cgroup) -> std::io::Result<CString> {
    CString::new(cg.path().join("cgroup.procs").as_os_str().as_bytes()).map_err(|_| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "cgroup path contained interior NUL byte",
        )
    })
}

fn write_self_to_cgroup(cgroup_procs: &std::ffi::CStr) -> std::io::Result<()> {
    unsafe {
        let fd = libc::open(cgroup_procs.as_ptr(), libc::O_WRONLY | libc::O_CLOEXEC);
        if fd < 0 {
            return Err(std::io::Error::last_os_error());
        }

        let (pid_buf, pid_len) = pid_decimal_bytes(libc::getpid() as u32);
        let mut written = 0usize;
        while written < pid_len {
            let rc = libc::write(
                fd,
                pid_buf[written..pid_len].as_ptr().cast(),
                pid_len - written,
            );
            if rc < 0 {
                let err = std::io::Error::last_os_error();
                let _ = libc::close(fd);
                return Err(err);
            }
            written += rc as usize;
        }

        if libc::close(fd) < 0 {
            return Err(std::io::Error::last_os_error());
        }

        Ok(())
    }
}

fn pid_decimal_bytes(mut pid: u32) -> ([u8; 20], usize) {
    let mut scratch = [0u8; 20];
    let mut cursor = scratch.len();
    loop {
        cursor -= 1;
        scratch[cursor] = b'0' + (pid % 10) as u8;
        pid /= 10;
        if pid == 0 {
            break;
        }
    }

    let len = scratch.len() - cursor;
    let mut output = [0u8; 20];
    output[..len].copy_from_slice(&scratch[cursor..]);
    (output, len)
}

#[async_trait]
impl Container for LinuxContainer {
    fn core(&self) -> &ContainerCore {
        &self.core
    }

    fn core_mut(&mut self) -> &mut ContainerCore {
        &mut self.core
    }

    async fn members(&self) -> Vec<u32> {
        self.cg.members().await
    }

    async fn is_empty(&self) -> std::result::Result<bool, StageError> {
        self.cg
            .is_empty()
            .await
            .map_err(crate::error::StageError::from)
    }

    async fn destroy_resources(&mut self) -> Vec<Error> {
        match self.cg.destroy().await {
            Ok(()) => Vec::new(),
            Err(e) => vec![Error::Platform(crate::error::PlatformError::Cgroup(e))],
        }
    }

    async fn quit(&mut self, opts: QuitOptions) -> Result<QuitResult> {
        run_quit(self, opts).await
    }

    async fn destroy(&mut self, opts: DestroyOptions) -> DestroyResult {
        run_destroy(self, opts).await
    }
}

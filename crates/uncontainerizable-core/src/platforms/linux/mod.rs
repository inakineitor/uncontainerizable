//! Linux platform implementation.
//!
//! Identity preemption is cgroup v2: the cgroup directory itself is the
//! kernel-backed source of truth. `mkdir` is atomic, so concurrent spawns
//! serialize via `EEXIST`; before creating we freeze, SIGKILL, and
//! rmdir any prior cgroup at the path.
//!
//! The quit ladder is two stages (SIGTERM then SIGKILL), both delivered
//! race-free through freeze / signal / thaw (see `stages`).

use async_trait::async_trait;
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

    let mut cmd = Command::new(command);
    cmd.args(&opts.args).envs(opts.env.iter().cloned());
    if let Some(cwd) = &opts.cwd {
        cmd.current_dir(cwd);
    }
    let child = cmd.spawn().map_err(|e| Error::Spawn {
        command: command.into(),
        source: e,
    })?;
    let pid = child.id().ok_or_else(|| Error::Spawn {
        command: command.into(),
        source: std::io::Error::other("child has no pid"),
    })?;

    cg.add(pid)
        .await
        .map_err(crate::error::PlatformError::Cgroup)?;

    let probe = probe::capture_probe(pid).await?;
    let stages = stages::linux_stages();
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

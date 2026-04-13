//! macOS platform implementation.
//!
//! Identity preemption is argv[0] tagging (see `argv_tag`): best-effort
//! because argv[0] is not a kernel primitive like cgroup v2 or a named
//! Job Object. The three-stage quit ladder (see `stages`) escalates from
//! Apple Events through SIGTERM and SIGKILL. We place the spawned root in
//! its own process group so helpers remain targetable even if the root
//! exits and macOS reparents descendants to launchd.

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

pub mod argv_tag;
pub mod lsappinfo;
pub mod probe;
pub mod stages;

pub async fn spawn(app: &App, command: &str, opts: ContainOptions) -> Result<Box<dyn Container>> {
    let tagged_argv0 = if let Some(ident) = &opts.identity {
        identity::validate(ident)?;
        let combined = identity::combine(app.prefix(), ident);
        argv_tag::kill_existing(&combined)
            .await
            .map_err(|e| Error::Preempt {
                identity: combined.clone(),
                source: Box::new(e),
            })?;
        if opts.darwin_tag_argv0 {
            Some(argv_tag::format(&combined, command))
        } else {
            None
        }
    } else {
        None
    };

    let mut cmd = Command::new(command);
    cmd.args(&opts.args).envs(opts.env.iter().cloned());
    if let Some(cwd) = &opts.cwd {
        cmd.current_dir(cwd);
    }
    unsafe {
        cmd.pre_exec(set_process_group_self);
    }
    if let Some(argv0) = &tagged_argv0 {
        cmd.arg0(argv0);
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
    let stages = stages::darwin_stages();
    let core = ContainerCore::new(pid, probe, opts.adapters, stages);
    Ok(Box::new(DarwinContainer::new(core)))
}

/// macOS container. Adds no state beyond `ContainerCore`: there's no
/// kernel primitive to hold, and membership is tracked by walking the
/// process tree on demand.
pub struct DarwinContainer {
    core: ContainerCore,
}

impl DarwinContainer {
    pub fn new(core: ContainerCore) -> Self {
        Self { core }
    }
}

#[async_trait]
impl Container for DarwinContainer {
    fn core(&self) -> &ContainerCore {
        &self.core
    }

    fn core_mut(&mut self) -> &mut ContainerCore {
        &mut self.core
    }

    async fn members(&self) -> Vec<u32> {
        members_for_process_group(self.core.pid).await
    }

    async fn is_empty(&self) -> std::result::Result<bool, StageError> {
        Ok(!process_group_alive(self.core.pid))
    }

    async fn destroy_resources(&mut self) -> Vec<Error> {
        // No kernel resource to release on Darwin; argv[0] tag is just a
        // string, and the process tree is reaped by kill_tree in the stages.
        Vec::new()
    }

    async fn quit(&mut self, opts: QuitOptions) -> Result<QuitResult> {
        run_quit(self, opts).await
    }

    async fn destroy(&mut self, opts: DestroyOptions) -> DestroyResult {
        run_destroy(self, opts).await
    }
}

fn set_process_group_self() -> std::io::Result<()> {
    unsafe {
        if libc::setpgid(0, 0) == 0 {
            Ok(())
        } else {
            Err(std::io::Error::last_os_error())
        }
    }
}

/// Enumerate members of the dedicated process group we create for every
/// contained root. The group id is the root pid because `pre_exec`
/// calls `setpgid(0, 0)` in the child before `exec`.
async fn members_for_process_group(process_group: u32) -> Vec<u32> {
    let Ok(output) = Command::new("ps")
        .args(["-axo", "pid=,pgid="])
        .output()
        .await
    else {
        return fallback_members(process_group);
    };
    if !output.status.success() {
        return fallback_members(process_group);
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let mut members = Vec::new();
    for line in stdout.lines() {
        let mut iter = line.split_whitespace();
        let (Some(pid_str), Some(pgid_str)) = (iter.next(), iter.next()) else {
            continue;
        };
        let (Ok(pid), Ok(pgid)) = (pid_str.parse::<u32>(), pgid_str.parse::<u32>()) else {
            continue;
        };
        if pgid == process_group {
            members.push(pid);
        }
    }
    members
}

fn fallback_members(process_group: u32) -> Vec<u32> {
    if process_group_alive(process_group) {
        vec![process_group]
    } else {
        Vec::new()
    }
}

fn process_group_alive(process_group: u32) -> bool {
    use nix::errno::Errno;
    use nix::sys::signal::kill;
    use nix::unistd::Pid;
    // Plan invariant 4: EPERM counts as alive (the process group exists,
    // we just can't signal it).
    match kill(Pid::from_raw(-(process_group as i32)), None) {
        Ok(()) => true,
        Err(Errno::EPERM) => true,
        Err(_) => false,
    }
}

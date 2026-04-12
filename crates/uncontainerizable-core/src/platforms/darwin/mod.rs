//! macOS platform implementation.
//!
//! Identity preemption is argv[0] tagging (see `argv_tag`): best-effort
//! because argv[0] is not a kernel primitive like cgroup v2 or a named
//! Job Object. The three-stage quit ladder (see `stages`) escalates from
//! Apple Events through SIGTERM and SIGKILL, walking the whole process
//! tree via `kill_tree`.

use async_trait::async_trait;
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
        walk_tree(self.core.pid).await
    }

    async fn is_empty(&self) -> std::result::Result<bool, StageError> {
        Ok(self.members().await.is_empty())
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

/// Walk the process tree rooted at `root_pid` by parsing `ps -axo pid,ppid`.
/// Returns every descendant PID (plus the root itself) that is still alive
/// according to a subsequent `kill(pid, 0)` probe. Returns an empty vector
/// if the root PID is not alive.
async fn walk_tree(root_pid: u32) -> Vec<u32> {
    if !pid_alive(root_pid) {
        return Vec::new();
    }
    let Ok(output) = Command::new("ps")
        .args(["-axo", "pid=,ppid="])
        .output()
        .await
    else {
        return vec![root_pid];
    };
    if !output.status.success() {
        return vec![root_pid];
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let mut children_of: std::collections::HashMap<u32, Vec<u32>> =
        std::collections::HashMap::new();
    for line in stdout.lines() {
        let mut iter = line.split_whitespace();
        let (Some(pid_str), Some(ppid_str)) = (iter.next(), iter.next()) else {
            continue;
        };
        let (Ok(pid), Ok(ppid)) = (pid_str.parse::<u32>(), ppid_str.parse::<u32>()) else {
            continue;
        };
        children_of.entry(ppid).or_default().push(pid);
    }

    let mut tree = vec![root_pid];
    let mut queue = vec![root_pid];
    while let Some(cur) = queue.pop() {
        if let Some(children) = children_of.get(&cur) {
            for &child in children {
                if !tree.contains(&child) {
                    tree.push(child);
                    queue.push(child);
                }
            }
        }
    }

    tree.into_iter().filter(|p| pid_alive(*p)).collect()
}

fn pid_alive(pid: u32) -> bool {
    use nix::errno::Errno;
    use nix::sys::signal::kill;
    use nix::unistd::Pid;
    // Plan invariant 4: EPERM counts as alive (process exists, we just
    // can't signal it).
    match kill(Pid::from_raw(pid as i32), None) {
        Ok(()) => true,
        Err(Errno::EPERM) => true,
        Err(_) => false,
    }
}

//! macOS platform implementation.
//!
//! Two launch routes, chosen purely by the shape of the `command` path:
//!
//! * **Launch Services** (`bundle::is_app_bundle(command)` returns true):
//!   `command` is a `.app` directory. We shell out to
//!   `open -n -F -a <bundle>` so the app is properly registered with
//!   `launchservicesd` (Dock, Apple Events, fresh saved state).
//!
//!   **Identity on this path is a singleton switch, not a scoping
//!   key.** When the caller passes `identity`, every running instance
//!   of the bundle's main executable gets SIGKILLed before `open`
//!   fires, regardless of which identity (if any) launched them and
//!   regardless of whether the prior launch went through this
//!   supervisor at all. The `identity` string is only consulted to
//!   decide whether to preempt; it is not used to filter the kill
//!   set. This means two concurrent LS launches of the same `.app`
//!   with different identities cannot coexist; the second call
//!   will terminate the first.
//!
//!   The reason identity can't act as a scope here is that LS
//!   rewrites argv at spawn time (so argv[0] tagging is gone) and
//!   macOS `ps -E` does not surface the environment to non-root
//!   callers (so env-var tagging can't be read back either). `ps
//!   comm=` is the only reliable "is this a running instance of my
//!   bundle" signal that survives an external launch, and it carries
//!   no per-launch metadata. Callers that need multiple concurrent
//!   instances of the same bundle with separate identities should
//!   pass the inner executable path (e.g.
//!   `/Applications/Foo.app/Contents/MacOS/Foo`) to fall onto the
//!   direct-exec route, which does support per-identity argv[0]
//!   tagging, at the cost of losing LS integration.
//!
//!   PID is resolved after `open` returns by polling `ps` for a new
//!   process whose executable matches the bundle's main exec.
//! * **Direct exec** (everything else): `tokio::process::Command` fires
//!   `posix_spawn` on the given path, and we place the spawned root in
//!   its own process group via `pre_exec(setpgid(0, 0))` so helpers
//!   remain targetable even if the root exits and macOS reparents
//!   descendants to launchd. Identity preemption uses argv[0] tagging
//!   (see `argv_tag`). Best-effort: argv[0] is not a kernel primitive
//!   like cgroup v2 or a named Job Object.
//!
//! The three-stage quit ladder (see `stages`) is shared across both
//! routes and escalates from Apple Events through SIGTERM and SIGKILL.

use std::path::Path;
use std::time::{Duration, Instant};

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
pub mod bundle;
pub mod lsappinfo;
pub mod probe;
pub mod stages;

/// Time budget for resolving the PID of a Launch Services-launched
/// app. `open -n -F -a` exits ~immediately after LS accepts the
/// request, but the actual app process shows up in `ps` with a
/// variable lag (50ms-1s on warm launches, longer on cold).
const LS_PID_RESOLVE_TIMEOUT: Duration = Duration::from_secs(3);

pub async fn spawn(app: &App, command: &str, opts: ContainOptions) -> Result<Box<dyn Container>> {
    if bundle::is_app_bundle(command) {
        spawn_bundle(app, command, opts).await
    } else {
        spawn_direct(app, command, opts).await
    }
}

async fn spawn_direct(
    app: &App,
    command: &str,
    opts: ContainOptions,
) -> Result<Box<dyn Container>> {
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

async fn spawn_bundle(
    _app: &App,
    command: &str,
    opts: ContainOptions,
) -> Result<Box<dyn Container>> {
    let bundle_path = Path::new(command)
        .canonicalize()
        .map_err(|e| Error::Spawn {
            command: command.into(),
            source: e,
        })?;
    let info = bundle::read_info(&bundle_path).await?;

    // Identity on this path is a singleton switch, not a scoping key:
    // when `identity` is Some we SIGKILL every running instance of
    // the bundle's main executable before `open` fires, ignoring the
    // actual identity value. Two concurrent LS launches of the same
    // `.app` with distinct identities cannot coexist; the second
    // call will terminate the first. See the module-level docs for
    // the full rationale (argv and env-based per-launch tagging both
    // fall off under LS, leaving `ps comm=` as the only "is this my
    // bundle running" signal that survives an external launch).
    //
    // `baseline` must be a fresh snapshot taken immediately before
    // `open` fires, after any best-effort preemption has settled. That
    // way PID resolution treats any survivor or concurrent external
    // launch as "already present" and only attaches to a PID that
    // actually appeared after this spawn request.
    if let Some(ident) = opts.identity.as_deref() {
        identity::validate(ident)?;
        bundle::kill_existing_bundle_instances(&info.executable_path).await;
    }
    let baseline = bundle::snapshot_bundle_pids(&info.executable_path).await;

    let mut cmd = Command::new("open");
    cmd.args(["-n", "-F", "-a", &bundle_path.to_string_lossy()]);
    if !opts.args.is_empty() {
        cmd.arg("--args").args(&opts.args);
    }
    cmd.envs(opts.env.iter().cloned());
    if let Some(cwd) = &opts.cwd {
        cmd.current_dir(cwd);
    }
    let status = cmd.status().await.map_err(|e| Error::Spawn {
        command: "open".into(),
        source: e,
    })?;
    if !status.success() {
        return Err(Error::Bundle(crate::error::BundleError::OpenFailed {
            bundle_path: bundle_path.clone(),
            exit_code: status.code(),
        }));
    }

    let deadline = Instant::now() + LS_PID_RESOLVE_TIMEOUT;
    let pid = bundle::resolve_new_pid(&info.executable_path, &baseline, deadline, &info.bundle_id)
        .await?;

    let mut probe = probe::capture_probe_with_bundle(pid, Some(info.bundle_id.clone())).await?;
    // Probe executable_path from `ps comm=` truncates on macOS, so
    // prefer the Info.plist-derived path we already have.
    probe.executable_path = Some(info.executable_path.clone());

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

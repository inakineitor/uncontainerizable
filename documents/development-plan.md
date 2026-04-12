# Plan: `uncontainerizable`

A Rust core library plus Node bindings for running programs in a sandbox-style container, with graceful teardown, tree-aware signalling, per-app crash-state adapters, and **identity-based singleton enforcement** — "kill any previous instance with this identity before spawning a new one."

The name reflects the scope honestly: this is for programs that *can't* be put in real Linux containers (browsers, GUI apps, tools that need the user's window server, keychain, display, etc.) but still need supervisor-style lifecycle management. If you can put it in Docker or a Linux namespace, you should. This library is for the rest.

---

## Goals and non-goals

**Goals:**

- Graceful escalation: try the platform's polite quit channel first, escalate if ignored.
- Walk the process tree; don't leave helper processes behind.
- Run per-app adapters to suppress "didn't shut down correctly" dialogs after force-kill.
- Identity-based singleton: at most one container per identity is alive at any time. Kill predecessors on spawn.
- File-less wherever possible. Use kernel primitives (cgroups on Linux, Job Objects on Windows) as the source of truth.
- Async Rust core, usable from Rust directly and from Node via napi-rs bindings.

**Non-goals:**

- Actual isolation (namespaces, seccomp, landlock — use a real container runtime).
- Resume/reattach to previous containers — we kill old instances, we don't adopt them.
- Cross-supervisor coordination beyond "last one wins on a given identity."
- Tracking all spawns ever — only the currently-claimed identities.

---

## Core concepts

**Container.** Handle returned by `contain()`. Owns the spawned root PID, the probe captured at spawn time, the adapter list, and on Linux a cgroup. Async trait with three concrete implementations — `BasicContainer`, `DarwinContainer`, `LinuxContainer` — plus a fourth for Windows (`WindowsContainer`) that holds a Job Object handle.

**Probe.** Identity info (bundle ID, executable path, platform) captured at spawn time so adapter matching survives post-mortem.

**Adapter.** Async trait with five optional lifecycle hooks (`before_quit`, `before_stage`, `after_stage`, `after_quit`, `clear_crash_state`). Hook errors are collected, never interrupt escalation.

**Staged quit.** Platform-specific escalation ladder:

- **macOS**: `aevt/quit` (root only; AppKit handles fanout) → `SIGTERM` (tree) → `SIGKILL` (tree)
- **Windows**: `WM_CLOSE` → `TerminateJobObject`
- **Linux**: `SIGTERM` → `SIGKILL`, both delivered race-free through cgroup freeze

**Identity.** Opaque string chosen by the caller. Two containers with the same identity cannot coexist — spawning one kills the other. Identity is namespaced via an app prefix to avoid collisions between unrelated libraries using `uncontainerizable`.

**Infallible destroy.** `destroy()` collects errors from every step into the result; never throws.

---

## Invariants

1. Adapter hooks are advisory. Errors collected, never interrupt escalation.
2. Probe captured at spawn, not teardown.
3. `clear_crash_state` runs only after `reached_terminal_stage`.
4. `EPERM` from `kill(pid, 0)` means alive.
5. All matching adapters run each hook, in order.
6. `is_empty()` is authoritative about the container, not just the root.
7. Apple-event stage targets root only; AppKit handles helper fanout.
8. Identity-based preemption uses kernel primitives on Linux and Windows; macOS is best-effort via argv[0] tagging.
9. `contain()` with an identity does not fail if no predecessor exists. It does fail if the predecessor exists and can't be killed (e.g., EPERM).

---

## Identity mechanism

A caller creates an `App` once per application, which holds a prefix:

```rust
let app = App::new("com.example.my-supervisor");
let container = app.contain("chromium", ContainOptions {
    identity: Some("browser-main".into()),
    ..Default::default()
}).await?;
```

Internally, the identity becomes `com.example.my-supervisor:browser-main`. That string maps per-platform:

- **Linux**: cgroup path `{session}/uncontainerizable/{identity}/`. Deterministic. `mkdir` is atomic, so concurrent spawns serialize via `EEXIST`. Before creating, enumerate any existing cgroup at the path: freeze, SIGKILL all members, rmdir, then create fresh.
- **Windows**: Named Job Object `Local\uncontainerizable-{identity}`. `OpenJobObjectW` finds the predecessor; `TerminateJobObject` kills every process inside; close handle; `CreateJobObjectW` creates a fresh one for this spawn. `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE` is set so the current supervisor's death also kills its children, no cleanup needed on next startup.
- **macOS**: argv[0] tagging — spawn with `argv[0] = "uncontainerizable:{identity}/original-name"`. Before spawn, `ps -A -o pid,command` for matching tags, kill via `kill_tree` with SIGKILL. Best-effort — the caller can opt out of tagging if it interferes with the managed program.

Omitting `identity` skips all of this — the container launches unconditionally with no preemption.

---

## Repository layout

```
uncontainerizable/
├── .github/
│   └── workflows/
│       ├── ci.yml
│       └── release.yml
├── .changeset/
│   └── config.json
├── crates/
│   ├── uncontainerizable-core/
│   │   ├── Cargo.toml
│   │   └── src/
│   │       ├── lib.rs
│   │       ├── app.rs              # App + ContainOptions + public contain()
│   │       ├── identity.rs         # identity normalization, prefix logic
│   │       ├── error.rs
│   │       ├── probe.rs
│   │       ├── container.rs        # Container trait, ContainerCore, impls
│   │       ├── adapter.rs          # Adapter trait + built-in adapters
│   │       └── platforms/
│   │           ├── mod.rs
│   │           ├── darwin/
│   │           │   ├── mod.rs
│   │           │   ├── stages.rs
│   │           │   ├── lsappinfo.rs
│   │           │   ├── argv_tag.rs    # argv[0] tag scanning and killing
│   │           │   └── probe.rs
│   │           ├── win32/
│   │           │   ├── mod.rs
│   │           │   ├── stages.rs
│   │           │   ├── job_object.rs  # named Job Object management
│   │           │   └── probe.rs
│   │           └── linux/
│   │               ├── mod.rs
│   │               ├── stages.rs
│   │               ├── cgroup.rs       # wrapper over cgroups-rs
│   │               └── probe.rs
│   └── uncontainerizable-node/
│       ├── Cargo.toml
│       ├── build.rs
│       ├── package.json
│       ├── npm/
│       └── src/
│           ├── lib.rs
│           ├── app.rs
│           ├── container.rs
│           ├── adapter_bridge.rs
│           └── errors.rs
├── packages/
│   └── uncontainerizable/
│       ├── package.json
│       ├── tsconfig.json
│       ├── tsdown.config.ts
│       ├── src/
│       │   ├── index.ts
│       │   ├── adapters/
│       │   │   ├── index.ts
│       │   │   ├── chromium.ts
│       │   │   ├── firefox.ts
│       │   │   └── appkit.ts
│       │   └── types.ts
│       └── test/
├── biome.json
├── lefthook.yml
├── pnpm-workspace.yaml
├── package.json
├── Cargo.toml
├── rust-toolchain.toml
└── README.md
```

---

## Cargo workspace

**Root `Cargo.toml`:**

```toml
[workspace]
members = ["crates/uncontainerizable-core", "crates/uncontainerizable-node"]
resolver = "2"

[workspace.package]
version = "0.1.0"
edition = "2021"
license = "MIT"
repository = "https://github.com/YOUR_ORG/uncontainerizable"
rust-version = "1.80"

[workspace.dependencies]
async-trait = "0.1"
futures = "0.3"
thiserror = "1"
tokio = { version = "1", features = ["rt-multi-thread", "macros", "process", "fs", "time", "sync"] }
kill_tree = { version = "0.2", features = ["tokio"] }
serde = { version = "1", features = ["derive"] }
serde_json = "1"
tracing = "0.1"
```

**`crates/uncontainerizable-core/Cargo.toml`:**

```toml
[package]
name = "uncontainerizable-core"
version.workspace = true
edition.workspace = true
license.workspace = true
repository.workspace = true
description = "Graceful process lifecycle for programs that can't be containerized"

[dependencies]
async-trait.workspace = true
futures.workspace = true
thiserror.workspace = true
tokio.workspace = true
kill_tree.workspace = true
serde.workspace = true
serde_json.workspace = true
tracing.workspace = true

[target.'cfg(target_os = "linux")'.dependencies]
cgroups-rs = "0.4"
nix = { version = "0.27", features = ["signal", "process"] }

[target.'cfg(windows)'.dependencies]
windows = { version = "0.56", features = [
    "Win32_Foundation",
    "Win32_Security",
    "Win32_System_JobObjects",
    "Win32_System_Threading",
    "Win32_System_ProcessStatus",
    "Win32_UI_WindowsAndMessaging",
] }

[dev-dependencies]
anyhow = "1"
tokio = { version = "1", features = ["full"] }
```

**`rust-toolchain.toml`:**

```toml
[toolchain]
channel = "1.80"
components = ["rustfmt", "clippy"]
```

---

## Core: errors

```rust
// crates/uncontainerizable-core/src/error.rs
use thiserror::Error;

#[derive(Debug, Error)]
pub enum Error {
    #[error("platform {0} is not supported")]
    UnsupportedPlatform(String),

    #[error("failed to spawn {command}")]
    Spawn {
        command: String,
        #[source] source: std::io::Error,
    },

    #[error("failed to preempt prior instance of identity {identity:?}")]
    Preempt {
        identity: String,
        #[source] source: Box<dyn std::error::Error + Send + Sync>,
    },

    #[error("invalid identity: {0}")]
    InvalidIdentity(String),

    #[error(transparent)]
    Probe(#[from] ProbeError),

    #[error("container has already been destroyed")]
    AlreadyDestroyed,

    #[error(transparent)]
    Platform(#[from] PlatformError),
}

#[derive(Debug, Error)]
pub enum ProbeError {
    #[error(transparent)] Io(#[from] std::io::Error),
    #[error("subprocess {command} failed: {message}")]
    Subprocess { command: String, message: String },
}

#[derive(Debug, Error)]
pub enum PlatformError {
    #[cfg(target_os = "linux")]
    #[error(transparent)] Cgroup(#[from] CgroupError),

    #[cfg(windows)]
    #[error(transparent)] JobObject(#[from] JobObjectError),

    #[error(transparent)] Io(#[from] std::io::Error),
    #[error("platform subsystem error: {0}")] Other(String),
}

#[cfg(target_os = "linux")]
#[derive(Debug, Error)]
pub enum CgroupError {
    #[error("cgroup v2 not mounted at /sys/fs/cgroup")] NotV2,
    #[error("no write access to {path} — session likely missing Delegate=yes")]
    NotDelegated { path: String },
    #[error("invalid cgroup name: {0:?}")] InvalidName(String),
    #[error("cgroup {path} did not reach frozen={target} within {timeout_ms}ms")]
    FreezeTimeout { path: String, target: bool, timeout_ms: u64 },
    #[error(transparent)] Io(#[from] std::io::Error),
    #[error(transparent)] Upstream(#[from] cgroups_rs::error::Error),
}

#[cfg(windows)]
#[derive(Debug, Error)]
pub enum JobObjectError {
    #[error("failed to open or create job object {name:?}")]
    OpenOrCreate { name: String, #[source] source: windows::core::Error },
    #[error("failed to terminate predecessor job")]
    TerminatePredecessor { #[source] source: windows::core::Error },
    #[error("failed to assign process to job")]
    AssignProcess { #[source] source: windows::core::Error },
}

#[derive(Debug, Error)]
pub enum StageError {
    #[error("missing probe field: {0}")] MissingProbe(&'static str),
    #[error(transparent)] KillTree(#[from] kill_tree::Error),
    #[cfg(target_os = "linux")]
    #[error(transparent)] Cgroup(#[from] CgroupError),
    #[cfg(windows)]
    #[error(transparent)] JobObject(#[from] JobObjectError),
    #[error(transparent)] Io(#[from] std::io::Error),
    #[cfg(unix)]
    #[error("signal delivery failed")] Signal(#[from] nix::errno::Errno),
}

#[derive(Debug, Error)]
pub enum AdapterError {
    #[error(transparent)] Io(#[from] std::io::Error),
    #[error(transparent)] Serde(#[from] serde_json::Error),
    #[error("adapter callback failed: {0}")] Callback(String),
}

pub type Result<T> = std::result::Result<T, Error>;
```

---

## Core: identity

```rust
// crates/uncontainerizable-core/src/identity.rs
use crate::error::Error;

/// Characters allowed in identities after combination with the prefix.
/// Restrictive by design — this must be safe as a cgroup dirname, a Windows
/// object name, and an argv[0] tag.
fn is_valid_identity_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-' | ':')
}

pub fn validate(raw: &str) -> Result<(), Error> {
    if raw.is_empty() || raw.len() > 200 {
        return Err(Error::InvalidIdentity(format!(
            "must be 1..=200 chars, got {}", raw.len(),
        )));
    }
    if !raw.chars().all(is_valid_identity_char) {
        return Err(Error::InvalidIdentity(
            "allowed chars: a-z A-Z 0-9 . _ - :".into(),
        ));
    }
    Ok(())
}

pub fn combine(prefix: &str, identity: &str) -> String {
    format!("{}:{}", prefix, identity)
}
```

---

## Core: App

```rust
// crates/uncontainerizable-core/src/app.rs
use crate::adapter::Adapter;
use crate::container::Container;
use crate::error::{Error, Result};
use crate::identity;
use std::sync::Arc;

/// An application handle. The prefix namespaces identities so two unrelated
/// libraries using uncontainerizable don't collide.
///
/// Conventionally a reverse-DNS string, like "com.example.my-supervisor".
#[derive(Debug, Clone)]
pub struct App {
    prefix: String,
}

impl App {
    pub fn new(prefix: impl Into<String>) -> Result<Self> {
        let prefix = prefix.into();
        identity::validate(&prefix)?;
        Ok(Self { prefix })
    }

    pub fn prefix(&self) -> &str { &self.prefix }

    /// Spawn a container. If `opts.identity` is set, any previous instance
    /// with the same (prefix, identity) pair is killed before this one
    /// launches.
    pub async fn contain(
        &self, command: &str, opts: ContainOptions,
    ) -> Result<Box<dyn Container>> {
        crate::platforms::spawn(self, command, opts).await
    }
}

#[derive(Debug, Default)]
pub struct ContainOptions {
    pub args: Vec<String>,
    pub env: Vec<(String, String)>,
    pub cwd: Option<std::path::PathBuf>,
    pub adapters: Vec<Arc<dyn Adapter>>,

    /// Enable identity-based singleton enforcement. None = no preemption.
    pub identity: Option<String>,

    /// macOS only: if `false`, don't rewrite argv[0] with the identity tag,
    /// even if identity is set. On macOS without the tag we can't find and
    /// kill predecessors; this is for callers whose managed program breaks
    /// if argv[0] doesn't match the executable name.
    pub darwin_tag_argv0: bool,
}
```

---

## Core: platform dispatch

```rust
// crates/uncontainerizable-core/src/platforms/mod.rs
use crate::app::{App, ContainOptions};
use crate::container::Container;
use crate::error::Result;

pub async fn spawn(
    app: &App, command: &str, opts: ContainOptions,
) -> Result<Box<dyn Container>> {
    #[cfg(target_os = "linux")]
    return linux::spawn(app, command, opts).await;
    #[cfg(target_os = "macos")]
    return darwin::spawn(app, command, opts).await;
    #[cfg(windows)]
    return win32::spawn(app, command, opts).await;
    #[cfg(not(any(target_os = "linux", target_os = "macos", windows)))]
    Err(crate::error::Error::UnsupportedPlatform(
        std::env::consts::OS.to_string(),
    ))
}

#[cfg(target_os = "linux")] pub mod linux;
#[cfg(target_os = "macos")] pub mod darwin;
#[cfg(windows)] pub mod win32;
```

---

## Core: Linux spawn and cgroup

```rust
// crates/uncontainerizable-core/src/platforms/linux/mod.rs
use crate::app::{App, ContainOptions};
use crate::container::{Container, ContainerCore, LinuxContainer};
use crate::error::{Error, Result};
use crate::identity;
use crate::probe;
use std::sync::Arc;
use tokio::process::Command;
use self::cgroup::Cgroup;

pub mod cgroup;
pub mod stages;
pub mod probe as probe_impl;

pub async fn spawn(
    app: &App, command: &str, opts: ContainOptions,
) -> Result<Box<dyn Container>> {
    Cgroup::assert_available().await?;

    // Resolve the cgroup path. If identity is set, the path is deterministic
    // and we preempt any prior instance. Otherwise we create an anonymous
    // cgroup for this one-shot spawn.
    let cg = if let Some(ident) = &opts.identity {
        identity::validate(ident)?;
        let combined = identity::combine(app.prefix(), ident);
        Cgroup::open_or_replace(&combined).await
            .map_err(|e| Error::Preempt {
                identity: combined,
                source: Box::new(e),
            })?
    } else {
        Cgroup::create_anonymous().await?
    };

    let child = Command::new(command)
        .args(&opts.args)
        .envs(opts.env.iter().cloned())
        .spawn()
        .map_err(|e| Error::Spawn { command: command.into(), source: e })?;

    let pid = child.id().ok_or_else(|| Error::Spawn {
        command: command.into(),
        source: std::io::Error::new(std::io::ErrorKind::Other, "no pid"),
    })?;
    cg.add(pid).await.map_err(Error::from)?;

    let probe = probe::capture_probe(pid).await?;
    let stages = stages::linux_stages();
    let core = ContainerCore::new(pid, probe, opts.adapters, stages);
    Ok(Box::new(LinuxContainer::new(core, cg)))
}
```

```rust
// crates/uncontainerizable-core/src/platforms/linux/cgroup.rs
use crate::error::CgroupError;
use cgroups_rs::{cgroup_builder::CgroupBuilder, hierarchies, Cgroup as CgCgroup, CgroupPid};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};
use tokio::fs;
use tokio::time::sleep;

const POLL_INTERVAL: Duration = Duration::from_millis(50);
const FREEZE_TIMEOUT: Duration = Duration::from_millis(1000);
const REAP_TIMEOUT: Duration = Duration::from_millis(500);

pub struct Cgroup {
    inner: CgCgroup,
    path: PathBuf,
}

impl Cgroup {
    pub async fn assert_available() -> Result<(), CgroupError> {
        if !hierarchies::is_cgroup2_unified_mode() { return Err(CgroupError::NotV2); }
        let current = current_cgroup_path().await?;
        let probe = current.join("cgroup.procs");
        let meta = fs::metadata(&probe).await?;
        if meta.permissions().readonly() {
            return Err(CgroupError::NotDelegated {
                path: current.display().to_string(),
            });
        }
        Ok(())
    }

    /// Create a cgroup at a deterministic path derived from the identity.
    /// If a cgroup exists there, kill its members and replace it.
    pub async fn open_or_replace(identity: &str) -> Result<Self, CgroupError> {
        let sanitized = sanitize_for_cgroup(identity)?;
        let base = current_cgroup_path().await?;
        let path = base.join("uncontainerizable").join(&sanitized);

        if fs::metadata(&path).await.is_ok() {
            kill_and_remove_cgroup(&path).await?;
        }

        let name = format!("uncontainerizable/{}", sanitized);
        let (inner, path) = spawn_build(&name).await?;
        Ok(Self { inner, path })
    }

    /// Create a cgroup with a unique anonymous name (for spawns with no identity).
    pub async fn create_anonymous() -> Result<Self, CgroupError> {
        let name = format!(
            "uncontainerizable/anon-{}-{}",
            std::process::id(),
            Instant::now().elapsed().as_nanos(),
        );
        let (inner, path) = spawn_build(&name).await?;
        Ok(Self { inner, path })
    }

    pub fn path(&self) -> &Path { &self.path }

    pub async fn add(&self, pid: u32) -> Result<(), CgroupError> {
        self.inner.add_task(CgroupPid::from(pid as u64))?;
        Ok(())
    }

    pub async fn freeze(&self) -> Result<(), CgroupError> {
        self.inner.freeze()?;
        self.wait_frozen(true).await
    }

    pub async fn thaw(&self) -> Result<(), CgroupError> {
        self.inner.thaw()?;
        self.wait_frozen(false).await
    }

    pub fn members(&self) -> Vec<u32> {
        self.inner.tasks().into_iter().map(|p| p.pid as u32).collect()
    }

    pub async fn is_empty(&self) -> Result<bool, CgroupError> {
        let raw = fs::read_to_string(self.path.join("cgroup.events")).await?;
        Ok(raw.lines().any(|l| l == "populated 0"))
    }

    pub async fn destroy(&self) -> Result<(), CgroupError> {
        self.inner.delete()?;
        Ok(())
    }

    async fn wait_frozen(&self, target: bool) -> Result<(), CgroupError> {
        let events = self.path.join("cgroup.events");
        let deadline = Instant::now() + FREEZE_TIMEOUT;
        while Instant::now() < deadline {
            let raw = fs::read_to_string(&events).await?;
            let frozen = raw.lines()
                .find_map(|l| l.strip_prefix("frozen "))
                .map(|v| v == "1").unwrap_or(false);
            if frozen == target { return Ok(()); }
            sleep(POLL_INTERVAL).await;
        }
        Err(CgroupError::FreezeTimeout {
            path: self.path.display().to_string(),
            target, timeout_ms: FREEZE_TIMEOUT.as_millis() as u64,
        })
    }
}

async fn spawn_build(name: &str) -> Result<(CgCgroup, PathBuf), CgroupError> {
    let name = name.to_string();
    tokio::task::spawn_blocking(move || {
        let hier = hierarchies::auto();
        let inner = CgroupBuilder::new(&name).build(hier)?;
        let path = inner.path().to_path_buf();
        Ok::<_, CgroupError>((inner, path))
    }).await.expect("cgroup build panicked")
}

async fn kill_and_remove_cgroup(path: &Path) -> Result<(), CgroupError> {
    use nix::sys::signal::{kill, Signal};
    use nix::unistd::Pid;

    let procs = fs::read_to_string(path.join("cgroup.procs")).await?;
    let pids: Vec<u32> = procs.lines().filter_map(|l| l.parse().ok()).collect();
    if pids.is_empty() {
        let _ = fs::remove_dir(path).await;
        return Ok(());
    }

    // Freeze → SIGKILL → thaw → wait for reap. No SIGTERM here: these are
    // predecessors being preempted; we're not being polite.
    fs::write(path.join("cgroup.freeze"), "1").await?;
    wait_populated(path, true).await.ok(); // best-effort
    for pid in &pids {
        let _ = kill(Pid::from_raw(*pid as i32), Signal::SIGKILL);
    }
    fs::write(path.join("cgroup.freeze"), "0").await?;

    let deadline = Instant::now() + REAP_TIMEOUT;
    while Instant::now() < deadline {
        let events = fs::read_to_string(path.join("cgroup.events")).await?;
        if events.lines().any(|l| l == "populated 0") { break; }
        sleep(POLL_INTERVAL).await;
    }
    let _ = fs::remove_dir(path).await;
    Ok(())
}

async fn wait_populated(path: &Path, target: bool) -> Result<(), CgroupError> {
    let deadline = Instant::now() + FREEZE_TIMEOUT;
    let events = path.join("cgroup.events");
    while Instant::now() < deadline {
        let raw = fs::read_to_string(&events).await?;
        let frozen = raw.lines()
            .find_map(|l| l.strip_prefix("frozen "))
            .map(|v| v == "1").unwrap_or(false);
        if frozen == target { return Ok(()); }
        sleep(POLL_INTERVAL).await;
    }
    Ok(())
}

async fn current_cgroup_path() -> Result<PathBuf, CgroupError> {
    let raw = fs::read_to_string("/proc/self/cgroup").await?;
    let rel = raw.lines().find_map(|l| l.strip_prefix("0::"))
        .ok_or(CgroupError::NotV2)?;
    Ok(Path::new("/sys/fs/cgroup").join(rel.trim_start_matches('/')))
}

fn sanitize_for_cgroup(identity: &str) -> Result<String, CgroupError> {
    // cgroup dir names can't contain '/'; our identities can contain ':' which
    // is fine in cgroup fs but ugly. Replace ':' with '.' for neatness.
    let s: String = identity.chars()
        .map(|c| if c == ':' { '.' } else { c }).collect();
    if s.is_empty() || s == "." || s == ".." {
        return Err(CgroupError::InvalidName(identity.into()));
    }
    Ok(s)
}
```

---

## Core: macOS spawn and argv[0] tagging

```rust
// crates/uncontainerizable-core/src/platforms/darwin/mod.rs
use crate::app::{App, ContainOptions};
use crate::container::{Container, ContainerCore, DarwinContainer};
use crate::error::{Error, Result};
use crate::identity;
use crate::probe;
use std::sync::Arc;
use tokio::process::Command;

pub mod stages;
pub mod lsappinfo;
pub mod argv_tag;
pub mod probe as probe_impl;

pub async fn spawn(
    app: &App, command: &str, opts: ContainOptions,
) -> Result<Box<dyn Container>> {
    let tagged_argv0 = if let Some(ident) = &opts.identity {
        identity::validate(ident)?;
        let combined = identity::combine(app.prefix(), ident);
        argv_tag::kill_existing(&combined).await
            .map_err(|e| Error::Preempt {
                identity: combined.clone(),
                source: Box::new(e),
            })?;
        if opts.darwin_tag_argv0 {
            Some(argv_tag::format(&combined, command))
        } else { None }
    } else { None };

    let mut cmd = Command::new(command);
    cmd.args(&opts.args).envs(opts.env.iter().cloned());
    if let Some(argv0) = &tagged_argv0 {
        cmd.arg0(argv0);
    }
    let child = cmd.spawn()
        .map_err(|e| Error::Spawn { command: command.into(), source: e })?;

    let pid = child.id().ok_or_else(|| Error::Spawn {
        command: command.into(),
        source: std::io::Error::new(std::io::ErrorKind::Other, "no pid"),
    })?;
    let probe = probe::capture_probe(pid).await?;
    let stages = stages::darwin_stages();
    let core = ContainerCore::new(pid, probe, opts.adapters, stages);
    Ok(Box::new(DarwinContainer::new(core)))
}
```

```rust
// crates/uncontainerizable-core/src/platforms/darwin/argv_tag.rs
use kill_tree::tokio::kill_tree_with_config;
use kill_tree::Config;
use tokio::process::Command;

const TAG_PREFIX: &str = "uncontainerizable";

pub fn format(identity: &str, command: &str) -> String {
    let base = std::path::Path::new(command)
        .file_name()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| command.to_string());
    format!("{}:{}/{}", TAG_PREFIX, identity, base)
}

/// Find and kill any running processes whose argv[0] starts with our tag
/// for this identity.
pub async fn kill_existing(identity: &str) -> Result<(), std::io::Error> {
    let needle = format!("{}:{}/", TAG_PREFIX, identity);
    let output = Command::new("ps")
        .args(["-A", "-o", "pid=,command="])
        .output().await?;
    if !output.status.success() { return Ok(()); }

    let stdout = String::from_utf8_lossy(&output.stdout);
    for line in stdout.lines() {
        let line = line.trim();
        let Some((pid_str, rest)) = line.split_once(char::is_whitespace) else {
            continue;
        };
        if !rest.trim_start().starts_with(&needle) { continue; }
        let Ok(pid) = pid_str.trim().parse::<u32>() else { continue };
        let _ = kill_tree_with_config(pid, &Config {
            signal: "SIGKILL".into(),
            include_target: true,
            ..Default::default()
        }).await;
    }
    Ok(())
}
```

---

## Core: Windows spawn and Job Object

```rust
// crates/uncontainerizable-core/src/platforms/win32/mod.rs
use crate::app::{App, ContainOptions};
use crate::container::{Container, ContainerCore, WindowsContainer};
use crate::error::{Error, Result};
use crate::identity;
use crate::probe;
use self::job_object::WorkspaceJob;
use std::sync::Arc;
use tokio::process::Command;

pub mod stages;
pub mod job_object;
pub mod probe as probe_impl;

pub async fn spawn(
    app: &App, command: &str, opts: ContainOptions,
) -> Result<Box<dyn Container>> {
    let job = if let Some(ident) = &opts.identity {
        identity::validate(ident)?;
        let combined = identity::combine(app.prefix(), ident);
        WorkspaceJob::replace(&combined)
            .map_err(|e| Error::Preempt {
                identity: combined,
                source: Box::new(e),
            })?
    } else {
        WorkspaceJob::anonymous()?
    };

    let mut cmd = Command::new(command);
    cmd.args(&opts.args).envs(opts.env.iter().cloned());
    let child = cmd.spawn()
        .map_err(|e| Error::Spawn { command: command.into(), source: e })?;

    let pid = child.id().ok_or_else(|| Error::Spawn {
        command: command.into(),
        source: std::io::Error::new(std::io::ErrorKind::Other, "no pid"),
    })?;
    job.assign_pid(pid)?;

    let probe = probe::capture_probe(pid).await?;
    let stages = stages::win32_stages();
    let core = ContainerCore::new(pid, probe, opts.adapters, stages);
    Ok(Box::new(WindowsContainer::new(core, job)))
}
```

```rust
// crates/uncontainerizable-core/src/platforms/win32/job_object.rs
use crate::error::JobObjectError;
use windows::core::PCWSTR;
use windows::Win32::Foundation::{CloseHandle, HANDLE};
use windows::Win32::System::JobObjects::*;
use windows::Win32::System::Threading::{OpenProcess, PROCESS_SET_QUOTA, PROCESS_TERMINATE};

pub struct WorkspaceJob {
    handle: HANDLE,
    name: Option<String>,
}

impl WorkspaceJob {
    /// Open or create a named job object. If a predecessor exists, terminate
    /// every process in it, then create a fresh job with the same name.
    pub fn replace(identity: &str) -> Result<Self, JobObjectError> {
        let wide_name = encode_name(identity);
        let name_pcwstr = PCWSTR(wide_name.as_ptr());

        // Try to open an existing job with this name.
        let existing = unsafe {
            OpenJobObjectW(JOB_OBJECT_ALL_ACCESS.0, false, name_pcwstr).ok()
        };
        if let Some(handle) = existing {
            let _ = unsafe { TerminateJobObject(handle, 1) };
            let _ = unsafe { CloseHandle(handle) };
        }

        // Now create a fresh one with the same name.
        let handle = unsafe {
            CreateJobObjectW(None, name_pcwstr)
                .map_err(|e| JobObjectError::OpenOrCreate {
                    name: identity.to_string(), source: e,
                })?
        };
        configure_kill_on_close(handle)?;
        Ok(Self { handle, name: Some(identity.to_string()) })
    }

    pub fn anonymous() -> Result<Self, JobObjectError> {
        let handle = unsafe {
            CreateJobObjectW(None, PCWSTR::null())
                .map_err(|e| JobObjectError::OpenOrCreate {
                    name: "<anonymous>".into(), source: e,
                })?
        };
        configure_kill_on_close(handle)?;
        Ok(Self { handle, name: None })
    }

    pub fn assign_pid(&self, pid: u32) -> Result<(), JobObjectError> {
        unsafe {
            let process = OpenProcess(PROCESS_SET_QUOTA | PROCESS_TERMINATE, false, pid)
                .map_err(|e| JobObjectError::AssignProcess { source: e })?;
            let result = AssignProcessToJobObject(self.handle, process);
            let _ = CloseHandle(process);
            result.map_err(|e| JobObjectError::AssignProcess { source: e })
        }
    }

    pub fn handle(&self) -> HANDLE { self.handle }

    pub fn terminate_all(&self) -> Result<(), JobObjectError> {
        unsafe { TerminateJobObject(self.handle, 1)
            .map_err(|e| JobObjectError::TerminatePredecessor { source: e }) }
    }
}

impl Drop for WorkspaceJob {
    fn drop(&mut self) {
        unsafe { let _ = CloseHandle(self.handle); }
    }
}

fn configure_kill_on_close(handle: HANDLE) -> Result<(), JobObjectError> {
    let mut info = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
    info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
    unsafe {
        SetInformationJobObject(
            handle,
            JobObjectExtendedLimitInformation,
            &info as *const _ as *const _,
            std::mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
        ).map_err(|e| JobObjectError::OpenOrCreate {
            name: "<configure>".into(), source: e,
        })
    }
}

fn encode_name(identity: &str) -> Vec<u16> {
    let name = format!("Local\\uncontainerizable-{}", identity);
    name.encode_utf16().chain(std::iter::once(0)).collect()
}
```

---

## Core: adapter and container (abridged — full source same shape as prior iterations)

```rust
// crates/uncontainerizable-core/src/adapter.rs
use crate::container::Container;
use crate::error::AdapterError;
use crate::probe::Probe;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StageResult {
    pub stage_name: String, pub index: usize,
    pub exited: bool, pub is_terminal: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QuitResult {
    pub exited_at_stage: Option<String>,
    pub reached_terminal_stage: bool,
}

#[async_trait]
pub trait Adapter: Send + Sync {
    fn name(&self) -> &str;
    fn matches(&self, probe: &Probe) -> bool;
    async fn before_quit(&self, _: &Probe, _: &dyn Container) -> Result<(), AdapterError> { Ok(()) }
    async fn before_stage(&self, _: &Probe, _: &str, _: &dyn Container) -> Result<(), AdapterError> { Ok(()) }
    async fn after_stage(&self, _: &Probe, _: &StageResult, _: &dyn Container) -> Result<(), AdapterError> { Ok(()) }
    async fn after_quit(&self, _: &Probe, _: &QuitResult, _: &dyn Container) -> Result<(), AdapterError> { Ok(()) }
    async fn clear_crash_state(&self, _: &Probe) -> Result<(), AdapterError> { Ok(()) }
}
```

The `Container` trait, `ContainerCore`, `BasicContainer`, `DarwinContainer`, `LinuxContainer`, and `WindowsContainer` are structured as in prior iterations: `ContainerCore` holds the `quit`/`destroy` loop; each subclass adds its platform-specific state (`Cgroup`, `WorkspaceJob`, tree-walking `members()`) and overrides `is_empty`, `members`, and `destroy_resources` accordingly. Full listing omitted for space; the behavior is exactly what we've been discussing throughout.

---

## Node bindings

**`crates/uncontainerizable-node/Cargo.toml`:**

```toml
[package]
name = "uncontainerizable-node"
version.workspace = true
edition.workspace = true
license.workspace = true

[lib]
crate-type = ["cdylib"]

[dependencies]
uncontainerizable-core = { path = "../uncontainerizable-core" }
napi = { version = "2", default-features = false, features = ["napi9", "async", "serde-json"] }
napi-derive = "2"
tokio = { workspace = true, features = ["full"] }
async-trait.workspace = true
futures.workspace = true
thiserror.workspace = true
serde.workspace = true
serde_json.workspace = true

[build-dependencies]
napi-build = "2"

[profile.release]
lto = true
```

**`crates/uncontainerizable-node/package.json`:**

```json
{
  "name": "@uncontainerizable/native",
  "version": "0.1.0",
  "description": "Native bindings for uncontainerizable",
  "main": "index.js",
  "types": "index.d.ts",
  "license": "MIT",
  "napi": {
    "name": "uncontainerizable",
    "triples": {
      "defaults": false,
      "additional": [
        "x86_64-apple-darwin",
        "aarch64-apple-darwin",
        "x86_64-unknown-linux-gnu",
        "aarch64-unknown-linux-gnu",
        "x86_64-unknown-linux-musl",
        "aarch64-unknown-linux-musl",
        "x86_64-pc-windows-msvc",
        "aarch64-pc-windows-msvc"
      ]
    }
  },
  "engines": { "node": ">=18" },
  "scripts": {
    "artifacts": "napi artifacts",
    "build": "napi build --platform --release",
    "build:debug": "napi build --platform",
    "prepublishOnly": "napi prepublish -t npm",
    "universal": "napi universal",
    "version": "napi version"
  },
  "devDependencies": { "@napi-rs/cli": "^3.0.0" },
  "files": ["index.js", "index.d.ts"]
}
```

**`crates/uncontainerizable-node/src/lib.rs`:**

```rust
#![deny(clippy::all)]
use napi::bindgen_prelude::*;
use napi_derive::napi;
use std::sync::Arc;

mod adapter_bridge;
mod app;
mod container;
mod errors;

#[napi(object)]
pub struct NodeContainOptions {
    pub args: Option<Vec<String>>,
    pub env: Option<std::collections::HashMap<String, String>>,
    pub cwd: Option<String>,
    pub identity: Option<String>,
    pub darwin_tag_argv0: Option<bool>,
    pub adapters: Option<Vec<adapter_bridge::JsAdapter>>,
}

#[napi]
pub struct NodeApp { inner: uncontainerizable_core::App }

#[napi]
impl NodeApp {
    #[napi(constructor)]
    pub fn new(prefix: String) -> Result<Self> {
        let inner = uncontainerizable_core::App::new(prefix)
            .map_err(errors::to_napi)?;
        Ok(Self { inner })
    }

    #[napi]
    pub async fn contain(
        &self, command: String, opts: Option<NodeContainOptions>,
    ) -> Result<container::NodeContainer> {
        let opts = opts.unwrap_or_default();
        let core_opts = uncontainerizable_core::ContainOptions {
            args: opts.args.unwrap_or_default(),
            env: opts.env.unwrap_or_default().into_iter().collect(),
            cwd: opts.cwd.map(std::path::PathBuf::from),
            identity: opts.identity,
            darwin_tag_argv0: opts.darwin_tag_argv0.unwrap_or(true),
            adapters: opts.adapters.unwrap_or_default()
                .into_iter()
                .map(adapter_bridge::into_dynamic)
                .map(|a| Arc::new(a) as Arc<dyn uncontainerizable_core::Adapter>)
                .collect(),
        };
        let container = self.inner.contain(&command, core_opts).await
            .map_err(errors::to_napi)?;
        Ok(container::NodeContainer::wrap(container))
    }
}
```

The adapter bridge uses `ThreadsafeFunction` to let JS callbacks be invoked from Rust async tasks. Caching of `matches` results per (adapter, probe) avoids per-hook round-trips to the JS thread.

---

## Tooling

**`pnpm-workspace.yaml`:**

```yaml
packages:
  - "crates/uncontainerizable-node"
  - "packages/*"
```

**Root `package.json`:**

```json
{
  "name": "uncontainerizable-monorepo",
  "private": true,
  "type": "module",
  "packageManager": "pnpm@9.12.0",
  "scripts": {
    "build": "pnpm -r build",
    "build:native": "pnpm --filter @uncontainerizable/native build",
    "build:ts": "pnpm --filter uncontainerizable build",
    "test": "pnpm -r test",
    "lint": "pnpm dlx ultracite check",
    "lint:fix": "pnpm dlx ultracite write",
    "release": "changeset publish",
    "version": "changeset version"
  },
  "devDependencies": {
    "@changesets/cli": "^2.27.0",
    "@biomejs/biome": "^1.9.0",
    "lefthook": "^1.7.0",
    "typescript": "^6.0.0",
    "ultracite": "^4.0.0"
  }
}
```

**`biome.json`:**

```json
{
  "$schema": "https://biomejs.dev/schemas/1.9.0/schema.json",
  "extends": ["ultracite"],
  "files": {
    "ignore": ["**/dist/**", "**/target/**", "**/node_modules/**", "**/*.d.ts"]
  }
}
```

**`lefthook.yml`:**

```yaml
pre-commit:
  parallel: true
  commands:
    ultracite:
      glob: "*.{ts,tsx,js,jsx,json}"
      run: pnpm dlx ultracite check {staged_files}
    rust-fmt:
      glob: "*.rs"
      run: cargo fmt --check -- {staged_files}
    rust-clippy:
      run: cargo clippy --workspace --all-targets -- -D warnings

commit-msg:
  commands:
    conventional:
      run: pnpm dlx commitlint --edit {1}
```

**`.changeset/config.json`:**

```json
{
  "$schema": "https://unpkg.com/@changesets/config@3.0.0/schema.json",
  "changelog": "@changesets/cli/changelog",
  "commit": false,
  "access": "public",
  "baseBranch": "main",
  "updateInternalDependencies": "patch",
  "linked": [["uncontainerizable", "@uncontainerizable/native"]],
  "ignore": []
}
```

**`packages/uncontainerizable/package.json`:**

```json
{
  "name": "uncontainerizable",
  "version": "0.1.0",
  "description": "Graceful process lifecycle for programs that can't be containerized",
  "type": "module",
  "exports": {
    ".": {
      "types": "./dist/index.d.ts",
      "import": "./dist/index.js",
      "require": "./dist/index.cjs"
    },
    "./adapters": {
      "types": "./dist/adapters/index.d.ts",
      "import": "./dist/adapters/index.js",
      "require": "./dist/adapters/index.cjs"
    }
  },
  "files": ["dist"],
  "scripts": {
    "build": "tsdown",
    "dev": "tsdown --watch",
    "test": "vitest run",
    "typecheck": "tsc --noEmit"
  },
  "dependencies": {
    "@uncontainerizable/native": "workspace:*"
  },
  "devDependencies": {
    "@types/node": "^22.0.0",
    "tsdown": "^0.9.0",
    "typescript": "^6.0.0",
    "vitest": "^2.1.0"
  }
}
```

**`packages/uncontainerizable/tsdown.config.ts`:**

```typescript
import { defineConfig } from "tsdown";

export default defineConfig({
  entry: ["src/index.ts", "src/adapters/index.ts"],
  format: ["esm", "cjs"],
  dts: true,
  sourcemap: true,
  clean: true,
  target: "node18",
  platform: "node",
  external: ["@uncontainerizable/native"],
});
```

**`packages/uncontainerizable/tsconfig.json`:**

```json
{
  "compilerOptions": {
    "target": "ES2022",
    "module": "NodeNext",
    "moduleResolution": "NodeNext",
    "strict": true,
    "esModuleInterop": true,
    "skipLibCheck": true,
    "isolatedModules": true,
    "declaration": true,
    "sourceMap": true,
    "rootDir": "src",
    "outDir": "dist"
  },
  "include": ["src/**/*.ts"],
  "exclude": ["dist", "node_modules", "test"]
}
```

**`packages/uncontainerizable/src/index.ts`:**

```typescript
import { NodeApp } from "@uncontainerizable/native";
import type { ContainOptions, Container } from "./types.js";

export class App {
  #inner: NodeApp;
  constructor(prefix: string) { this.#inner = new NodeApp(prefix); }

  async contain(command: string, options: ContainOptions = {}): Promise<Container> {
    return this.#inner.contain(command, options) as Promise<Container>;
  }
}

export { chromium, firefox, appkit, defaultAdapters } from "./adapters/index.js";
export type * from "./types.js";
```

---

## CI and release

**`.github/workflows/ci.yml`:**

```yaml
name: CI
on:
  push: { branches: [main] }
  pull_request: {}

jobs:
  lint:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - uses: pnpm/action-setup@v4
      - uses: actions/setup-node@v4
        with: { node-version: 20, cache: pnpm }
      - run: pnpm install --frozen-lockfile
      - run: pnpm lint
      - uses: dtolnay/rust-toolchain@stable
        with: { components: rustfmt, clippy }
      - run: cargo fmt --check
      - run: cargo clippy --workspace --all-targets -- -D warnings

  test-rust:
    strategy:
      matrix: { os: [ubuntu-latest, macos-latest, windows-latest] }
    runs-on: ${{ matrix.os }}
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@stable
      - run: cargo test --workspace

  test-node:
    strategy:
      matrix: { os: [ubuntu-latest, macos-latest, windows-latest] }
    runs-on: ${{ matrix.os }}
    steps:
      - uses: actions/checkout@v4
      - uses: pnpm/action-setup@v4
      - uses: actions/setup-node@v4
        with: { node-version: 20, cache: pnpm }
      - uses: dtolnay/rust-toolchain@stable
      - run: pnpm install --frozen-lockfile
      - run: pnpm build:native
      - run: pnpm build:ts
      - run: pnpm test
```

**`.github/workflows/release.yml`:**

```yaml
name: Release
on:
  push:
    branches: [main]

permissions:
  contents: write
  pull-requests: write
  id-token: write

jobs:
  build-native:
    strategy:
      fail-fast: false
      matrix:
        settings:
          - host: macos-latest
            target: x86_64-apple-darwin
          - host: macos-latest
            target: aarch64-apple-darwin
          - host: ubuntu-latest
            target: x86_64-unknown-linux-gnu
          - host: ubuntu-latest
            target: aarch64-unknown-linux-gnu
            docker: ghcr.io/napi-rs/napi-rs/nodejs-rust:lts-debian-aarch64
          - host: ubuntu-latest
            target: x86_64-unknown-linux-musl
            docker: ghcr.io/napi-rs/napi-rs/nodejs-rust:lts-alpine
          - host: ubuntu-latest
            target: aarch64-unknown-linux-musl
            docker: ghcr.io/napi-rs/napi-rs/nodejs-rust:lts-alpine-aarch64
          - host: windows-latest
            target: x86_64-pc-windows-msvc
          - host: windows-latest
            target: aarch64-pc-windows-msvc
    name: Build ${{ matrix.settings.target }}
    runs-on: ${{ matrix.settings.host }}
    steps:
      - uses: actions/checkout@v4
      - uses: pnpm/action-setup@v4
      - uses: actions/setup-node@v4
        with: { node-version: 20, cache: pnpm }
      - uses: dtolnay/rust-toolchain@stable
        with: { targets: ${{ matrix.settings.target }} }
      - run: pnpm install --frozen-lockfile

      - name: Build (native)
        if: ${{ !matrix.settings.docker }}
        run: pnpm --filter @uncontainerizable/native build -- --target ${{ matrix.settings.target }}

      - name: Build (docker)
        if: ${{ matrix.settings.docker }}
        uses: addnab/docker-run-action@v3
        with:
          image: ${{ matrix.settings.docker }}
          options: --user 0:0 -v ${{ github.workspace }}/.cargo-cache/git:/root/.cargo/git -v ${{ github.workspace }}/.cargo-cache/registry:/root/.cargo/registry -v ${{ github.workspace }}:/build -w /build
          run: |
            pnpm install --frozen-lockfile
            pnpm --filter @uncontainerizable/native build -- --target ${{ matrix.settings.target }}

      - uses: actions/upload-artifact@v4
        with:
          name: bindings-${{ matrix.settings.target }}
          path: crates/uncontainerizable-node/*.node
          if-no-files-found: error

  release:
    needs: build-native
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
        with: { fetch-depth: 0 }
      - uses: pnpm/action-setup@v4
      - uses: actions/setup-node@v4
        with: { node-version: 20, cache: pnpm, registry-url: https://registry.npmjs.org }
      - run: pnpm install --frozen-lockfile

      - uses: actions/download-artifact@v4
        with: { path: crates/uncontainerizable-node/artifacts }

      - name: Move artifacts into per-target npm packages
        working-directory: crates/uncontainerizable-node
        run: pnpm artifacts

      - name: Build TS wrapper
        run: pnpm build:ts

      - name: Create release PR or publish
        uses: changesets/action@v1
        with:
          publish: pnpm release
          version: pnpm version
        env:
          GITHUB_TOKEN: ${{ secrets.GITHUB_TOKEN }}
          NPM_TOKEN: ${{ secrets.NPM_TOKEN }}
          NPM_CONFIG_PROVENANCE: "true"
```

---

## Usage

**Rust:**

```rust
use uncontainerizable_core::{App, ContainOptions};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let app = App::new("com.example.my-supervisor")?;

    // Kills any previous instance with identity "browser-main" before spawning.
    let container = app.contain("chromium", ContainOptions {
        args: vec!["https://example.com".into()],
        identity: Some("browser-main".into()),
        darwin_tag_argv0: true,
        ..Default::default()
    }).await?;

    // ... work ...

    let result = container.destroy(Default::default()).await;
    if !result.errors.is_empty() {
        eprintln!("partial teardown: {:?}", result.errors);
    }
    Ok(())
}
```

**TypeScript:**

```typescript
import { App, defaultAdapters, appkit } from "uncontainerizable";

const app = new App("com.example.my-supervisor");

const container = await app.contain("chromium", {
  args: ["https://example.com"],
  identity: "browser-main",
  adapters: defaultAdapters,
});

try {
  // ... workload ...
} finally {
  const result = await container.destroy();
  if (result.errors.length) console.warn("partial teardown:", result.errors);
}

// Kiosk use: also wipe AppKit saved state on macOS.
const kiosk = await app.contain("/Applications/Safari.app/Contents/MacOS/Safari", {
  identity: "kiosk",
  adapters: [...defaultAdapters, appkit],
});
```

---

## Design notes

**Why `App` with a prefix.** Identity is just a string, so two unrelated supervisors using the same identity would fight. The `App` construct forces namespacing at the type level: you can't spawn without an `App`, and an `App` requires a prefix. Convention is reverse-DNS. This eliminates accidental collision as a failure mode.

**Why identity is opt-in.** One-shot spawns don't need singleton enforcement and shouldn't pay for it. Omitting `identity` creates an anonymous cgroup / anonymous Job Object — the container still works, just without preemption. Identity is for the "there should only be one of these at a time" use case, which is common but not universal.

**Why file-less.** The kernel already tracks what we need. On Linux, the cgroup directory's existence and membership is authoritative — no extra marker files. On Windows, the named Job Object is authoritative and kernel-enforced; `KILL_ON_JOB_CLOSE` also handles the "our supervisor died" case for free. Only macOS lacks a primitive, and there we degrade to argv[0] tagging with a clear "best effort" disclaimer.

**Why `KILL_ON_JOB_CLOSE` on Windows.** Two benefits: (a) our current children die when we die, cleaning up without our intervention; (b) on the next supervisor startup, the predecessor job name may or may not still exist, but any processes that were in it are already gone. This means Windows doesn't need *any* preemption logic for the case of "previous supervisor crashed" — only for the case of "a different, still-running process is holding the job." The `TerminateJobObject` call handles that.

**Why argv[0] tagging on macOS rather than environment scanning.** macOS makes reading another process's environment variables fragile under SIP and hardened runtime. argv is always readable via `ps`. The tradeoff: some programs inspect argv[0] and misbehave. `darwin_tag_argv0: bool` defaults to `true` and lets callers opt out for problematic programs, at the cost of losing predecessor killing on macOS for that container.

**Why SIGKILL immediately when preempting predecessors.** They're orphans being replaced; politeness is for the current container's own teardown. Waiting for SIGTERM acknowledgment at spawn time adds 5+ seconds of latency per spawn for no benefit.

**Why `Error::Preempt` is its own variant.** A caller that sees a spawn fail because of preemption can retry or surface it differently than a spawn that fails because the binary doesn't exist. Typed errors let that matching happen without string parsing.

**Why three concrete container types.** Each platform has strictly different state and behavior — Linux has a cgroup, Windows has a Job Object, macOS has neither and walks the tree via `pgrep`/`kill_tree`. A base class with optional fields would require branches in every method; subclasses localize each platform's quirks to one file.

**Why the quit loop polls.** `kqueue` on macOS, `pidfd` on Linux, and `WaitForSingleObject` on Windows could eliminate polling, but each is platform-specific and we'd have three implementations to save ~50ms of latency per stage. The existing 50ms poll is fine and lets `is_empty` be the authoritative signal across platforms uniformly.

---

## What's explicitly out of scope

- **Container resume / reattach.** Previous instances are killed, not adopted. Containers from a prior supervisor are garbage by design.
- **Resource limits** (memory, CPU, pid count). Worth adding on top of the `Cgroup` type later; orthogonal to lifecycle.
- **Cross-supervisor coordination.** Two supervisors using the same `App` prefix and the same identity will preempt each other aggressively. "Last one wins" is the contract; shared coordination would be a separate, larger feature.
- **Namespace / seccomp / landlock isolation.** If you need that, you need a real container runtime. `uncontainerizable` is for things that can't be put in one.
- **macOS equivalents to cgroups or job objects.** They don't exist in the OS; we use the best available approximations and document the gap.
- **Python and Go bindings.** The Rust core is callable from them but no bindings are shipped in v0.1.

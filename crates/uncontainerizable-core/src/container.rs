//! `Container` trait + shared quit/destroy machinery.
//!
//! Three public items carry most of the weight:
//!
//! * `Container` (trait) is the platform-agnostic handle. Implementations
//!   only need to supply accessors to the `ContainerCore` plus
//!   platform-specific `members`, `is_empty`, and `destroy_resources`; the
//!   shared `quit` and `destroy` methods are provided by default.
//! * `ContainerCore` is the struct every concrete container holds: root pid,
//!   probe, adapter list, stage ladder, and teardown flags.
//! * `Stage` (trait) represents one step in the platform-specific quit
//!   ladder. Stages are executed sequentially; after each one the loop polls
//!   `is_empty` until the container has drained or `max_wait` elapses.
//!
//! `BasicContainer` is a thin non-platform-specific implementation. It
//! checks the root PID's liveness with a signal-0 probe on Unix and
//! `OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION)` on Windows. Useful for
//! tests and as a generic fallback; real platform containers subclass with
//! cgroup/Job-Object/tree-walk state.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::adapter::Adapter;
use crate::error::{Error, StageError};
use crate::probe::Probe;

const POLL_INTERVAL: Duration = Duration::from_millis(50);

/// Options for `Container::quit`.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct QuitOptions {
    /// Override the default `max_wait` for specific stages by name.
    pub stage_timeouts: HashMap<String, Duration>,
    /// Stages to skip entirely (by name).
    pub skip_stages: Vec<String>,
}

/// Options for `Container::destroy`.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct DestroyOptions {
    pub quit: QuitOptions,
}

/// Outcome of a single stage execution.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct StageResult {
    pub stage_name: String,
    pub index: usize,
    /// `true` if the container was empty after this stage's execute + poll.
    pub exited: bool,
    pub is_terminal: bool,
}

/// Outcome of the full quit ladder.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct QuitResult {
    /// Name of the stage that emptied the container, or
    /// `Some("before:<stage_name>")` if the container was already empty
    /// before a stage ran.
    pub exited_at_stage: Option<String>,
    pub reached_terminal_stage: bool,
    pub stage_results: Vec<StageResult>,
    /// Adapter hook errors collected during the loop. Each entry is
    /// `"<adapter_name>: <message>"`.
    pub adapter_errors: Vec<String>,
}

/// Outcome of a full teardown. Contains the `quit` phase's result plus any
/// errors surfaced while releasing platform resources.
#[derive(Debug)]
pub struct DestroyResult {
    pub quit: QuitResult,
    /// `destroy()` is infallible by design: surfacing errors means the caller
    /// can log/metric but never has to handle a thrown exception in a
    /// `finally` block. This list gathers every recoverable failure.
    pub errors: Vec<Error>,
}

/// A step in a container's quit ladder.
#[async_trait]
pub trait Stage: Send + Sync {
    fn name(&self) -> &str;
    /// `true` if reaching this stage counts as "fully torn down" regardless
    /// of whether the container actually emptied within `max_wait`.
    fn is_terminal(&self) -> bool;
    /// Per-stage timeout, overridable via `QuitOptions::stage_timeouts`.
    fn max_wait(&self) -> Duration;
    /// Execute the stage's side-effect (signal delivery, Apple-event, etc.)
    /// without waiting for the container to drain: the shared loop handles
    /// post-execute polling.
    async fn execute(&self, container: &dyn Container) -> Result<(), StageError>;
}

/// Shared teardown state every concrete `Container` holds.
pub struct ContainerCore {
    pub pid: u32,
    pub probe: Probe,
    pub adapters: Vec<Arc<dyn Adapter>>,
    pub stages: Vec<Arc<dyn Stage>>,
    pub reached_terminal: bool,
    pub destroyed: bool,
}

impl ContainerCore {
    pub fn new(
        pid: u32,
        probe: Probe,
        adapters: Vec<Arc<dyn Adapter>>,
        stages: Vec<Arc<dyn Stage>>,
    ) -> Self {
        Self {
            pid,
            probe,
            adapters,
            stages,
            reached_terminal: false,
            destroyed: false,
        }
    }
}

#[async_trait]
pub trait Container: Send + Sync {
    fn core(&self) -> &ContainerCore;
    fn core_mut(&mut self) -> &mut ContainerCore;

    /// Enumerate every PID the container considers a member, including the
    /// root. Source of truth is platform-specific (cgroup.procs on Linux,
    /// Job Object process list on Windows, tree walk on Darwin).
    async fn members(&self) -> Vec<u32>;

    /// `true` iff the container has no running members. Authoritative over
    /// observing the root PID alone: helper processes can outlive the root.
    async fn is_empty(&self) -> Result<bool, StageError>;

    /// Release the platform resource (cgroup, job object handle, tag). Any
    /// errors are surfaced through `DestroyResult.errors`.
    async fn destroy_resources(&mut self) -> Vec<Error>;

    /// Run the staged quit ladder. Adapter hook errors are collected, not
    /// propagated; stage execution errors are propagated immediately.
    ///
    /// Concrete implementations almost always delegate to `run_quit`.
    async fn quit(&mut self, opts: QuitOptions) -> Result<QuitResult, Error>;

    /// `quit` plus resource release. Infallible: every failure surfaces in
    /// `DestroyResult.errors`.
    ///
    /// Concrete implementations almost always delegate to `run_destroy`.
    async fn destroy(&mut self, opts: DestroyOptions) -> DestroyResult;

    fn pid(&self) -> u32 {
        self.core().pid
    }

    fn probe(&self) -> &Probe {
        &self.core().probe
    }
}

/// Shared teardown loop. Platform `Container` impls forward their `quit`
/// method here. Exposed publicly because the trait's `quit` method can't
/// carry a default body: coercing `&mut Self` to `&mut dyn Container`
/// requires `Self: Sized`, which would remove it from the trait-object
/// vtable and break `Box<dyn Container>` callers.
pub async fn run_quit(c: &mut dyn Container, opts: QuitOptions) -> Result<QuitResult, Error> {
    if c.core().destroyed {
        return Err(Error::AlreadyDestroyed);
    }

    // Snapshot inputs up front so we don't juggle simultaneous borrows of
    // `c` while running the loop. Arc clones are refcount bumps, cheap.
    let probe = c.core().probe.clone();
    let matching: Vec<Arc<dyn Adapter>> = c
        .core()
        .adapters
        .iter()
        .filter(|a| a.matches(&probe))
        .cloned()
        .collect();
    let stages: Vec<Arc<dyn Stage>> = c.core().stages.clone();

    let mut adapter_errors: Vec<String> = Vec::new();

    for adapter in &matching {
        if let Err(e) = adapter.before_quit(&probe, &*c).await {
            adapter_errors.push(format!("{}: {e}", adapter.name()));
        }
    }

    let mut stage_results: Vec<StageResult> = Vec::new();
    let mut exited_at_stage: Option<String> = None;
    let mut reached_terminal = false;

    for (idx, stage) in stages.iter().enumerate() {
        if opts.skip_stages.iter().any(|s| s == stage.name()) {
            continue;
        }

        if c.is_empty().await? {
            if exited_at_stage.is_none() {
                exited_at_stage = Some(format!("before:{}", stage.name()));
            }
            break;
        }

        for adapter in &matching {
            if let Err(e) = adapter.before_stage(&probe, stage.name(), &*c).await {
                adapter_errors.push(format!("{}: {e}", adapter.name()));
            }
        }

        stage.execute(&*c).await?;

        let max_wait = opts
            .stage_timeouts
            .get(stage.name())
            .copied()
            .unwrap_or_else(|| stage.max_wait());

        let start = Instant::now();
        let mut exited = false;
        while start.elapsed() < max_wait {
            if c.is_empty().await? {
                exited = true;
                break;
            }
            tokio::time::sleep(POLL_INTERVAL).await;
        }

        let result = StageResult {
            stage_name: stage.name().to_string(),
            index: idx,
            exited,
            is_terminal: stage.is_terminal(),
        };

        for adapter in &matching {
            if let Err(e) = adapter.after_stage(&probe, &result, &*c).await {
                adapter_errors.push(format!("{}: {e}", adapter.name()));
            }
        }

        if exited && exited_at_stage.is_none() {
            exited_at_stage = Some(stage.name().to_string());
        }

        let was_terminal = result.is_terminal;
        stage_results.push(result);

        if was_terminal {
            reached_terminal = true;
            break;
        }
        if exited {
            break;
        }
    }

    if reached_terminal {
        c.core_mut().reached_terminal = true;
        for adapter in &matching {
            if let Err(e) = adapter.clear_crash_state(&probe).await {
                adapter_errors.push(format!("{}: {e}", adapter.name()));
            }
        }
    }

    let quit_result_so_far = QuitResult {
        exited_at_stage: exited_at_stage.clone(),
        reached_terminal_stage: reached_terminal,
        stage_results: stage_results.clone(),
        adapter_errors: adapter_errors.clone(),
    };

    for adapter in &matching {
        if let Err(e) = adapter.after_quit(&probe, &quit_result_so_far, &*c).await {
            adapter_errors.push(format!("{}: {e}", adapter.name()));
        }
    }

    Ok(QuitResult {
        exited_at_stage,
        reached_terminal_stage: reached_terminal,
        stage_results,
        adapter_errors,
    })
}

/// Generic, non-platform-specific container. `members` and `is_empty` are
/// driven by a simple liveness probe on the root PID: `kill(pid, 0)` on
/// Unix, `OpenProcess` on Windows. Good enough for tests and as a generic
/// fallback; real platform containers hold a cgroup / job object / tree
/// walker alongside the core.
pub struct BasicContainer {
    core: ContainerCore,
}

impl BasicContainer {
    pub fn new(core: ContainerCore) -> Self {
        Self { core }
    }
}

#[async_trait]
impl Container for BasicContainer {
    fn core(&self) -> &ContainerCore {
        &self.core
    }

    fn core_mut(&mut self) -> &mut ContainerCore {
        &mut self.core
    }

    async fn members(&self) -> Vec<u32> {
        if pid_alive(self.core.pid) {
            vec![self.core.pid]
        } else {
            Vec::new()
        }
    }

    async fn is_empty(&self) -> Result<bool, StageError> {
        Ok(!pid_alive(self.core.pid))
    }

    async fn destroy_resources(&mut self) -> Vec<Error> {
        Vec::new()
    }

    async fn quit(&mut self, opts: QuitOptions) -> Result<QuitResult, Error> {
        run_quit(self, opts).await
    }

    async fn destroy(&mut self, opts: DestroyOptions) -> DestroyResult {
        run_destroy(self, opts).await
    }
}

/// Shared destroy loop; see `run_quit` for why this is a free function
/// rather than a trait default method.
pub async fn run_destroy(c: &mut dyn Container, opts: DestroyOptions) -> DestroyResult {
    if c.core().destroyed {
        return DestroyResult {
            quit: QuitResult {
                exited_at_stage: None,
                reached_terminal_stage: c.core().reached_terminal,
                stage_results: Vec::new(),
                adapter_errors: Vec::new(),
            },
            errors: vec![Error::AlreadyDestroyed],
        };
    }
    let mut errors: Vec<Error> = Vec::new();
    let quit = match c.quit(opts.quit).await {
        Ok(q) => q,
        Err(e) => {
            errors.push(e);
            QuitResult {
                exited_at_stage: None,
                reached_terminal_stage: false,
                stage_results: Vec::new(),
                adapter_errors: Vec::new(),
            }
        }
    };
    let mut resource_errors = c.destroy_resources().await;
    errors.append(&mut resource_errors);
    c.core_mut().destroyed = true;
    DestroyResult { quit, errors }
}

#[cfg(unix)]
fn pid_alive(pid: u32) -> bool {
    use nix::errno::Errno;
    use nix::sys::signal::kill;
    use nix::unistd::Pid;
    // Per plan invariant 4: EPERM from kill(pid, 0) means the PID exists
    // but belongs to another user; still counts as alive.
    match kill(Pid::from_raw(pid as i32), None) {
        Ok(()) => true,
        Err(Errno::EPERM) => true,
        Err(_) => false,
    }
}

#[cfg(windows)]
fn pid_alive(pid: u32) -> bool {
    use windows::Win32::Foundation::CloseHandle;
    use windows::Win32::System::Threading::{OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION};
    // SAFETY: passing a PID to OpenProcess is always sound; we close the
    // returned handle on success.
    unsafe {
        match OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) {
            Ok(handle) => {
                let _ = CloseHandle(handle);
                true
            }
            Err(_) => false,
        }
    }
}

#[cfg(not(any(unix, windows)))]
fn pid_alive(_pid: u32) -> bool {
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::AdapterError;
    use crate::probe::SupportedPlatform;
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

    fn probe() -> Probe {
        Probe::new(
            std::process::id(),
            SupportedPlatform::current().unwrap_or(SupportedPlatform::Linux),
        )
    }

    // A `Container` that lies about membership: `is_empty` returns `true`
    // only after `reveal_empty` is called. Used to drive the quit loop
    // deterministically without real processes.
    struct MockContainer {
        core: ContainerCore,
        reveal_empty: Arc<AtomicBool>,
    }

    #[async_trait]
    impl Container for MockContainer {
        fn core(&self) -> &ContainerCore {
            &self.core
        }
        fn core_mut(&mut self) -> &mut ContainerCore {
            &mut self.core
        }
        async fn members(&self) -> Vec<u32> {
            if self.reveal_empty.load(Ordering::SeqCst) {
                Vec::new()
            } else {
                vec![self.core.pid]
            }
        }
        async fn is_empty(&self) -> Result<bool, StageError> {
            Ok(self.reveal_empty.load(Ordering::SeqCst))
        }
        async fn destroy_resources(&mut self) -> Vec<Error> {
            Vec::new()
        }
        async fn quit(&mut self, opts: QuitOptions) -> Result<QuitResult, Error> {
            run_quit(self, opts).await
        }
        async fn destroy(&mut self, opts: DestroyOptions) -> DestroyResult {
            run_destroy(self, opts).await
        }
    }

    /// Stage that flips `reveal_empty` to true the first time it runs.
    /// Subsequent runs are no-ops.
    struct FlipStage {
        name: String,
        terminal: bool,
        max_wait: Duration,
        reveal: Arc<AtomicBool>,
        executed: Arc<AtomicUsize>,
    }

    #[async_trait]
    impl Stage for FlipStage {
        fn name(&self) -> &str {
            &self.name
        }
        fn is_terminal(&self) -> bool {
            self.terminal
        }
        fn max_wait(&self) -> Duration {
            self.max_wait
        }
        async fn execute(&self, _c: &dyn Container) -> Result<(), StageError> {
            self.executed.fetch_add(1, Ordering::SeqCst);
            self.reveal.store(true, Ordering::SeqCst);
            Ok(())
        }
    }

    /// Stage that never causes the container to drain.
    struct NoopStage {
        name: String,
        terminal: bool,
        max_wait: Duration,
        executed: Arc<AtomicUsize>,
    }

    #[async_trait]
    impl Stage for NoopStage {
        fn name(&self) -> &str {
            &self.name
        }
        fn is_terminal(&self) -> bool {
            self.terminal
        }
        fn max_wait(&self) -> Duration {
            self.max_wait
        }
        async fn execute(&self, _c: &dyn Container) -> Result<(), StageError> {
            self.executed.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }
    }

    struct CountingAdapter {
        name: String,
        before_quit_calls: Arc<AtomicUsize>,
        before_stage_calls: Arc<AtomicUsize>,
        after_stage_calls: Arc<AtomicUsize>,
        after_quit_calls: Arc<AtomicUsize>,
        clear_crash_state_calls: Arc<AtomicUsize>,
    }

    impl CountingAdapter {
        fn new(name: &str) -> Self {
            Self {
                name: name.into(),
                before_quit_calls: Arc::new(AtomicUsize::new(0)),
                before_stage_calls: Arc::new(AtomicUsize::new(0)),
                after_stage_calls: Arc::new(AtomicUsize::new(0)),
                after_quit_calls: Arc::new(AtomicUsize::new(0)),
                clear_crash_state_calls: Arc::new(AtomicUsize::new(0)),
            }
        }
    }

    #[async_trait]
    impl Adapter for CountingAdapter {
        fn name(&self) -> &str {
            &self.name
        }
        fn matches(&self, _probe: &Probe) -> bool {
            true
        }
        async fn before_quit(&self, _: &Probe, _: &dyn Container) -> Result<(), AdapterError> {
            self.before_quit_calls.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }
        async fn before_stage(
            &self,
            _: &Probe,
            _: &str,
            _: &dyn Container,
        ) -> Result<(), AdapterError> {
            self.before_stage_calls.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }
        async fn after_stage(
            &self,
            _: &Probe,
            _: &StageResult,
            _: &dyn Container,
        ) -> Result<(), AdapterError> {
            self.after_stage_calls.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }
        async fn after_quit(
            &self,
            _: &Probe,
            _: &QuitResult,
            _: &dyn Container,
        ) -> Result<(), AdapterError> {
            self.after_quit_calls.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }
        async fn clear_crash_state(&self, _: &Probe) -> Result<(), AdapterError> {
            self.clear_crash_state_calls.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }
    }

    struct FailingAdapter {
        name: String,
    }

    #[async_trait]
    impl Adapter for FailingAdapter {
        fn name(&self) -> &str {
            &self.name
        }
        fn matches(&self, _probe: &Probe) -> bool {
            true
        }
        async fn before_quit(&self, _: &Probe, _: &dyn Container) -> Result<(), AdapterError> {
            Err(AdapterError::Callback("synthetic failure".into()))
        }
    }

    struct NonMatchingAdapter {
        name: String,
        before_quit_calls: Arc<AtomicUsize>,
    }

    #[async_trait]
    impl Adapter for NonMatchingAdapter {
        fn name(&self) -> &str {
            &self.name
        }
        fn matches(&self, _probe: &Probe) -> bool {
            false
        }
        async fn before_quit(&self, _: &Probe, _: &dyn Container) -> Result<(), AdapterError> {
            self.before_quit_calls.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }
    }

    #[tokio::test]
    async fn quit_loop_exits_at_first_draining_stage() {
        let reveal = Arc::new(AtomicBool::new(false));
        let flip_executed = Arc::new(AtomicUsize::new(0));
        let stage = Arc::new(FlipStage {
            name: "flip".into(),
            terminal: false,
            max_wait: Duration::from_millis(500),
            reveal: reveal.clone(),
            executed: flip_executed.clone(),
        });
        let core = ContainerCore::new(1, probe(), vec![], vec![stage]);
        let mut c = MockContainer {
            core,
            reveal_empty: reveal,
        };
        let result = c.quit(QuitOptions::default()).await.unwrap();
        assert_eq!(result.exited_at_stage.as_deref(), Some("flip"));
        assert!(!result.reached_terminal_stage);
        assert_eq!(result.stage_results.len(), 1);
        assert!(result.stage_results[0].exited);
        assert_eq!(flip_executed.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn quit_loop_reaches_terminal_stage_when_container_ignores_earlier_signals() {
        let reveal = Arc::new(AtomicBool::new(false));
        let noop_executed = Arc::new(AtomicUsize::new(0));
        let flip_executed = Arc::new(AtomicUsize::new(0));
        let noop = Arc::new(NoopStage {
            name: "noop".into(),
            terminal: false,
            max_wait: Duration::from_millis(80),
            executed: noop_executed.clone(),
        });
        let flip = Arc::new(FlipStage {
            name: "flip-terminal".into(),
            terminal: true,
            max_wait: Duration::from_millis(500),
            reveal: reveal.clone(),
            executed: flip_executed.clone(),
        });
        let core = ContainerCore::new(1, probe(), vec![], vec![noop, flip]);
        let mut c = MockContainer {
            core,
            reveal_empty: reveal,
        };
        let result = c.quit(QuitOptions::default()).await.unwrap();
        assert_eq!(noop_executed.load(Ordering::SeqCst), 1);
        assert_eq!(flip_executed.load(Ordering::SeqCst), 1);
        assert!(result.reached_terminal_stage);
        assert_eq!(result.exited_at_stage.as_deref(), Some("flip-terminal"));
        assert_eq!(result.stage_results.len(), 2);
        assert!(!result.stage_results[0].exited);
        assert!(result.stage_results[1].exited);
    }

    #[tokio::test]
    async fn quit_loop_skips_stages_in_skip_list() {
        let reveal = Arc::new(AtomicBool::new(false));
        let noop_executed = Arc::new(AtomicUsize::new(0));
        let flip_executed = Arc::new(AtomicUsize::new(0));
        let noop = Arc::new(NoopStage {
            name: "skip-me".into(),
            terminal: false,
            max_wait: Duration::from_millis(80),
            executed: noop_executed.clone(),
        });
        let flip = Arc::new(FlipStage {
            name: "do-me".into(),
            terminal: true,
            max_wait: Duration::from_millis(500),
            reveal: reveal.clone(),
            executed: flip_executed.clone(),
        });
        let core = ContainerCore::new(1, probe(), vec![], vec![noop, flip]);
        let mut c = MockContainer {
            core,
            reveal_empty: reveal,
        };
        let opts = QuitOptions {
            skip_stages: vec!["skip-me".into()],
            ..Default::default()
        };
        c.quit(opts).await.unwrap();
        assert_eq!(noop_executed.load(Ordering::SeqCst), 0);
        assert_eq!(flip_executed.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn quit_loop_honors_per_stage_timeout_override() {
        let noop_executed = Arc::new(AtomicUsize::new(0));
        let noop = Arc::new(NoopStage {
            name: "slow".into(),
            terminal: true,
            max_wait: Duration::from_millis(2000),
            executed: noop_executed.clone(),
        });
        let core = ContainerCore::new(1, probe(), vec![], vec![noop]);
        let mut c = MockContainer {
            core,
            reveal_empty: Arc::new(AtomicBool::new(false)),
        };
        let opts = QuitOptions {
            stage_timeouts: {
                let mut m = HashMap::new();
                m.insert("slow".into(), Duration::from_millis(30));
                m
            },
            ..Default::default()
        };
        let started = Instant::now();
        c.quit(opts).await.unwrap();
        assert!(
            started.elapsed() < Duration::from_millis(500),
            "timeout override should short-circuit the default 2s"
        );
    }

    #[tokio::test]
    async fn adapter_hooks_fire_in_expected_order_for_matching_adapter() {
        let reveal = Arc::new(AtomicBool::new(false));
        let stage = Arc::new(FlipStage {
            name: "flip".into(),
            terminal: true,
            max_wait: Duration::from_millis(500),
            reveal: reveal.clone(),
            executed: Arc::new(AtomicUsize::new(0)),
        });
        let adapter = Arc::new(CountingAdapter::new("counter"));
        let before_quit_calls = adapter.before_quit_calls.clone();
        let before_stage_calls = adapter.before_stage_calls.clone();
        let after_stage_calls = adapter.after_stage_calls.clone();
        let after_quit_calls = adapter.after_quit_calls.clone();
        let clear_crash_state_calls = adapter.clear_crash_state_calls.clone();
        let core = ContainerCore::new(1, probe(), vec![adapter], vec![stage]);
        let mut c = MockContainer {
            core,
            reveal_empty: reveal,
        };
        c.quit(QuitOptions::default()).await.unwrap();
        assert_eq!(before_quit_calls.load(Ordering::SeqCst), 1);
        assert_eq!(before_stage_calls.load(Ordering::SeqCst), 1);
        assert_eq!(after_stage_calls.load(Ordering::SeqCst), 1);
        assert_eq!(after_quit_calls.load(Ordering::SeqCst), 1);
        assert_eq!(clear_crash_state_calls.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn adapter_errors_collect_but_do_not_abort() {
        let reveal = Arc::new(AtomicBool::new(false));
        let stage = Arc::new(FlipStage {
            name: "flip".into(),
            terminal: true,
            max_wait: Duration::from_millis(500),
            reveal: reveal.clone(),
            executed: Arc::new(AtomicUsize::new(0)),
        });
        let failing = Arc::new(FailingAdapter {
            name: "fails".into(),
        });
        let core = ContainerCore::new(1, probe(), vec![failing], vec![stage]);
        let mut c = MockContainer {
            core,
            reveal_empty: reveal,
        };
        let result = c.quit(QuitOptions::default()).await.unwrap();
        assert!(result.reached_terminal_stage);
        assert!(!result.adapter_errors.is_empty());
        assert!(result.adapter_errors[0].contains("fails"));
    }

    #[tokio::test]
    async fn non_matching_adapter_has_no_hooks_invoked() {
        let reveal = Arc::new(AtomicBool::new(false));
        let stage = Arc::new(FlipStage {
            name: "flip".into(),
            terminal: true,
            max_wait: Duration::from_millis(500),
            reveal: reveal.clone(),
            executed: Arc::new(AtomicUsize::new(0)),
        });
        let counter = Arc::new(NonMatchingAdapter {
            name: "ignored".into(),
            before_quit_calls: Arc::new(AtomicUsize::new(0)),
        });
        let before_quit_calls = counter.before_quit_calls.clone();
        let core = ContainerCore::new(1, probe(), vec![counter], vec![stage]);
        let mut c = MockContainer {
            core,
            reveal_empty: reveal,
        };
        c.quit(QuitOptions::default()).await.unwrap();
        assert_eq!(before_quit_calls.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn clear_crash_state_only_runs_after_terminal_stage_reached() {
        let reveal = Arc::new(AtomicBool::new(false));
        // Stage flips reveal on execute, so container empties at this
        // (non-terminal) stage. Terminal should NOT be reached.
        let stage = Arc::new(FlipStage {
            name: "flip".into(),
            terminal: false,
            max_wait: Duration::from_millis(500),
            reveal: reveal.clone(),
            executed: Arc::new(AtomicUsize::new(0)),
        });
        let adapter = Arc::new(CountingAdapter::new("counter"));
        let clear_calls = adapter.clear_crash_state_calls.clone();
        let core = ContainerCore::new(1, probe(), vec![adapter], vec![stage]);
        let mut c = MockContainer {
            core,
            reveal_empty: reveal,
        };
        let result = c.quit(QuitOptions::default()).await.unwrap();
        assert!(!result.reached_terminal_stage);
        assert_eq!(clear_calls.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn destroy_marks_destroyed_and_calls_destroy_resources() {
        let reveal = Arc::new(AtomicBool::new(false));
        let stage = Arc::new(FlipStage {
            name: "flip".into(),
            terminal: true,
            max_wait: Duration::from_millis(500),
            reveal: reveal.clone(),
            executed: Arc::new(AtomicUsize::new(0)),
        });
        let core = ContainerCore::new(1, probe(), vec![], vec![stage]);
        let mut c = MockContainer {
            core,
            reveal_empty: reveal,
        };
        let result = c.destroy(DestroyOptions::default()).await;
        assert!(result.quit.reached_terminal_stage);
        assert!(result.errors.is_empty());
        assert!(c.core().destroyed);
    }

    #[tokio::test]
    async fn destroy_is_idempotent_after_first_call() {
        let reveal = Arc::new(AtomicBool::new(false));
        let stage = Arc::new(FlipStage {
            name: "flip".into(),
            terminal: true,
            max_wait: Duration::from_millis(500),
            reveal: reveal.clone(),
            executed: Arc::new(AtomicUsize::new(0)),
        });
        let core = ContainerCore::new(1, probe(), vec![], vec![stage]);
        let mut c = MockContainer {
            core,
            reveal_empty: reveal,
        };
        c.destroy(DestroyOptions::default()).await;
        let second = c.destroy(DestroyOptions::default()).await;
        assert_eq!(second.errors.len(), 1);
        assert!(matches!(second.errors[0], Error::AlreadyDestroyed));
    }

    #[tokio::test]
    async fn quit_errors_with_already_destroyed_after_destroy() {
        let reveal = Arc::new(AtomicBool::new(false));
        let stage = Arc::new(FlipStage {
            name: "flip".into(),
            terminal: true,
            max_wait: Duration::from_millis(500),
            reveal: reveal.clone(),
            executed: Arc::new(AtomicUsize::new(0)),
        });
        let core = ContainerCore::new(1, probe(), vec![], vec![stage]);
        let mut c = MockContainer {
            core,
            reveal_empty: reveal,
        };
        c.destroy(DestroyOptions::default()).await;
        let err = c.quit(QuitOptions::default()).await.unwrap_err();
        assert!(matches!(err, Error::AlreadyDestroyed));
    }

    #[tokio::test]
    async fn basic_container_is_empty_when_pid_is_not_alive() {
        // Use a PID that's almost certainly not alive: u32::MAX - 1.
        let core = ContainerCore::new(u32::MAX - 1, probe(), vec![], vec![]);
        let c = BasicContainer::new(core);
        assert!(c.is_empty().await.unwrap());
    }

    #[tokio::test]
    async fn basic_container_is_populated_when_pid_is_current_process() {
        // Current process is definitely alive.
        let core = ContainerCore::new(std::process::id(), probe(), vec![], vec![]);
        let c = BasicContainer::new(core);
        assert!(!c.is_empty().await.unwrap());
        assert_eq!(c.members().await, vec![std::process::id()]);
    }
}

//! Integration tests for the Darwin platform.
//!
//! Spawns real processes via `App::contain` and asserts the staged quit
//! ladder behaves as documented. Gated to `target_os = "macos"`.

#![cfg(target_os = "macos")]

use std::time::Duration;

use uncontainerizable_core::{App, ContainOptions, DestroyOptions};

/// `sleep` responds to SIGTERM, so the quit ladder should drain the
/// container at `sigterm_tree` without escalating to `sigkill_tree`.
#[tokio::test]
async fn sleep_drains_at_sigterm_tree_without_escalation() {
    let app = App::new("test.sleep_drains_at_sigterm").unwrap();
    let opts = ContainOptions {
        args: vec!["30".into()],
        darwin_tag_argv0: false,
        ..Default::default()
    };
    let mut container = app.contain("sleep", opts).await.expect("spawn sleep");

    let result = container.destroy(DestroyOptions::default()).await;

    assert!(
        result.errors.is_empty(),
        "destroy surfaced errors: {:?}",
        result.errors
    );
    let stage = result.quit.exited_at_stage.as_deref().unwrap_or("<none>");
    assert!(
        !result.quit.reached_terminal_stage,
        "`sleep` should drain before SIGKILL; got {:?}",
        result.quit
    );
    assert!(
        stage == "sigterm_tree" || stage.starts_with("before:"),
        "unexpected exit stage: {stage}"
    );
}

/// Passing `--ignore-sigterm` to the test-child binary simulates a
/// misbehaving process. The quit ladder must escalate to SIGKILL.
#[tokio::test]
async fn ignored_sigterm_escalates_to_sigkill() {
    let app = App::new("test.ignored_sigterm_escalates").unwrap();
    let test_child = cargo_example_path("test-child");
    let opts = ContainOptions {
        args: vec!["--ignore-sigterm".into()],
        darwin_tag_argv0: false,
        ..Default::default()
    };
    let mut container = app
        .contain(test_child.to_str().unwrap(), opts)
        .await
        .expect("spawn test-child");

    let result = container.destroy(DestroyOptions::default()).await;

    assert!(
        result.errors.is_empty(),
        "destroy surfaced errors: {:?}",
        result.errors
    );
    assert!(
        result.quit.reached_terminal_stage,
        "SIGTERM-ignoring process should escalate to SIGKILL; got {:?}",
        result.quit
    );
    assert_eq!(result.quit.exited_at_stage.as_deref(), Some("sigkill_tree"));
}

/// Spawning a second container with the same identity must kill the first
/// via argv[0] tag scanning.
#[tokio::test]
async fn identity_based_preemption_kills_predecessor() {
    let app = App::new("test.identity_preemption").unwrap();

    let first_opts = ContainOptions {
        args: vec!["60".into()],
        identity: Some("browser".into()),
        darwin_tag_argv0: true,
        ..Default::default()
    };
    let first = app.contain("sleep", first_opts).await.expect("first spawn");
    let first_pid = first.pid();

    let second_opts = ContainOptions {
        args: vec!["60".into()],
        identity: Some("browser".into()),
        darwin_tag_argv0: true,
        ..Default::default()
    };
    let mut second = app
        .contain("sleep", second_opts)
        .await
        .expect("second spawn");
    let second_pid = second.pid();
    assert_ne!(first_pid, second_pid);

    // Give the kernel a moment to reap the SIGKILL delivered to the first
    // process tree. 300ms is generous: SIGKILL is synchronous.
    tokio::time::sleep(Duration::from_millis(300)).await;

    assert!(
        first.is_empty().await.unwrap(),
        "predecessor should be dead after second spawn"
    );

    let _ = second.destroy(DestroyOptions::default()).await;
}

/// When no identity is provided, argv[0] is not rewritten and no
/// predecessor killing occurs.
#[tokio::test]
async fn spawn_without_identity_does_not_touch_other_processes() {
    let app = App::new("test.no_identity").unwrap();

    let first = app
        .contain(
            "sleep",
            ContainOptions {
                args: vec!["60".into()],
                ..Default::default()
            },
        )
        .await
        .expect("first spawn");
    let first_pid = first.pid();

    let mut second = app
        .contain(
            "sleep",
            ContainOptions {
                args: vec!["60".into()],
                ..Default::default()
            },
        )
        .await
        .expect("second spawn");

    tokio::time::sleep(Duration::from_millis(200)).await;

    // first should still be alive: no identity means no preemption.
    assert!(
        pid_alive(first_pid),
        "first spawn should survive second spawn without identity"
    );

    let _ = second.destroy(DestroyOptions::default()).await;

    // Clean up `first` by destroying it directly.
    let mut first = first;
    let _ = first.destroy(DestroyOptions::default()).await;
}

#[tokio::test]
async fn invalid_prefix_rejected_at_app_construction() {
    assert!(App::new("").is_err());
    assert!(App::new("has space").is_err());
    assert!(App::new("has/slash").is_err());
}

fn pid_alive(pid: u32) -> bool {
    use nix::errno::Errno;
    use nix::sys::signal::kill;
    use nix::unistd::Pid;
    match kill(Pid::from_raw(pid as i32), None) {
        Ok(()) => true,
        Err(Errno::EPERM) => true,
        Err(_) => false,
    }
}

/// Resolve the path to an example binary. Cargo places examples at
/// `$CARGO_TARGET_DIR/<profile>/examples/<name>`; we fall back to a
/// relative path from `CARGO_MANIFEST_DIR` when `CARGO_TARGET_DIR` isn't
/// set (the default workspace layout).
fn cargo_example_path(name: &str) -> std::path::PathBuf {
    let profile = if cfg!(debug_assertions) {
        "debug"
    } else {
        "release"
    };
    let target_dir = std::env::var("CARGO_TARGET_DIR").unwrap_or_else(|_| {
        let manifest_dir = std::env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR");
        // Workspace convention: target/ sits at the workspace root.
        let mut path = std::path::PathBuf::from(manifest_dir);
        path.pop(); // crates/uncontainerizable-core -> crates
        path.pop(); // crates -> workspace root
        path.push("target");
        path.to_string_lossy().into_owned()
    });
    let mut p = std::path::PathBuf::from(target_dir);
    p.push(profile);
    p.push("examples");
    p.push(name);
    p
}

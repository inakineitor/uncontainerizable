//! Integration tests for the Linux platform.
//!
//! Spawns real processes under cgroup v2 and asserts the staged quit
//! ladder empties the cgroup. Gated to `target_os = "linux"` and to
//! `#[ignore]` because cgroup v2 delegation isn't set up on the default
//! GitHub Actions Linux runner. Run manually with:
//!
//! ```bash
//! systemd-run --user --scope --property=Delegate=yes \
//!   cargo test --test linux -- --ignored
//! ```

#![cfg(target_os = "linux")]

use std::time::Duration;

use uncontainerizable_core::{App, ContainOptions, DestroyOptions};

#[tokio::test]
#[ignore = "requires cgroup v2 delegation; see module docs"]
async fn sleep_drains_at_sigterm_frozen_without_escalation() {
    let app = App::new("test.linux.sleep_drains").unwrap();
    let opts = ContainOptions {
        args: vec!["30".into()],
        identity: Some("sleep-drains".into()),
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
        stage == "sigterm_frozen" || stage.starts_with("before:"),
        "unexpected exit stage: {stage}"
    );
}

#[tokio::test]
#[ignore = "requires cgroup v2 delegation; see module docs"]
async fn ignored_sigterm_escalates_to_sigkill_frozen() {
    let app = App::new("test.linux.ignored_sigterm").unwrap();
    let test_child = cargo_example_path("test-child");
    let opts = ContainOptions {
        args: vec!["--ignore-sigterm".into()],
        identity: Some("ignore-sigterm".into()),
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
    assert_eq!(
        result.quit.exited_at_stage.as_deref(),
        Some("sigkill_frozen")
    );
}

#[tokio::test]
#[ignore = "requires cgroup v2 delegation; see module docs"]
async fn identity_based_preemption_kills_predecessor() {
    let app = App::new("test.linux.identity_preemption").unwrap();

    let first = app
        .contain(
            "sleep",
            ContainOptions {
                args: vec!["60".into()],
                identity: Some("browser".into()),
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
                identity: Some("browser".into()),
                ..Default::default()
            },
        )
        .await
        .expect("second spawn");
    assert_ne!(first_pid, second.pid());

    tokio::time::sleep(Duration::from_millis(300)).await;

    assert!(
        first.is_empty().await.unwrap(),
        "predecessor should be dead after second spawn"
    );

    let _ = second.destroy(DestroyOptions::default()).await;
}

#[tokio::test]
async fn invalid_prefix_rejected_at_app_construction() {
    assert!(App::new("").is_err());
    assert!(App::new("has space").is_err());
    assert!(App::new("has/slash").is_err());
}

fn cargo_example_path(name: &str) -> std::path::PathBuf {
    let profile = if cfg!(debug_assertions) {
        "debug"
    } else {
        "release"
    };
    let target_dir = std::env::var("CARGO_TARGET_DIR").unwrap_or_else(|_| {
        let manifest_dir = std::env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR");
        let mut path = std::path::PathBuf::from(manifest_dir);
        path.pop();
        path.pop();
        path.push("target");
        path.to_string_lossy().into_owned()
    });
    let mut p = std::path::PathBuf::from(target_dir);
    p.push(profile);
    p.push("examples");
    p.push(name);
    p
}

//! Integration tests for the Windows platform.
//!
//! Spawns real processes into a Job Object and asserts the staged quit
//! ladder drains the job. `test-child.exe` is a console app with no GUI,
//! so the `wm_close_root` stage has nothing to target and the ladder must
//! reach the terminal `terminate_job` stage.

#![cfg(windows)]

use std::time::Duration;

use uncontainerizable_core::{App, ContainOptions, DestroyOptions};

#[tokio::test]
async fn test_child_terminates_at_terminate_job() {
    let app = App::new("test.win32.terminate_job").unwrap();
    let test_child = cargo_example_path("test-child.exe");
    let opts = ContainOptions {
        identity: Some("term-job".into()),
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
        "console test-child has no window; ladder should escalate to `terminate_job`; got {:?}",
        result.quit
    );
    assert_eq!(
        result.quit.exited_at_stage.as_deref(),
        Some("terminate_job")
    );
}

#[tokio::test]
async fn identity_based_preemption_kills_predecessor() {
    let app = App::new("test.win32.identity_preemption").unwrap();
    let test_child = cargo_example_path("test-child.exe");

    let first = app
        .contain(
            test_child.to_str().unwrap(),
            ContainOptions {
                identity: Some("browser".into()),
                ..Default::default()
            },
        )
        .await
        .expect("first spawn");
    let first_pid = first.pid();

    let mut second = app
        .contain(
            test_child.to_str().unwrap(),
            ContainOptions {
                identity: Some("browser".into()),
                ..Default::default()
            },
        )
        .await
        .expect("second spawn");
    assert_ne!(first_pid, second.pid());

    // Give Windows time to complete the TerminateJobObject + process reap.
    tokio::time::sleep(Duration::from_millis(300)).await;

    assert!(
        first.is_empty().await.unwrap(),
        "predecessor job should be empty after second spawn"
    );

    let _ = second.destroy(DestroyOptions::default()).await;
}

#[tokio::test]
async fn destroying_superseded_container_does_not_kill_successor() {
    let app = App::new("test.win32.superseded_destroy").unwrap();
    let test_child = cargo_example_path("test-child.exe");

    let mut first = app
        .contain(
            test_child.to_str().unwrap(),
            ContainOptions {
                identity: Some("browser".into()),
                ..Default::default()
            },
        )
        .await
        .expect("first spawn");
    let mut second = app
        .contain(
            test_child.to_str().unwrap(),
            ContainOptions {
                identity: Some("browser".into()),
                ..Default::default()
            },
        )
        .await
        .expect("second spawn");
    let second_pid = second.pid();

    tokio::time::sleep(Duration::from_millis(300)).await;

    let result = first.destroy(DestroyOptions::default()).await;
    assert!(
        result.errors.is_empty(),
        "destroying superseded container surfaced errors: {:?}",
        result.errors
    );
    tokio::time::sleep(Duration::from_millis(200)).await;

    assert!(
        pid_alive(second_pid),
        "destroying superseded container should not kill successor"
    );

    let _ = second.destroy(DestroyOptions::default()).await;
}

#[tokio::test]
async fn spawn_without_identity_does_not_touch_other_processes() {
    let app = App::new("test.win32.no_identity").unwrap();
    let test_child = cargo_example_path("test-child.exe");

    let first = app
        .contain(test_child.to_str().unwrap(), ContainOptions::default())
        .await
        .expect("first spawn");
    let first_pid = first.pid();

    let mut second = app
        .contain(test_child.to_str().unwrap(), ContainOptions::default())
        .await
        .expect("second spawn");

    tokio::time::sleep(Duration::from_millis(200)).await;

    assert!(
        pid_alive(first_pid),
        "first spawn should survive second spawn without identity"
    );

    let _ = second.destroy(DestroyOptions::default()).await;

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
    use windows::Win32::Foundation::CloseHandle;
    use windows::Win32::System::Threading::{OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION};
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

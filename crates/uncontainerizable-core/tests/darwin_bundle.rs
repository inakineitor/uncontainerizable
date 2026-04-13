//! Integration tests for the Darwin Launch Services spawn path.
//!
//! Builds a synthetic `.app` bundle from the `test-child` example binary
//! at test-run time (not via `build.rs`, so the fixture is scoped to
//! this test binary). All tests require `cargo build --example
//! test-child` to have run first, same convention as `tests/darwin.rs`.
//!
//! Gated to `target_os = "macos"`; the entire bundle route and
//! `platforms::darwin` module are cfg-gated, so these tests don't
//! compile on Linux / Windows.

#![cfg(target_os = "macos")]

use std::path::PathBuf;
use std::time::Duration;

use tokio::fs;

use uncontainerizable_core::{App, ContainOptions, DestroyOptions};

const FIXTURE_EXEC_NAME: &str = "TestChild";

struct FixtureBundle {
    bundle_id: String,
    path: PathBuf,
}

/// Assemble a fresh synthetic `.app` bundle for one test invocation.
/// Each fixture gets a unique path and bundle ID so parallel tests don't
/// race through shared Launch Services state.
async fn fixture_bundle(label: &str) -> Option<FixtureBundle> {
    let test_child = cargo_example_path("test-child");
    if !test_child.exists() {
        eprintln!(
            "skipping: {} not found; run `cargo build --example test-child`",
            test_child.display()
        );
        return None;
    }

    let suffix = fixture_suffix(label);
    let bundle_id = format!("com.uncontainerizable.tests.testchild.{suffix}");
    let target_dir = target_dir();
    let bundle_root = target_dir
        .join("tests-fixtures")
        .join(format!("TestChild-{suffix}.app"));
    let macos_dir = bundle_root.join("Contents").join("MacOS");
    let plist_path = bundle_root.join("Contents").join("Info.plist");
    let exec_path = macos_dir.join(FIXTURE_EXEC_NAME);

    fs::create_dir_all(&macos_dir)
        .await
        .expect("create bundle MacOS dir");
    fs::write(&plist_path, info_plist_xml(&bundle_id))
        .await
        .expect("write Info.plist");

    // Hardlink (or replace) the test-child binary into the bundle. LS
    // refuses bundles whose main exec is a symlink pointing outside the
    // bundle on recent macOS versions, so we hardlink or copy.
    let _ = fs::remove_file(&exec_path).await;
    if let Err(e) = fs::hard_link(&test_child, &exec_path).await {
        // Fall back to copy on cross-device link errors (EXDEV), which
        // happen when CARGO_TARGET_DIR lives on a different filesystem.
        if e.raw_os_error() == Some(libc_exdev()) {
            fs::copy(&test_child, &exec_path)
                .await
                .expect("copy test-child into bundle");
        } else {
            panic!("failed to link test-child into bundle: {e}");
        }
    }

    // chmod +x is only strictly necessary after copy, but always
    // setting it avoids platform-edge flake.
    use std::os::unix::fs::PermissionsExt;
    let mut perms = std::fs::metadata(&exec_path)
        .expect("exec metadata")
        .permissions();
    perms.set_mode(0o755);
    std::fs::set_permissions(&exec_path, perms).expect("chmod +x");

    Some(FixtureBundle {
        bundle_id,
        path: bundle_root,
    })
}

fn fixture_suffix(label: &str) -> String {
    let sanitized_label: String = label
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() {
                c.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect();
    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    format!("{sanitized_label}.{}.{}", std::process::id(), nonce)
}

fn info_plist_xml(bundle_id: &str) -> String {
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>CFBundleIdentifier</key>
    <string>{bundle_id}</string>
    <key>CFBundleExecutable</key>
    <string>{exec_name}</string>
    <key>CFBundleName</key>
    <string>TestChild</string>
    <key>CFBundlePackageType</key>
    <string>APPL</string>
    <key>CFBundleVersion</key>
    <string>1.0</string>
    <key>LSUIElement</key>
    <true/>
</dict>
</plist>
"#,
        bundle_id = bundle_id,
        exec_name = FIXTURE_EXEC_NAME
    )
}

fn cargo_example_path(name: &str) -> PathBuf {
    let mut p = target_dir();
    p.push(if cfg!(debug_assertions) {
        "debug"
    } else {
        "release"
    });
    p.push("examples");
    p.push(name);
    p
}

fn target_dir() -> PathBuf {
    let dir = std::env::var("CARGO_TARGET_DIR").unwrap_or_else(|_| {
        let manifest_dir = std::env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR");
        let mut path = PathBuf::from(manifest_dir);
        path.pop(); // crates/uncontainerizable-core -> crates
        path.pop(); // crates -> workspace root
        path.push("target");
        path.to_string_lossy().into_owned()
    });
    PathBuf::from(dir)
}

fn libc_exdev() -> i32 {
    18 // EXDEV: cross-device link on Linux/macOS.
}

#[tokio::test]
async fn bundle_launch_populates_bundle_id_from_info_plist() {
    let Some(bundle) = fixture_bundle("bundle-id-populated").await else {
        return;
    };

    let app = App::new("test.ls.bundle_id_populated").unwrap();
    let mut container = app
        .contain(bundle.path.to_str().unwrap(), ContainOptions::default())
        .await
        .expect("spawn bundle");

    assert_eq!(
        container.probe().bundle_id.as_deref(),
        Some(bundle.bundle_id.as_str()),
        "bundle id should come from Info.plist, not lsappinfo race"
    );

    let _ = container.destroy(DestroyOptions::default()).await;
}

#[tokio::test]
async fn bundle_launch_resolves_pid_within_timeout() {
    let Some(bundle) = fixture_bundle("pid-resolves").await else {
        return;
    };

    let app = App::new("test.ls.pid_resolves").unwrap();
    let mut container = app
        .contain(bundle.path.to_str().unwrap(), ContainOptions::default())
        .await
        .expect("spawn bundle");

    assert!(container.pid() > 1, "resolved PID must be real");

    let _ = container.destroy(DestroyOptions::default()).await;
}

#[tokio::test]
async fn bundle_launch_destroys_cleanly() {
    let Some(bundle) = fixture_bundle("destroy-clean").await else {
        return;
    };

    let app = App::new("test.ls.destroy_clean").unwrap();
    let mut container = app
        .contain(bundle.path.to_str().unwrap(), ContainOptions::default())
        .await
        .expect("spawn bundle");

    let result = container.destroy(DestroyOptions::default()).await;
    assert!(
        result.errors.is_empty(),
        "destroy surfaced errors: {:?}",
        result.errors
    );
    // test-child is a console binary, so Apple Events typically fail
    // to reach it (no event loop listening for bundle events). The
    // ladder escalates to SIGTERM tree, which test-child handles.
    let stage = result.quit.exited_at_stage.as_deref().unwrap_or("<none>");
    assert!(
        stage == "sigterm_tree" || stage == "apple_event_quit" || stage.starts_with("before:"),
        "unexpected exit stage: {stage}"
    );
}

#[tokio::test]
async fn bundle_identity_preemption_via_pidfile() {
    let Some(bundle) = fixture_bundle("pid-preempt").await else {
        return;
    };

    let app = App::new("test.ls.pid_preempt").unwrap();

    let first = app
        .contain(
            bundle.path.to_str().unwrap(),
            ContainOptions {
                identity: Some("run".into()),
                ..Default::default()
            },
        )
        .await
        .expect("first spawn");
    let first_pid = first.pid();

    let mut second = app
        .contain(
            bundle.path.to_str().unwrap(),
            ContainOptions {
                identity: Some("run".into()),
                ..Default::default()
            },
        )
        .await
        .expect("second spawn");
    assert_ne!(first_pid, second.pid());

    tokio::time::sleep(Duration::from_millis(500)).await;

    assert!(
        !pid_alive(first_pid),
        "predecessor PID {first_pid} should be dead after pidfile preemption"
    );

    let _ = second.destroy(DestroyOptions::default()).await;
}

#[tokio::test]
async fn bundle_args_are_passed_via_dash_dash_args() {
    let Some(bundle) = fixture_bundle("args-passthrough").await else {
        return;
    };

    // Unique dump path so concurrent test runs don't collide.
    let dump = std::env::temp_dir().join(format!(
        "uncont-test-argv-{}-{}.txt",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0)
    ));
    let _ = std::fs::remove_file(&dump);

    let app = App::new("test.ls.args_passthrough").unwrap();
    let opts = ContainOptions {
        args: vec!["--flag".into(), "value".into()],
        env: vec![(
            "UNCONTAINERIZABLE_ARGV_DUMP".into(),
            dump.to_string_lossy().into_owned(),
        )],
        ..Default::default()
    };
    let mut container = app
        .contain(bundle.path.to_str().unwrap(), opts)
        .await
        .expect("spawn bundle");

    // Poll for the dump file; LS-launched apps start asynchronously.
    let deadline = tokio::time::Instant::now() + Duration::from_secs(3);
    let contents = loop {
        if let Ok(s) = fs::read_to_string(&dump).await {
            break s;
        }
        if tokio::time::Instant::now() >= deadline {
            let _ = container.destroy(DestroyOptions::default()).await;
            panic!("test-child never wrote argv dump at {}", dump.display());
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    };

    let _ = container.destroy(DestroyOptions::default()).await;
    let _ = std::fs::remove_file(&dump);

    assert!(
        contents.contains("--flag"),
        "dumped argv should contain --flag; got:\n{contents}"
    );
    assert!(
        contents.contains("value"),
        "dumped argv should contain value; got:\n{contents}"
    );
}

#[tokio::test]
async fn non_bundle_path_uses_direct_exec() {
    // `sleep` doesn't end in .app, so this must take the direct-exec
    // path. The test doesn't verify much beyond "it still works" — the
    // regression risk is that `is_app_bundle` misclassifies something.
    let app = App::new("test.ls.direct_unchanged").unwrap();
    let opts = ContainOptions {
        args: vec!["30".into()],
        darwin_tag_argv0: false,
        ..Default::default()
    };
    let mut container = app.contain("sleep", opts).await.expect("spawn sleep");
    let result = container.destroy(DestroyOptions::default()).await;
    assert!(
        result.errors.is_empty(),
        "direct exec broke: {:?}",
        result.errors
    );
}

#[tokio::test]
async fn bundle_path_that_is_a_file_is_not_detected_as_bundle() {
    // A file named something.app falls through to direct-exec because
    // metadata().is_dir() is false. Since the file isn't executable
    // in this test, the resulting error is a direct-exec Spawn failure,
    // not a BundleError.
    let path = std::env::temp_dir().join(format!(
        "uncont-fake-{}-{}.app",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0)
    ));
    std::fs::write(&path, b"not a bundle").expect("write file");

    let app = App::new("test.ls.fake_app_file").unwrap();
    let result = app
        .contain(path.to_str().unwrap(), ContainOptions::default())
        .await;

    let _ = std::fs::remove_file(&path);

    match result {
        Ok(_) => panic!("expected failure for non-bundle file path"),
        Err(e) => {
            // Direct-exec error, not BundleError: matches the
            // documented "no walk-up inference" contract.
            let msg = e.to_string();
            assert!(
                msg.contains("spawn") || msg.contains("Spawn"),
                "expected direct-exec Spawn error, got {msg}"
            );
        }
    }
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

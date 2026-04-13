//! Launch Services spawn support for `.app` bundles.
//!
//! When `command` ends in `.app` and resolves to a directory, the Darwin
//! `spawn` routes here instead of `posix_spawn`. Launching through
//! `open -n -F -a <bundle>` gets the app properly registered with
//! `launchservicesd`: Dock icon, bundle-ID-addressable Apple Events, no
//! "Reopen windows?" prompt on next launch (the `-F` flag discards
//! saved state, the `-n` flag forces a new instance past any stale PSN
//! LS kept from a killed predecessor).
//!
//! argv[0] tagging doesn't work on this path because LS rewrites argv.
//! Identity preemption uses a PID file per combined-identity under
//! `~/Library/Caches/uncontainerizable/`. The file stores the last
//! spawned PID; on subsequent spawns with the same identity we read
//! the file, verify the PID is still alive *and* its executable still
//! matches this bundle (to avoid killing an unrelated process that
//! inherited the PID), then SIGKILL its tree. This mechanism survives
//! supervisor crashes because the file outlives us.
//!
//! An earlier design used `UNCONTAINERIZABLE_IDENTITY=<combined>` env
//! variable tagging parsed via `ps -E`. On macOS the `-E` flag does
//! not surface the environment to non-root processes despite the man
//! page, so that mechanism was replaced.
//!
//! PID resolution after `open` returns requires a poll: `open` exits
//! well before LS has spawned the app, and the actual app process is a
//! child of `launchservicesd`, not us. We snapshot the set of running
//! PIDs whose executable matches the bundle's main exec before launch,
//! then poll `ps` after launch for a new PID matching the same exec.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use kill_tree::Config;
use kill_tree::tokio::kill_tree_with_config;
use tokio::fs;
use tokio::process::Command;
use tokio::time::sleep;

use crate::error::BundleError;

const PLIST_BUDDY: &str = "/usr/libexec/PlistBuddy";
const PID_POLL_INTERVAL: Duration = Duration::from_millis(100);

/// Directory under the user's cache tree that holds per-identity PID
/// files for bundle launches. Created on demand.
const PIDFILE_DIR: &str = "Library/Caches/uncontainerizable";

/// Plist fields we care about. Narrow on purpose: other keys aren't
/// needed for launching.
pub struct BundleInfo {
    /// `CFBundleIdentifier` from the app's `Info.plist`. Used for probe
    /// population and downstream Apple Events addressing.
    pub bundle_id: String,
    /// Absolute path to `<bundle>/Contents/MacOS/<CFBundleExecutable>`.
    /// Used for PID resolution (matching against `ps comm=`) and as the
    /// authoritative `Probe::executable_path` on this route.
    pub executable_path: PathBuf,
}

/// `true` if `command` points at a `.app` directory (symlinks resolved).
///
/// Deliberately narrow: we do not walk up parent directories to find an
/// enclosing `.app`. Callers who pass `/Applications/Foo.app/Contents/MacOS/Foo`
/// get direct-exec, by design. That's the escape hatch for launching a
/// specific helper binary without LS involvement.
pub fn is_app_bundle(command: &str) -> bool {
    let path = Path::new(command);
    if path.extension().and_then(|s| s.to_str()) != Some("app") {
        return false;
    }
    // `metadata()` (not `symlink_metadata`) resolves symlinks; a symlink
    // to an .app directory still counts. Non-existent paths and regular
    // files both fall through to false.
    std::fs::metadata(path).map(|m| m.is_dir()).unwrap_or(false)
}

/// Read `CFBundleIdentifier` and `CFBundleExecutable` from the bundle's
/// `Info.plist` via `PlistBuddy`. Shell-out rather than a dep so we
/// handle XML and binary plists alike with no extra crates.
pub async fn read_info(bundle_path: &Path) -> Result<BundleInfo, BundleError> {
    if !bundle_path.is_dir() {
        return Err(BundleError::NotADirectory {
            path: bundle_path.to_path_buf(),
        });
    }
    let plist = bundle_path.join("Contents").join("Info.plist");
    if fs::metadata(&plist).await.is_err() {
        return Err(BundleError::PlistMissing { path: plist });
    }
    let bundle_id = read_plist_field(&plist, "CFBundleIdentifier").await?;
    let exec_name = read_plist_field(&plist, "CFBundleExecutable").await?;
    let executable_path = bundle_path.join("Contents").join("MacOS").join(exec_name);
    Ok(BundleInfo {
        bundle_id,
        executable_path,
    })
}

async fn read_plist_field(plist: &Path, field: &'static str) -> Result<String, BundleError> {
    let output = Command::new(PLIST_BUDDY)
        .args([
            "-c",
            &format!("Print :{field}"),
            &plist.display().to_string(),
        ])
        .output()
        .await
        .map_err(|source| BundleError::PlistField {
            field,
            plist: plist.to_path_buf(),
            source,
        })?;
    if !output.status.success() {
        return Err(BundleError::PlistField {
            field,
            plist: plist.to_path_buf(),
            source: std::io::Error::other(format!("PlistBuddy exited with {}", output.status)),
        });
    }
    let value = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if value.is_empty() {
        return Err(BundleError::PlistField {
            field,
            plist: plist.to_path_buf(),
            source: std::io::Error::other("empty value"),
        });
    }
    Ok(value)
}

/// Resolve the per-identity PID file path.
/// Returns `None` if `$HOME` isn't set (shouldn't happen on macOS but
/// we handle it rather than panic).
pub fn identity_pidfile(combined_identity: &str) -> Option<PathBuf> {
    let home = std::env::var_os("HOME")?;
    let sanitized = sanitize_identity(combined_identity);
    Some(
        PathBuf::from(home)
            .join(PIDFILE_DIR)
            .join(format!("{sanitized}.pid")),
    )
}

/// Replace filesystem-hostile characters in a combined identity (which
/// is `<prefix>:<identity>` using dots, colons, slashes) with `.`. Keeps
/// the identity human-readable while guaranteeing a flat filename.
fn sanitize_identity(combined: &str) -> String {
    combined
        .chars()
        .map(|c| match c {
            '/' | ':' | '\\' | '\0' => '.',
            c => c,
        })
        .collect()
}

/// Kill any running predecessor registered for this identity. Reads the
/// PID file, verifies the PID is still alive AND its executable still
/// matches the bundle (guards against PID reuse after a crash), then
/// SIGKILLs the whole tree. Best-effort: file missing, stale PID, or
/// mismatched executable all produce a silent no-op.
///
/// The file is not deleted here — `spawn_bundle` overwrites it with
/// the new PID after a successful launch, so the write is what
/// "commits" the handoff.
pub async fn kill_existing_by_pidfile(
    combined_identity: &str,
    executable_path: &Path,
) -> std::io::Result<()> {
    let Some(pidfile) = identity_pidfile(combined_identity) else {
        return Ok(());
    };
    let raw = match fs::read_to_string(&pidfile).await {
        Ok(s) => s,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(e) => return Err(e),
    };
    let Ok(pid) = raw.trim().parse::<u32>() else {
        return Ok(());
    };
    if !pid_exec_matches(pid, executable_path).await {
        // Stale PID file: the old process is gone, or the PID was
        // reused by something unrelated. Leave alone and let the fresh
        // spawn overwrite the file.
        return Ok(());
    }
    let _ = kill_tree_with_config(
        pid,
        &Config {
            signal: "SIGKILL".into(),
            include_target: true,
        },
    )
    .await;
    Ok(())
}

/// `true` iff `pid` is alive and its executable (via `ps comm=`) is an
/// exact match for `expected`. Used to reject stale PID files before
/// signalling so we don't kill an unrelated process that inherited a
/// reused PID.
async fn pid_exec_matches(pid: u32, expected: &Path) -> bool {
    let Ok(output) = Command::new("ps")
        .args(["-p", &pid.to_string(), "-o", "comm="])
        .output()
        .await
    else {
        return false;
    };
    if !output.status.success() {
        return false;
    }
    let target = expected.to_string_lossy();
    let stdout = String::from_utf8_lossy(&output.stdout);
    stdout
        .lines()
        .next()
        .map(|l| l.trim() == target)
        .unwrap_or(false)
}

/// Atomically record the PID for `combined_identity`. Writes to a
/// tempfile under the same directory and renames over the final path
/// so readers never see a half-written file.
pub async fn write_pidfile(combined_identity: &str, pid: u32) -> std::io::Result<()> {
    let Some(pidfile) = identity_pidfile(combined_identity) else {
        return Ok(());
    };
    if let Some(parent) = pidfile.parent() {
        fs::create_dir_all(parent).await?;
    }
    let tmp = pidfile.with_extension("pid.tmp");
    fs::write(&tmp, pid.to_string().as_bytes()).await?;
    fs::rename(&tmp, &pidfile).await
}

/// Collect the current set of PIDs whose `ps comm=` matches
/// `exec_path`. Used as the "before" snapshot for PID resolution: the
/// new PID must be one that wasn't here pre-launch.
pub async fn snapshot_bundle_pids(exec_path: &Path) -> HashSet<u32> {
    scan_matching_pids(exec_path).await.unwrap_or_default()
}

async fn scan_matching_pids(exec_path: &Path) -> std::io::Result<HashSet<u32>> {
    let output = Command::new("ps")
        .args(["-ax", "-o", "pid=,comm="])
        .output()
        .await?;
    if !output.status.success() {
        return Ok(HashSet::new());
    }
    let target = exec_path.to_string_lossy();
    let stdout = String::from_utf8_lossy(&output.stdout);
    let mut pids = HashSet::new();
    for line in stdout.lines() {
        let Some((pid_str, comm)) = line.trim_start().split_once(char::is_whitespace) else {
            continue;
        };
        if comm.trim() != target {
            continue;
        }
        if let Ok(pid) = pid_str.trim().parse::<u32>() {
            pids.insert(pid);
        }
    }
    Ok(pids)
}

/// Poll `ps` for a PID matching `exec_path` that wasn't in `baseline`.
/// Returns `PidResolveTimeout` once `deadline` passes.
///
/// 100ms poll interval — fast enough that LS registration delay (~50-
/// 500ms in practice) resolves within two or three polls on warm
/// launches, slow enough to avoid flooding `ps`.
pub async fn resolve_new_pid(
    exec_path: &Path,
    baseline: &HashSet<u32>,
    deadline: Instant,
    bundle_id: &str,
) -> Result<u32, BundleError> {
    let start = Instant::now();
    loop {
        let current = snapshot_bundle_pids(exec_path).await;
        if let Some(&pid) = current.iter().find(|p| !baseline.contains(p)) {
            return Ok(pid);
        }
        if Instant::now() >= deadline {
            return Err(BundleError::PidResolveTimeout {
                bundle_id: bundle_id.to_string(),
                executable_path: exec_path.to_path_buf(),
                waited_ms: start.elapsed().as_millis() as u64,
            });
        }
        sleep(PID_POLL_INTERVAL).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn is_app_bundle_rejects_non_app_extensions() {
        assert!(!is_app_bundle("/usr/bin/sleep"));
        assert!(!is_app_bundle("/Applications/Foo"));
        assert!(!is_app_bundle("/tmp/Foo.dmg"));
    }

    #[test]
    fn is_app_bundle_rejects_missing_paths() {
        assert!(!is_app_bundle("/does/not/exist/TotallyMissing.app"));
    }

    #[test]
    fn sanitize_identity_replaces_path_separators_and_colons() {
        assert_eq!(
            sanitize_identity("com.example.app:browser-main"),
            "com.example.app.browser-main"
        );
        assert_eq!(sanitize_identity("a/b:c\\d"), "a.b.c.d");
    }

    #[test]
    fn sanitize_identity_preserves_safe_characters() {
        assert_eq!(sanitize_identity("com.example.app"), "com.example.app");
        assert_eq!(sanitize_identity("abc-def_123"), "abc-def_123");
    }

    #[test]
    fn identity_pidfile_lands_under_home() {
        let Some(path) = identity_pidfile("com.example:run") else {
            // HOME not set in CI container; skip.
            return;
        };
        let s = path.to_string_lossy();
        assert!(s.contains("Library/Caches/uncontainerizable"));
        assert!(s.ends_with("com.example.run.pid"));
    }
}

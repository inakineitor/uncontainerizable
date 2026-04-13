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
//! argv[0] tagging doesn't work on this path because LS rewrites argv,
//! and macOS `ps -E` hides the environment from non-root callers, so
//! neither of the per-launch tagging schemes we use on other platforms
//! can round-trip through LS. The only signal for "is a matching
//! instance running right now" that survives an external launch is
//! `ps comm=` against the bundle's main-exec path.
//!
//! This makes identity preemption on the LS route a **singleton
//! switch**: when the caller passes an identity on a bundle launch we
//! scan `ps` for every process whose executable is
//! `<bundle>/Contents/MacOS/<CFBundleExecutable>` and SIGKILL each
//! tree — regardless of which identity (if any) started them. Two
//! concurrent LS launches of the same `.app` with different identities
//! cannot coexist; the second will terminate the first. Callers that
//! need multiple concurrent instances of a bundle should pass the
//! inner executable path to route through direct-exec, which does
//! keep per-identity tagging via argv[0].
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
/// Grace period for LS/kernel to fully reap a SIGKILL'd app before we
/// launch the replacement. Without it the post-launch PID poll can see
/// the old PID briefly and misinterpret it as the new instance.
const REAP_SETTLE: Duration = Duration::from_millis(300);

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

/// Kill every running instance of the bundle's main executable, not
/// just ones this supervisor launched. Returns the set of PIDs we
/// signalled, so callers can skip them when resolving the new PID and
/// wait for reap before the post-launch poll.
///
/// Best-effort: `ps` or `kill_tree` failures are swallowed because
/// preemption is not allowed to block a fresh spawn. A surviving
/// predecessor at worst means two instances coexist for a moment,
/// which `open -n` handles via its forced-new-instance semantics.
pub async fn kill_existing_bundle_instances(executable_path: &Path) -> HashSet<u32> {
    let pids = snapshot_bundle_pids(executable_path).await;
    if pids.is_empty() {
        return pids;
    }
    for &pid in &pids {
        let _ = kill_tree_with_config(
            pid,
            &Config {
                signal: "SIGKILL".into(),
                include_target: true,
            },
        )
        .await;
    }
    // Give the kernel a moment to reap the signalled processes. Without
    // this, `resolve_new_pid` may briefly see the SIGKILL'd PID (still
    // in the process table mid-reap) and either mistake it for the new
    // instance or burn poll iterations waiting for it to disappear.
    sleep(REAP_SETTLE).await;
    pids
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
}

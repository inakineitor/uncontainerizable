//! Bundle-ID lookup via the `lsappinfo` command-line tool.
//!
//! `lsappinfo info -only bundleID <pid>` emits a single line like
//! `"LSBundleID"="com.apple.Safari"`. We parse the quoted value and
//! return `Some(bundle_id)` when present; any failure (pid not
//! recognized as an app, tool missing, parse error) returns `None`.
//! Bundle IDs are optional on `Probe`, so callers degrade gracefully.

use tokio::process::Command;

pub async fn bundle_id(pid: u32) -> Option<String> {
    let output = Command::new("lsappinfo")
        .args(["info", "-only", "bundleID", &pid.to_string()])
        .output()
        .await
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    let line = stdout.lines().next()?.trim();
    // Expected shape: `"LSBundleID"="com.apple.Safari"`. Also tolerates
    // tools that emit just `com.apple.Safari` or lines without quotes.
    let value = line.split_once('=').map_or(line, |(_, rhs)| rhs).trim();
    let bundle = value.trim_matches('"').trim();
    // Filter out the sentinels `lsappinfo` emits when it has no bundle
    // association for a pid: `(null)`, `[ NULL ]`, bare brackets, or an
    // empty string. Anything bracketed is a human-readable placeholder
    // rather than a real bundle id, which would be reverse-DNS.
    if bundle.is_empty() || bundle.starts_with('[') || bundle == "(null)" {
        None
    } else {
        Some(bundle.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn returns_none_for_pid_with_no_app_association() {
        // PID 1 (launchd) isn't an app registered with LaunchServices,
        // so `lsappinfo` emits `"LSBundleID"="(null)"`. Expect None.
        let result = bundle_id(1).await;
        assert_eq!(result, None);
    }
}

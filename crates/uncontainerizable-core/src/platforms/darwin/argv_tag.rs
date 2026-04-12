//! argv[0] tagging for identity-based preemption on macOS.
//!
//! macOS has no kernel primitive analogous to cgroup v2 or Windows Job
//! Objects. We approximate it by rewriting argv[0] at spawn time to a tag
//! like `uncontainerizable:<identity>/<executable-name>`, then finding and
//! killing any running processes whose argv[0] begins with that tag before
//! the new one starts.
//!
//! Best-effort: some programs inspect argv[0] and misbehave. Callers can
//! opt out via `ContainOptions::darwin_tag_argv0 = false`, at the cost of
//! losing predecessor killing for that spawn.

use std::io;
use std::path::Path;

use kill_tree::Config;
use kill_tree::tokio::kill_tree_with_config;
use tokio::process::Command;

pub const TAG_PREFIX: &str = "uncontainerizable";

/// Build the argv[0] tag for an identity and command. The base filename is
/// preserved so ps / Activity Monitor still show something recognizable.
pub fn format(identity: &str, command: &str) -> String {
    let base = Path::new(command)
        .file_name()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| command.to_string());
    format!("{TAG_PREFIX}:{identity}/{base}")
}

/// Scan `ps -A -o pid,command` for processes whose argv[0] starts with the
/// tag prefix for this identity, and SIGKILL their trees.
///
/// Failure to invoke `ps` or parse its output is swallowed: predecessor
/// killing is best-effort and shouldn't block a fresh spawn.
pub async fn kill_existing(identity: &str) -> io::Result<()> {
    let needle = format!("{TAG_PREFIX}:{identity}/");
    let output = Command::new("ps")
        .args(["-A", "-o", "pid=,command="])
        .output()
        .await?;
    if !output.status.success() {
        return Ok(());
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    for line in stdout.lines() {
        let line = line.trim_start();
        let Some((pid_str, rest)) = line.split_once(char::is_whitespace) else {
            continue;
        };
        let rest = rest.trim_start();
        if !rest.starts_with(&needle) {
            continue;
        }
        let Ok(pid) = pid_str.trim().parse::<u32>() else {
            continue;
        };
        let _ = kill_tree_with_config(
            pid,
            &Config {
                signal: "SIGKILL".into(),
                include_target: true,
            },
        )
        .await;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn format_preserves_filename() {
        let tag = format("com.example.app:run", "/usr/bin/sleep");
        assert_eq!(tag, "uncontainerizable:com.example.app:run/sleep");
    }

    #[test]
    fn format_uses_command_verbatim_when_not_a_path() {
        let tag = format("ident", "mybin");
        assert_eq!(tag, "uncontainerizable:ident/mybin");
    }
}

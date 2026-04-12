//! Darwin quit-ladder stages.
//!
//! Three stages in order:
//! 1. `apple_event_quit`: sends an Apple Event "quit" to the root app via
//!    `osascript`. Only fires when a bundle ID is available. AppKit handles
//!    helper fanout: closing the root app asks children to shut down
//!    cleanly.
//! 2. `sigterm_tree`: SIGTERM delivered to the whole process tree via
//!    `kill_tree`.
//! 3. `sigkill_tree`: SIGKILL the whole tree. Terminal.

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use kill_tree::Config;
use kill_tree::tokio::kill_tree_with_config;
use tokio::process::Command;

use crate::container::{Container, Stage};
use crate::error::StageError;

pub fn darwin_stages() -> Vec<Arc<dyn Stage>> {
    vec![
        Arc::new(AppleEventQuitStage),
        Arc::new(SigTermTreeStage),
        Arc::new(SigKillTreeStage),
    ]
}

pub struct AppleEventQuitStage;

#[async_trait]
impl Stage for AppleEventQuitStage {
    fn name(&self) -> &str {
        "apple_event_quit"
    }
    fn is_terminal(&self) -> bool {
        false
    }
    fn max_wait(&self) -> Duration {
        Duration::from_secs(3)
    }
    async fn execute(&self, c: &dyn Container) -> Result<(), StageError> {
        let Some(bundle_id) = c.probe().bundle_id.as_ref() else {
            // No bundle ID available; Apple Events need one. Silently
            // no-op and let later stages handle teardown.
            return Ok(());
        };
        let script = format!(r#"tell application id "{bundle_id}" to quit"#);
        // osascript exit codes are inconsistent (a missing app, a
        // user-declined quit dialog, etc. all fail differently). We don't
        // propagate the failure: later stages will handle unresponsive
        // targets via SIGTERM / SIGKILL.
        let _ = Command::new("osascript")
            .args(["-e", &script])
            .output()
            .await?;
        Ok(())
    }
}

pub struct SigTermTreeStage;

#[async_trait]
impl Stage for SigTermTreeStage {
    fn name(&self) -> &str {
        "sigterm_tree"
    }
    fn is_terminal(&self) -> bool {
        false
    }
    fn max_wait(&self) -> Duration {
        Duration::from_secs(2)
    }
    async fn execute(&self, c: &dyn Container) -> Result<(), StageError> {
        let _ = kill_tree_with_config(
            c.pid(),
            &Config {
                signal: "SIGTERM".into(),
                include_target: true,
            },
        )
        .await?;
        Ok(())
    }
}

pub struct SigKillTreeStage;

#[async_trait]
impl Stage for SigKillTreeStage {
    fn name(&self) -> &str {
        "sigkill_tree"
    }
    fn is_terminal(&self) -> bool {
        true
    }
    fn max_wait(&self) -> Duration {
        Duration::from_millis(500)
    }
    async fn execute(&self, c: &dyn Container) -> Result<(), StageError> {
        let _ = kill_tree_with_config(
            c.pid(),
            &Config {
                signal: "SIGKILL".into(),
                include_target: true,
            },
        )
        .await?;
        Ok(())
    }
}

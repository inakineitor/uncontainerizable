//! Synthetic test child for uncontainerizable integration tests.
//!
//! Without arguments: installs handlers for SIGTERM/SIGINT/SIGHUP, prints
//! `got sig <name>` then exits 0; otherwise sleeps 60 seconds and exits.
//!
//! With `--ignore-sigterm`: also installs a SIGTERM handler but ignores it,
//! forcing callers to escalate to SIGKILL to shut the process down.
//!
//! Built via `cargo build --example test-child`.

use std::env;
use std::time::Duration;

use tokio::signal::unix::{SignalKind, signal};

#[tokio::main]
async fn main() {
    let ignore_sigterm = env::args().any(|a| a == "--ignore-sigterm");
    println!("test-child pid={} ready", std::process::id());

    let mut sigint = signal(SignalKind::interrupt()).expect("install SIGINT");
    let mut sighup = signal(SignalKind::hangup()).expect("install SIGHUP");

    if ignore_sigterm {
        // Still install the handler so the default kernel action doesn't
        // terminate us; we just don't react to the signal.
        let mut sigterm = signal(SignalKind::terminate()).expect("install SIGTERM");
        tokio::spawn(async move {
            loop {
                let _ = sigterm.recv().await;
                println!("got sig SIGTERM (ignored)");
            }
        });
    }

    let mut sigterm_react = if ignore_sigterm {
        None
    } else {
        Some(signal(SignalKind::terminate()).expect("install SIGTERM"))
    };

    tokio::select! {
        _ = async {
            if let Some(s) = sigterm_react.as_mut() {
                s.recv().await
            } else {
                std::future::pending().await
            }
        } => {
            println!("got sig SIGTERM");
        }
        _ = sigint.recv() => {
            println!("got sig SIGINT");
        }
        _ = sighup.recv() => {
            println!("got sig SIGHUP");
        }
        _ = tokio::time::sleep(Duration::from_secs(60)) => {
            println!("timeout, exiting");
        }
    }
}

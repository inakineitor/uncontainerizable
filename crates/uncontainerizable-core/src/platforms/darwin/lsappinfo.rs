//! Bundle-ID lookup via `lsappinfo`.
//!
//! Currently a stub that always returns `None`. Real implementation shells
//! out to `lsappinfo info -only bundleID <pid>` and parses the output;
//! deferred until the first consumer actually needs bundle-aware adapter
//! matching (the AppKit saved-state adapter).
//!
//! `Probe::bundle_id` is `Option<String>` so `None` is a valid runtime
//! outcome: adapters that need a bundle ID just won't match.

pub async fn bundle_id(_pid: u32) -> Option<String> {
    None
}

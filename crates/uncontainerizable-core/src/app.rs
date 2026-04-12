//! `App`: namespaced handle for spawning contained processes.
//!
//! The prefix namespaces identities so two unrelated libraries using
//! uncontainerizable can't collide. Convention is reverse-DNS.

use std::path::PathBuf;
use std::sync::Arc;

use crate::adapter::Adapter;
use crate::container::Container;
use crate::error::Result;
use crate::identity;

#[derive(Clone, Debug)]
pub struct App {
    prefix: String,
}

impl App {
    pub fn new(prefix: impl Into<String>) -> Result<Self> {
        let prefix = prefix.into();
        identity::validate(&prefix)?;
        Ok(Self { prefix })
    }

    pub fn prefix(&self) -> &str {
        &self.prefix
    }

    /// Spawn a contained process. If `opts.identity` is set, any previous
    /// instance with the same (prefix, identity) pair is killed before this
    /// one launches.
    pub async fn contain(&self, command: &str, opts: ContainOptions) -> Result<Box<dyn Container>> {
        crate::platforms::spawn(self, command, opts).await
    }
}

#[derive(Default)]
pub struct ContainOptions {
    pub args: Vec<String>,
    pub env: Vec<(String, String)>,
    pub cwd: Option<PathBuf>,
    pub adapters: Vec<Arc<dyn Adapter>>,
    /// Enable identity-based singleton enforcement. `None` = no preemption.
    pub identity: Option<String>,
    /// macOS only: if `false`, skip rewriting argv[0] with the identity tag
    /// even when `identity` is set. Loses predecessor killing on macOS for
    /// programs that inspect argv[0] and misbehave.
    pub darwin_tag_argv0: bool,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::Error;

    #[test]
    fn new_accepts_valid_prefix() {
        let app = App::new("com.example.my-supervisor").unwrap();
        assert_eq!(app.prefix(), "com.example.my-supervisor");
    }

    #[test]
    fn new_rejects_empty_prefix() {
        let err = App::new("").unwrap_err();
        assert!(matches!(err, Error::InvalidIdentity(_)));
    }

    #[test]
    fn new_rejects_whitespace_prefix() {
        let err = App::new("bad prefix").unwrap_err();
        assert!(matches!(err, Error::InvalidIdentity(_)));
    }

    #[test]
    fn new_rejects_slash_prefix() {
        let err = App::new("com/example").unwrap_err();
        assert!(matches!(err, Error::InvalidIdentity(_)));
    }

    #[test]
    fn contain_options_default_has_no_identity() {
        let opts = ContainOptions::default();
        assert!(opts.identity.is_none());
        assert!(opts.adapters.is_empty());
        assert!(opts.args.is_empty());
        assert!(opts.env.is_empty());
        assert!(opts.cwd.is_none());
    }
}

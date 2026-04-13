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

    /// Spawn a contained process. If `opts.identity` is set, a prior
    /// matching instance is killed before launch. On macOS Launch Services
    /// `.app` launches, matching is bundle-scoped rather than
    /// `(prefix, identity)`-scoped, so this route cannot keep two
    /// instances of the same app alive concurrently. If the app itself
    /// supports multiple concurrent instances, pass the inner executable
    /// path (`Foo.app/Contents/MacOS/Foo`) to use the direct-exec route.
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
    /// Enable singleton-style preemption. `None` = no preemption.
    /// On macOS Launch Services `.app` launches, this becomes
    /// bundle-scoped rather than identity-scoped.
    pub identity: Option<String>,
    /// macOS direct-exec only: if `false`, skip rewriting argv[0] with the
    /// identity tag. This disables identity-scoped predecessor killing on
    /// the direct-exec route for programs that inspect argv[0] and
    /// misbehave. Ignored for Launch Services `.app` launches.
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

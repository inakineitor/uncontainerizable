//! `NodeApp`: napi-exposed wrapper around `uncontainerizable_core::App`.

use std::path::PathBuf;
use std::sync::Arc;

use napi_derive::napi;
use uncontainerizable_core::{Adapter, App, ContainOptions};

use crate::adapter_bridge::DynamicAdapter;
use crate::container::NodeContainer;
use crate::errors::to_napi;
use crate::types::JsContainOptions;

#[napi]
pub struct NodeApp {
    inner: App,
}

#[napi]
impl NodeApp {
    /// Create a namespaced app handle. `prefix` is validated to the
    /// identity char set; throws `INVALID_IDENTITY` if malformed.
    #[napi(constructor)]
    pub fn new(prefix: String) -> napi::Result<Self> {
        let inner = App::new(prefix).map_err(to_napi)?;
        Ok(Self { inner })
    }

    #[napi(getter)]
    pub fn prefix(&self) -> String {
        self.inner.prefix().to_string()
    }

    /// Spawn a contained process. If `opts.identity` is set, a prior
    /// matching instance is killed first. On macOS Launch Services `.app`
    /// launches, matching is bundle-scoped rather than `prefix + identity`
    /// scoped.
    #[napi]
    pub async fn contain(
        &self,
        command: String,
        opts: Option<JsContainOptions>,
    ) -> napi::Result<NodeContainer> {
        let opts = opts.unwrap_or(JsContainOptions {
            args: None,
            env: None,
            cwd: None,
            identity: None,
            darwin_tag_argv0: None,
            adapters: None,
        });
        let adapters: Vec<Arc<dyn Adapter>> = opts
            .adapters
            .unwrap_or_default()
            .into_iter()
            .map(|js| Arc::new(DynamicAdapter::from(js)) as Arc<dyn Adapter>)
            .collect();
        let core_opts = ContainOptions {
            args: opts.args.unwrap_or_default(),
            env: opts.env.unwrap_or_default().into_iter().collect(),
            cwd: opts.cwd.map(PathBuf::from),
            adapters,
            // Default `true` on Darwin so direct-exec identity preemption
            // works. Launch Services `.app` launches ignore argv tagging.
            darwin_tag_argv0: opts.darwin_tag_argv0.unwrap_or(true),
            identity: opts.identity,
        };
        let container = self
            .inner
            .contain(&command, core_opts)
            .await
            .map_err(to_napi)?;
        Ok(NodeContainer::wrap(container))
    }
}

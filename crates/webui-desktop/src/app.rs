// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

use serde::Serialize;
use serde_json::Value;
use std::collections::HashMap;
use std::path::PathBuf;

use crate::error::{DesktopError, Result};
use crate::ipc::IpcRegistry;
use crate::runtime::{DesktopRuntime, DesktopSourceConfig};

/// Builder for Rust-first WebUI desktop hosts.
///
/// Use this API when the desktop app owns state in Rust and wants to provide
/// route-scoped state to `@microsoft/webui-router`.
pub struct DesktopAppBuilder {
    config: DesktopSourceConfig,
}

impl DesktopAppBuilder {
    /// Create a desktop app builder from WebUI build options.
    #[must_use]
    pub fn new(build_options: webui::BuildOptions) -> Self {
        Self {
            config: DesktopSourceConfig::new(build_options),
        }
    }

    /// Set startup state from any serializable Rust value.
    ///
    /// # Errors
    ///
    /// Returns [`DesktopError`] if state serialization fails.
    pub fn state<T: Serialize>(mut self, state: &T) -> Result<Self> {
        self.config.state =
            Some(
                serde_json::to_value(state).map_err(|source| DesktopError::Serialization {
                    context: "serializing desktop app state".to_string(),
                    source,
                })?,
            );
        Ok(self)
    }

    /// Set startup state from an existing JSON value.
    #[must_use]
    pub fn state_value(mut self, state: Value) -> Self {
        self.config.state = Some(state);
        self
    }

    /// Set static asset root.
    #[must_use]
    pub fn asset_root(mut self, path: impl Into<std::path::PathBuf>) -> Self {
        self.config.asset_root = Some(path.into());
        self
    }

    /// Set pre-resolved design token CSS.
    #[must_use]
    pub fn token_css(mut self, token_css: HashMap<String, String>) -> Self {
        self.config.token_css = Some(token_css);
        self
    }

    /// Resolve a design token theme after the WebUI protocol is built.
    ///
    /// The `search_root` follows the same resolution rules as `webui serve
    /// --theme`: it may point at an app directory containing `node_modules`.
    #[must_use]
    pub fn theme(mut self, theme: impl Into<String>, search_root: impl Into<PathBuf>) -> Self {
        self.config.theme = Some((theme.into(), search_root.into()));
        self
    }

    /// Set the protobuf IPC registry.
    #[must_use]
    pub fn ipc_registry(mut self, registry: IpcRegistry) -> Self {
        self.config.ipc_registry = registry;
        self
    }

    /// Register a Rust route state provider.
    ///
    /// Patterns support literal segments and `:param` captures, e.g.
    /// `/contacts/:id`.
    ///
    /// # Errors
    ///
    /// Returns [`DesktopError`] if the pattern is invalid.
    pub fn route<F>(mut self, pattern: impl AsRef<str>, handler: F) -> Result<Self>
    where
        F: Fn(crate::runtime::RouteContext<'_>) -> Result<Value> + Send + Sync + 'static,
    {
        self.config.route_state.route(pattern, handler)?;
        Ok(self)
    }

    /// Register a Rust custom-protocol API handler.
    ///
    /// Patterns support literal segments and `:param` captures, e.g.
    /// `/api/contacts/:id`.
    ///
    /// # Errors
    ///
    /// Returns [`DesktopError`] if the pattern is invalid.
    pub fn api_route<F>(mut self, pattern: impl AsRef<str>, handler: F) -> Result<Self>
    where
        F: Fn(crate::runtime::ApiContext<'_>) -> Result<crate::DesktopProtocolResponse>
            + Send
            + Sync
            + 'static,
    {
        self.config.api_routes.route(pattern, handler)?;
        Ok(self)
    }

    /// Build the desktop runtime.
    ///
    /// # Errors
    ///
    /// Returns [`DesktopError`] if the WebUI build or startup render fails.
    pub fn build(self) -> Result<DesktopRuntime> {
        DesktopRuntime::from_source(self.config)
    }
}

/// Entry point for constructing Rust-first desktop apps.
pub struct DesktopApp;

impl DesktopApp {
    /// Create a desktop app builder from WebUI build options.
    #[must_use]
    pub fn builder(build_options: webui::BuildOptions) -> DesktopAppBuilder {
        DesktopAppBuilder::new(build_options)
    }
}

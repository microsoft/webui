// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

use serde::Serialize;
use serde_json::Value;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use crate::error::{DesktopError, Result};
use crate::frame::DesktopFrame;
use crate::ipc::{IpcOptions, IpcRegistry};
#[cfg(feature = "source")]
use crate::runtime::DesktopSourceConfig;
use crate::runtime::{DesktopBundleConfig, DesktopRuntime};
use crate::{DesktopBundleManifest, DesktopShellConfig, WindowOptions};

enum AppInput {
    #[cfg(feature = "source")]
    Source(Box<SourceApp>),
    Bundle(Box<BundleApp>),
}

#[cfg(feature = "source")]
struct SourceApp {
    config: DesktopSourceConfig,
    shell: DesktopShellConfig,
    app_id: Option<String>,
}

struct BundleApp {
    config: DesktopBundleConfig,
    manifest: DesktopBundleManifest,
}

/// Builder for Rust-first WebUI desktop hosts.
///
/// Source and bundle inputs share registration and window configuration. No
/// startup rendering occurs until [`Self::build`], so route providers and Rust
/// state participate in the first render, not only subsequent requests.
pub struct DesktopAppBuilder {
    input: AppInput,
    ipc_options: IpcOptions,
}

impl DesktopAppBuilder {
    /// Set the window options used by both rendering and the native frame.
    #[must_use]
    pub fn window(mut self, window: WindowOptions) -> Self {
        match &mut self.input {
            #[cfg(feature = "source")]
            AppInput::Source(app) => app.config.window = window,
            AppInput::Bundle(app) => app.manifest.window = window,
        }
        self
    }

    /// Set native shell configuration, overriding any bundled defaults.
    #[must_use]
    pub fn shell(mut self, shell: DesktopShellConfig) -> Self {
        match &mut self.input {
            #[cfg(feature = "source")]
            AppInput::Source(app) => app.shell = shell,
            AppInput::Bundle(app) => app.manifest.shell = shell,
        }
        self
    }

    /// Set the stable application identity used by the native frame.
    #[must_use]
    pub fn app_id(mut self, app_id: impl Into<String>) -> Self {
        let app_id = app_id.into();
        match &mut self.input {
            #[cfg(feature = "source")]
            AppInput::Source(app) => app.app_id = Some(app_id),
            AppInput::Bundle(app) => app.manifest.app_id = app_id,
        }
        self
    }

    /// Set the maximum asset bytes buffered in one protocol response.
    #[must_use]
    pub fn max_asset_bytes(mut self, max_asset_bytes: u64) -> Self {
        match &mut self.input {
            #[cfg(feature = "source")]
            AppInput::Source(app) => app.config.max_asset_bytes = max_asset_bytes,
            AppInput::Bundle(app) => app.config.max_asset_bytes = max_asset_bytes,
        }
        self
    }

    /// Set startup state from any serializable Rust value.
    ///
    /// # Errors
    ///
    /// Returns [`DesktopError`] if state serialization fails.
    pub fn state<T: Serialize>(self, state: &T) -> Result<Self> {
        let value = serde_json::to_value(state).map_err(|source| DesktopError::Serialization {
            context: "serializing desktop app state".to_string(),
            source,
        })?;
        Ok(self.state_value(value))
    }

    /// Set startup state from an existing JSON value.
    #[must_use]
    pub fn state_value(mut self, state: Value) -> Self {
        match &mut self.input {
            #[cfg(feature = "source")]
            AppInput::Source(app) => app.config.state = Some(state),
            AppInput::Bundle(app) => app.config.state = Some(state),
        }
        self
    }

    /// Set pre-resolved design token CSS.
    #[must_use]
    pub fn token_css(mut self, token_css: HashMap<String, String>) -> Self {
        match &mut self.input {
            #[cfg(feature = "source")]
            AppInput::Source(app) => app.config.token_css = Some(token_css),
            AppInput::Bundle(app) => app.config.token_css = Some(token_css),
        }
        self
    }

    /// Set the protobuf IPC registry.
    #[must_use]
    pub fn ipc_registry(mut self, registry: IpcRegistry) -> Self {
        match &mut self.input {
            #[cfg(feature = "source")]
            AppInput::Source(app) => app.config.ipc_registry = registry,
            AppInput::Bundle(app) => app.config.ipc_registry = registry,
        }
        self
    }

    /// Configure application IPC permissions and bounded resource limits.
    ///
    /// The default denies all application methods. Use
    /// [`IpcOptions::for_schema`] to explicitly grant a generated contract.
    #[must_use]
    pub fn ipc_options(mut self, options: IpcOptions) -> Self {
        self.ipc_options = options;
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
        match &mut self.input {
            #[cfg(feature = "source")]
            AppInput::Source(app) => app.config.route_state.route(pattern, handler)?,
            AppInput::Bundle(app) => app.config.route_state.route(pattern, handler)?,
        }
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
        match &mut self.input {
            #[cfg(feature = "source")]
            AppInput::Source(app) => app.config.api_routes.route(pattern, handler)?,
            AppInput::Bundle(app) => app.config.api_routes.route(pattern, handler)?,
        }
        Ok(self)
    }

    /// Render startup content and construct the owning desktop frame.
    ///
    /// # Errors
    ///
    /// Returns [`DesktopError`] if source compilation, bundle loading, a route
    /// provider, or startup rendering fails.
    pub fn build(self) -> Result<DesktopFrame> {
        match self.input {
            #[cfg(feature = "source")]
            AppInput::Source(app) => {
                let SourceApp {
                    config,
                    shell,
                    app_id,
                } = *app;
                let window = config.window.clone();
                let runtime = DesktopRuntime::from_source(config)?;
                let mut frame =
                    DesktopFrame::with_ipc_options(Arc::new(runtime), window, self.ipc_options)?
                        .with_shell(shell);
                frame.app_id = app_id;
                Ok(frame)
            }
            AppInput::Bundle(app) => {
                let BundleApp { config, manifest } = *app;
                let window = manifest.window.clone();
                let shell = manifest.shell.clone();
                let app_id = manifest.app_id.clone();
                let runtime = DesktopRuntime::from_bundle_config_and_manifest(config, manifest)?;
                Ok(
                    DesktopFrame::with_ipc_options(Arc::new(runtime), window, self.ipc_options)?
                        .with_shell(shell)
                        .with_app_id(app_id),
                )
            }
        }
    }
}

/// Entry point for constructing Rust-first desktop apps.
pub struct DesktopApp;

impl DesktopApp {
    /// Construct a source-backed app without compiling or rendering it yet.
    ///
    /// Source-only asset and theme settings belong on `config`. Register shared
    /// state and handlers on the returned builder before calling `build`.
    #[cfg(feature = "source")]
    #[must_use]
    pub fn from_source(config: DesktopSourceConfig) -> DesktopAppBuilder {
        DesktopAppBuilder {
            ipc_options: IpcOptions::default(),
            input: AppInput::Source(Box::new(SourceApp {
                config,
                shell: DesktopShellConfig::default(),
                app_id: None,
            })),
        }
    }

    /// Load bundle metadata and prepare an app for shared handler registration.
    ///
    /// # Errors
    ///
    /// Returns [`DesktopError`] when the bundle manifest cannot be read or parsed.
    pub fn from_bundle(bundle_dir: impl Into<PathBuf>) -> Result<DesktopAppBuilder> {
        Self::from_bundle_config(DesktopBundleConfig::new(bundle_dir.into()))
    }

    /// Prepare a bundle-backed app with existing Rust state and registrations.
    ///
    /// # Errors
    ///
    /// Returns [`DesktopError`] when the bundle manifest cannot be read or parsed.
    pub fn from_bundle_config(config: DesktopBundleConfig) -> Result<DesktopAppBuilder> {
        let manifest =
            DesktopBundleManifest::load(&config.bundle_dir.join("manifest.webui-desktop.json"))?;
        Ok(Self::from_bundle_config_and_manifest(config, manifest))
    }

    /// Prepare a bundle-backed app using an already-loaded manifest.
    ///
    /// This avoids reading metadata twice when the host also needs it to
    /// initialize application storage or other shared services.
    #[must_use]
    pub fn from_bundle_config_and_manifest(
        config: DesktopBundleConfig,
        manifest: DesktopBundleManifest,
    ) -> DesktopAppBuilder {
        DesktopAppBuilder {
            ipc_options: IpcOptions::default(),
            input: AppInput::Bundle(Box::new(BundleApp { config, manifest })),
        }
    }
}

/// Run a packaged app with bundled seed state and default Rust registrations.
///
/// Hosts needing route or IPC handlers should configure [`DesktopApp`] and
/// launch the resulting frame instead.
///
/// # Errors
///
/// Returns [`DesktopError`] when resources are missing, bundle loading fails,
/// or the native backend fails to initialize or run.
#[cfg(feature = "native")]
pub fn run_packaged_app() -> Result<()> {
    let resources = crate::frame::find_packaged_resources_dir()
        .ok_or(DesktopError::PackagedResourcesNotFound)?;
    crate::frame::run_frame(DesktopApp::from_bundle(resources)?.build()?)
}

#[cfg(test)]
#[allow(clippy::disallowed_methods)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use crate::frame::test_support;
    use crate::{BundleAsset, DesktopProtocolRequest, DesktopProtocolResponse, TitlebarStyle};

    #[derive(Serialize)]
    struct Seed {
        label: &'static str,
    }

    fn configure(builder: DesktopAppBuilder, calls: Arc<AtomicUsize>) -> Result<DesktopAppBuilder> {
        builder
            .state(&Seed { label: "seed" })?
            .token_css(HashMap::from([(
                "default".to_string(),
                "--accent:red".to_string(),
            )]))
            .max_asset_bytes(1024)
            .app_id("com.example.configured")
            .window(WindowOptions {
                title: "Configured".to_string(),
                titlebar: TitlebarStyle::Overlay { height: 48 },
                background: Some("#123456".parse().unwrap()),
                ..WindowOptions::default()
            })
            .shell(DesktopShellConfig {
                icon_path: Some(PathBuf::from("assets/icon.png")),
                ..DesktopShellConfig::default()
            })
            .ipc_registry(crate::ipc_test_support::registry())
            .ipc_options(crate::ipc_test_support::options())
            .route("/", move |ctx| {
                calls.fetch_add(1, Ordering::SeqCst);
                assert_eq!(ctx.base_state["label"], "seed");
                Ok(ctx.base_state.clone())
            })?
            .api_route("/api/:id", |ctx| {
                Ok(DesktopProtocolResponse::text(200, ctx.param("id").unwrap()))
            })
    }

    fn assert_configured_frame(frame: &DesktopFrame, calls: &AtomicUsize) {
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        assert_eq!(frame.app_id.as_deref(), Some("com.example.configured"));
        assert_eq!(frame.window.title, "Configured");
        assert_eq!(
            frame.shell.icon_path.as_deref(),
            Some(std::path::Path::new("assets/icon.png"))
        );
        assert!(frame
            .runtime
            .startup_html()
            .contains("--webui-titlebar-height:48px"));
        assert!(frame
            .runtime
            .startup_html()
            .contains("--webui-window-background:#123456"));
        let response = frame
            .runtime
            .handle_request(&DesktopProtocolRequest::get("/api/registered"))
            .unwrap();
        assert_eq!(response.body, b"registered");

        crate::ipc_test_support::assert_echo(frame, b"registered");
    }

    #[test]
    fn bundle_preserves_manifest_identity_window_and_shell_without_reloading_it() {
        let (dir, mut manifest) = test_support::bundle();
        manifest.window.title = "Bundled".to_string();
        manifest.window.background = Some("#abcdef".parse().unwrap());
        manifest.shell.icon_path = Some(PathBuf::from("assets/bundled.png"));
        std::fs::remove_file(dir.path().join("manifest.webui-desktop.json")).unwrap();

        let frame = DesktopApp::from_bundle_config_and_manifest(
            DesktopBundleConfig::new(dir.path().to_path_buf()),
            manifest,
        )
        .build()
        .unwrap();

        assert_eq!(
            frame.app_id.as_deref(),
            Some("com.example.webui.frame-test")
        );
        assert_eq!(frame.window.title, "Bundled");
        assert_eq!(
            frame.shell.icon_path.as_deref(),
            Some(std::path::Path::new("assets/bundled.png"))
        );
        assert!(frame
            .runtime
            .startup_html()
            .contains("--webui-window-background:#abcdef"));
    }

    #[test]
    fn bundle_shared_handlers_and_overrides_are_installed_before_startup_render() {
        let (dir, _) = test_support::bundle();
        let calls = Arc::new(AtomicUsize::new(0));
        let builder = configure(
            DesktopApp::from_bundle(dir.path()).unwrap(),
            Arc::clone(&calls),
        )
        .unwrap();
        assert_eq!(calls.load(Ordering::SeqCst), 0);
        // Construction already loaded metadata; build must not reload it.
        std::fs::remove_file(dir.path().join("manifest.webui-desktop.json")).unwrap();

        let frame = builder.build().unwrap();

        assert_configured_frame(&frame, &calls);
    }

    #[test]
    fn bundle_config_keeps_existing_registrations_and_accepts_json_state_override() {
        let (dir, _) = test_support::bundle();
        let mut config = DesktopBundleConfig::new(dir.path().to_path_buf());
        config
            .api_routes
            .route("/api/preset", |_| {
                Ok(DesktopProtocolResponse::text(200, "preset"))
            })
            .unwrap();
        let builder = DesktopApp::from_bundle_config(config)
            .unwrap()
            .state_value(serde_json::to_value(Seed { label: "override" }).unwrap())
            .route("/", |ctx| {
                assert_eq!(ctx.base_state["label"], "override");
                Ok(Value::Null)
            })
            .unwrap();

        let frame = builder.build().unwrap();

        assert_eq!(
            frame
                .runtime
                .handle_request(&DesktopProtocolRequest::get("/api/preset"))
                .unwrap()
                .body,
            b"preset"
        );
    }

    #[test]
    fn bundle_asset_limit_override_reaches_runtime_validation() {
        let (dir, mut manifest) = test_support::bundle();
        std::fs::write(dir.path().join("assets/large.txt"), b"large").unwrap();
        manifest.integrity.assets.push(BundleAsset {
            path: "assets/large.txt".to_string(),
            sha256: String::new(),
            size_bytes: 5,
        });
        let result = DesktopApp::from_bundle_config_and_manifest(
            DesktopBundleConfig::new(dir.path().to_path_buf()),
            manifest,
        )
        .max_asset_bytes(4)
        .build();

        assert!(matches!(result, Err(DesktopError::AssetTooLarge { .. })));
    }

    #[test]
    fn invalid_registrations_and_provider_failures_are_explicit() {
        let (dir, _) = test_support::bundle();
        assert!(matches!(
            DesktopApp::from_bundle(dir.path())
                .unwrap()
                .route("invalid", |_| Ok(Value::Null)),
            Err(DesktopError::InvalidRoutePattern { .. })
        ));
        assert!(matches!(
            DesktopApp::from_bundle(dir.path())
                .unwrap()
                .api_route("invalid", |_| Ok(DesktopProtocolResponse::text(200, ""))),
            Err(DesktopError::InvalidRoutePattern { .. })
        ));
        let result = DesktopApp::from_bundle(dir.path())
            .unwrap()
            .route("/", |_| {
                Err(DesktopError::UnsupportedRuntime {
                    message: "fixture provider failure".to_string(),
                    help: "return route state".to_string(),
                })
            })
            .unwrap()
            .build();
        assert!(matches!(result, Err(DesktopError::RouteProvider { .. })));
    }

    #[test]
    fn state_serialization_failure_is_returned() {
        struct InvalidState;
        impl Serialize for InvalidState {
            fn serialize<S: serde::Serializer>(
                &self,
                _serializer: S,
            ) -> std::result::Result<S::Ok, S::Error> {
                Err(serde::ser::Error::custom("fixture serialization failure"))
            }
        }

        let (dir, _) = test_support::bundle();
        assert!(matches!(
            DesktopApp::from_bundle(dir.path())
                .unwrap()
                .state(&InvalidState),
            Err(DesktopError::Serialization { .. })
        ));
    }

    #[cfg(feature = "source")]
    #[test]
    fn source_uses_the_same_handler_and_render_configuration() {
        let dir = tempfile::TempDir::new().unwrap();
        let config = DesktopSourceConfig::new(crate::BuildOptions {
            app_dir: dir.path().to_path_buf(),
            ..Default::default()
        });
        let calls = Arc::new(AtomicUsize::new(0));
        let builder = configure(DesktopApp::from_source(config), Arc::clone(&calls)).unwrap();
        // A source builder also defers compilation until build.
        std::fs::write(dir.path().join("index.html"), "<main>Hello</main>").unwrap();
        assert_eq!(calls.load(Ordering::SeqCst), 0);

        let frame = builder.build().unwrap();

        assert_configured_frame(&frame, &calls);
    }
}

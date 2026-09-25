// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

use std::collections::HashMap;
use std::fs;
use std::path::{Component, Path, PathBuf};
#[cfg(feature = "application-ipc")]
use std::sync::Arc;

use serde_json::Value;
use webui_handler::{Protocol, RenderOptions, ResponseWriter, WebUIHandler};

use crate::error::{DesktopError, Result};
use crate::hydration::handler_for_name;
#[cfg(feature = "source")]
use crate::hydration::handler_for_plugin;
#[cfg(feature = "application-ipc")]
use crate::ipc::IpcRegistry;
use crate::path::resolve_safe_path;
#[cfg(feature = "application-ipc")]
use crate::protocol::IPC_ENDPOINT;
use crate::protocol::{
    read_asset_response, read_known_asset_response, DesktopHttpMethod, DesktopProtocolRequest,
    DesktopProtocolResponse, DEFAULT_MAX_ASSET_BYTES,
};
use crate::routes::route_path;
use crate::routes::{ApiRouteRegistry, RouteStateRegistry};
use crate::{apply_window_css, window_css_block, DesktopPlatform};

/// Source-backed desktop runtime configuration.
#[cfg(feature = "source")]
pub struct DesktopSourceConfig {
    /// WebUI build options.
    pub build_options: webui::BuildOptions,
    /// Optional state JSON file.
    pub state_file: Option<PathBuf>,
    /// Optional in-memory startup state supplied by Rust app hosts.
    pub state: Option<Value>,
    /// Optional static asset root.
    pub asset_root: Option<PathBuf>,
    /// Maximum file length delivered by one protocol response.
    pub max_asset_bytes: u64,
    /// Protobuf IPC registry.
    #[cfg(feature = "application-ipc")]
    pub ipc_registry: IpcRegistry,
    /// Rust route state providers.
    pub route_state: RouteStateRegistry,
    /// Rust custom-protocol API handlers.
    pub api_routes: ApiRouteRegistry,
    /// Optional pre-resolved design token CSS keyed by theme name.
    pub token_css: Option<HashMap<String, String>>,
    /// Optional theme value and search root to resolve after protocol build.
    pub theme: Option<(String, PathBuf)>,
    /// Window configuration used to derive injected window CSS.
    ///
    /// Source-mode hosts must supply the same [`crate::WindowOptions`] they pass
    /// to the native runner. Frame construction rejects mismatched
    /// titlebar/background styling.
    pub window: crate::WindowOptions,
}

#[cfg(feature = "source")]
impl DesktopSourceConfig {
    /// Create a source config from WebUI build options.
    #[must_use]
    pub fn new(build_options: webui::BuildOptions) -> Self {
        Self {
            build_options,
            state_file: None,
            state: None,
            asset_root: None,
            max_asset_bytes: DEFAULT_MAX_ASSET_BYTES,
            #[cfg(feature = "application-ipc")]
            ipc_registry: IpcRegistry::default(),
            route_state: RouteStateRegistry::new(),
            api_routes: ApiRouteRegistry::new(),
            token_css: None,
            theme: None,
            window: crate::WindowOptions::default(),
        }
    }
}

/// Bundle-backed desktop runtime configuration.
///
/// Use this from app-specific packaged runners that load `protocol.bin` and
/// immutable assets from a bundle while registering Rust-owned route state and
/// typed IPC handlers in the executable.
pub struct DesktopBundleConfig {
    /// Desktop bundle directory created by `webui desktop build`.
    pub bundle_dir: PathBuf,
    /// Optional Rust-owned startup state. When omitted, bundled `state.json` is used.
    pub state: Option<Value>,
    /// Maximum file length delivered by one protocol response.
    pub max_asset_bytes: u64,
    /// Protobuf IPC registry.
    #[cfg(feature = "application-ipc")]
    pub ipc_registry: IpcRegistry,
    /// Rust route state providers.
    pub route_state: RouteStateRegistry,
    /// Rust custom-protocol API handlers.
    pub api_routes: ApiRouteRegistry,
    /// Optional pre-resolved design token CSS keyed by theme name.
    pub token_css: Option<HashMap<String, String>>,
}

impl DesktopBundleConfig {
    /// Create a bundle config with empty route/IPC registries.
    #[must_use]
    pub fn new(bundle_dir: PathBuf) -> Self {
        Self {
            bundle_dir,
            state: None,
            max_asset_bytes: DEFAULT_MAX_ASSET_BYTES,
            #[cfg(feature = "application-ipc")]
            ipc_registry: IpcRegistry::default(),
            route_state: RouteStateRegistry::new(),
            api_routes: ApiRouteRegistry::new(),
            token_css: None,
        }
    }
}

/// Runtime state shared by a desktop webview custom-protocol handler.
pub struct DesktopRuntime {
    protocol: Protocol,
    entry: String,
    state: Value,
    css_files: HashMap<String, String>,
    asset_root: Option<PathBuf>,
    #[cfg(all(feature = "native", target_os = "macos"))]
    bundle_root: Option<PathBuf>,
    asset_index: HashMap<PathBuf, DesktopAssetEntry>,
    max_asset_bytes: u64,
    startup_html: String,
    window_css: String,
    #[cfg(feature = "application-ipc")]
    ipc_registry: Arc<IpcRegistry>,
    #[cfg(feature = "application-ipc")]
    development: bool,
    handler: WebUIHandler,
    route_state: RouteStateRegistry,
    api_routes: ApiRouteRegistry,
    token_css: Option<HashMap<String, String>>,
}

struct DesktopAssetEntry {
    path: PathBuf,
    content_type: String,
    size_bytes: u64,
}

impl DesktopRuntime {
    /// Build and render a desktop runtime from source paths.
    ///
    /// # Errors
    ///
    /// Returns [`DesktopError`] if the WebUI build fails, state cannot be read
    /// or parsed, assets cannot be canonicalized, or startup rendering fails.
    #[cfg(feature = "source")]
    pub fn from_source(config: DesktopSourceConfig) -> Result<Self> {
        let build_result = webui::build(config.build_options.clone())?;
        let state = match config.state {
            Some(state) => state,
            None => read_state(config.state_file.as_ref())?,
        };
        let token_css = resolve_config_token_css(
            config.token_css,
            config.theme,
            &build_result.protocol.tokens,
        )?;
        let asset_root = canonical_asset_root(config.asset_root.as_ref())?;
        let css_files = build_result.css_files.into_iter().collect();
        let protocol = Protocol::new(build_result.protocol);
        let startup_state = state_for_request(StateRequestContext {
            protocol: &protocol,
            entry: &config.build_options.entry,
            base_state: &state,
            registry: &config.route_state,
            token_css: token_css.as_ref(),
            request_path: "/",
        })?;
        let handler = handler_for_plugin(config.build_options.plugin);
        let window_css = window_css_block(&config.window, DesktopPlatform::current());
        let startup_html = apply_window_css(
            render_html(
                &protocol,
                &handler,
                &config.build_options.entry,
                "/",
                &startup_state,
            )?,
            &window_css,
        );

        Ok(Self {
            protocol,
            entry: config.build_options.entry,
            state,
            css_files,
            asset_root,
            #[cfg(all(feature = "native", target_os = "macos"))]
            bundle_root: None,
            asset_index: HashMap::new(),
            max_asset_bytes: config.max_asset_bytes,
            startup_html,
            window_css,
            #[cfg(feature = "application-ipc")]
            ipc_registry: Arc::new(config.ipc_registry),
            #[cfg(feature = "application-ipc")]
            development: true,
            handler,
            route_state: config.route_state,
            api_routes: config.api_routes,
            token_css,
        })
    }

    /// Load a desktop runtime from a bundle directory.
    ///
    /// # Errors
    ///
    /// Returns [`DesktopError`] if the bundle manifest, protocol, state, or
    /// asset root cannot be loaded.
    pub fn from_bundle(bundle_dir: PathBuf) -> Result<Self> {
        Self::from_bundle_config(DesktopBundleConfig::new(bundle_dir))
    }

    /// Load a desktop runtime from a bundle directory with Rust host state.
    ///
    /// # Errors
    ///
    /// Returns [`DesktopError`] if the bundle manifest, protocol, state, or
    /// asset root cannot be loaded, or if startup rendering fails.
    pub fn from_bundle_config(config: DesktopBundleConfig) -> Result<Self> {
        let bundle_root = canonical_bundle_root(&config.bundle_dir)?;
        let manifest =
            crate::DesktopBundleManifest::load(&bundle_root.join("manifest.webui-desktop.json"))?;
        Self::from_canonical_bundle_config_and_manifest(config, bundle_root, manifest)
    }

    /// Load a desktop runtime from a bundle directory with an already-loaded manifest.
    ///
    /// Use this when the caller also needs manifest metadata such as window or
    /// shell configuration, so startup does not read and parse the manifest
    /// twice.
    ///
    /// # Errors
    ///
    /// Returns [`DesktopError`] if the protocol, state, or asset root cannot be
    /// loaded, or if startup rendering fails.
    pub fn from_bundle_config_and_manifest(
        config: DesktopBundleConfig,
        manifest: crate::DesktopBundleManifest,
    ) -> Result<Self> {
        let bundle_root = canonical_bundle_root(&config.bundle_dir)?;
        Self::from_canonical_bundle_config_and_manifest(config, bundle_root, manifest)
    }

    fn from_canonical_bundle_config_and_manifest(
        config: DesktopBundleConfig,
        bundle_root: PathBuf,
        manifest: crate::DesktopBundleManifest,
    ) -> Result<Self> {
        let protocol_path =
            resolve_manifest_path(&bundle_root, &manifest.protocol_path, "protocol")?;
        let protocol_bytes = fs::read(&protocol_path).map_err(|source| DesktopError::Io {
            context: format!("reading desktop protocol {}", protocol_path.display()),
            source,
        })?;
        let protocol = Protocol::from_protobuf(&protocol_bytes)?;
        let state_path = manifest
            .state_path
            .as_ref()
            .map(|path| resolve_manifest_path(&bundle_root, path, "state"))
            .transpose()?;
        let state = match config.state {
            Some(state) => state,
            None => read_state(state_path.as_ref())?,
        };
        let asset_root = resolve_manifest_path(&bundle_root, &manifest.assets_dir, "assets")?;
        let asset_index = build_asset_index(
            &asset_root,
            &manifest.integrity.assets,
            config.max_asset_bytes,
        )?;
        let handler = handler_for_name(manifest.plugin.as_deref());
        let startup_state = state_for_request(StateRequestContext {
            protocol: &protocol,
            entry: &manifest.entry,
            base_state: &state,
            registry: &config.route_state,
            token_css: config.token_css.as_ref(),
            request_path: "/",
        })?;
        let window_css = window_css_block(&manifest.window, DesktopPlatform::current());
        let startup_html = apply_window_css(
            render_html(&protocol, &handler, &manifest.entry, "/", &startup_state)?,
            &window_css,
        );

        Ok(Self {
            protocol,
            entry: manifest.entry,
            state,
            css_files: HashMap::new(),
            asset_root: Some(asset_root),
            #[cfg(all(feature = "native", target_os = "macos"))]
            bundle_root: Some(bundle_root),
            asset_index,
            max_asset_bytes: config.max_asset_bytes,
            startup_html,
            window_css,
            #[cfg(feature = "application-ipc")]
            ipc_registry: Arc::new(config.ipc_registry),
            #[cfg(feature = "application-ipc")]
            development: false,
            handler,
            route_state: config.route_state,
            api_routes: config.api_routes,
            token_css: config.token_css,
        })
    }

    /// Handle one custom-protocol request.
    ///
    /// This method is intentionally independent of any specific webview crate,
    /// so path safety, IPC dispatch, and routing can be unit tested without
    /// creating an OS window.
    pub fn handle_request(
        &self,
        request: &DesktopProtocolRequest<'_>,
    ) -> Result<DesktopProtocolResponse> {
        let path = route_path(request.path);
        #[cfg(feature = "application-ipc")]
        if matches!(request.method, DesktopHttpMethod::Get) {
            if let Some(response) = crate::ipc_assets::response(path) {
                return Ok(response);
            }
        }
        #[cfg(feature = "application-ipc")]
        if path == IPC_ENDPOINT || path == "/_webui/ipc/outbound" {
            return Err(DesktopError::UnsupportedRuntime {
                message: "application IPC requires a frame-scoped document session".to_string(),
                help: "route reserved IPC requests through the desktop frame's IPC bridge"
                    .to_string(),
            });
        }

        if let Some(response) = self.api_routes.resolve(request)? {
            return Ok(response);
        }

        if matches!(request.method, DesktopHttpMethod::Get) {
            let request_path = path;
            if let Some(css) = self.generated_css(request_path) {
                return Ok(DesktopProtocolResponse::new(
                    200,
                    "text/css; charset=utf-8",
                    css.as_bytes().to_vec(),
                ));
            }

            if request_path != "/" && request_path != "/index.html" {
                if let Some(response) = self.asset_response(request_path)? {
                    return Ok(response);
                }
            }

            if request.wants_json {
                return self.partial_response(request.path);
            }

            if request_path == "/" || request_path == "/index.html" {
                if self.route_state.has_provider("/") {
                    let html = apply_window_css(
                        render_html(
                            &self.protocol,
                            &self.handler,
                            &self.entry,
                            "/",
                            &self.state_for_request("/")?,
                        )?,
                        &self.window_css,
                    );
                    return Ok(DesktopProtocolResponse::html(html.into_bytes()));
                }
                return Ok(DesktopProtocolResponse::html(
                    self.startup_html.as_bytes().to_vec(),
                ));
            }

            if self.protocol.matches_route(&self.entry, request_path) {
                let html = apply_window_css(
                    render_html(
                        &self.protocol,
                        &self.handler,
                        &self.entry,
                        request_path,
                        &self.state_for_request(request_path)?,
                    )?,
                    &self.window_css,
                );
                return Ok(DesktopProtocolResponse::html(html.into_bytes()));
            }
        }

        Ok(DesktopProtocolResponse::text(404, "Not Found"))
    }

    /// Return the construction-time HTML snapshot rendered for `/`.
    ///
    /// This does not invoke route providers again. Use [`Self::handle_request`]
    /// for a fresh provider-backed root response.
    #[must_use]
    pub fn startup_html(&self) -> &str {
        &self.startup_html
    }

    #[cfg(all(feature = "native", target_os = "macos"))]
    pub(crate) fn bundle_root(&self) -> Option<&Path> {
        self.bundle_root.as_deref()
    }

    pub(crate) fn validate_window(&self, window: &crate::WindowOptions) -> Result<()> {
        if window_css_block(window, DesktopPlatform::current()) != self.window_css {
            return Err(DesktopError::UnsupportedRuntime {
                message: "frame window styling differs from the rendered desktop document".into(),
                help: "configure window options on DesktopAppBuilder before build, or use the same titlebar and background when constructing the runtime and frame".into(),
            });
        }
        Ok(())
    }

    #[cfg(feature = "application-ipc")]
    pub(crate) fn ipc_registry(&self) -> Arc<IpcRegistry> {
        Arc::clone(&self.ipc_registry)
    }

    #[cfg(feature = "application-ipc")]
    pub(crate) fn is_development(&self) -> bool {
        self.development
    }

    fn generated_css(&self, request_path: &str) -> Option<&str> {
        let name = request_path.trim_start_matches('/');
        self.css_files.get(name).map(String::as_str)
    }

    fn asset_response(&self, request_path: &str) -> Result<Option<DesktopProtocolResponse>> {
        let Some(root) = &self.asset_root else {
            return Ok(None);
        };

        if !self.asset_index.is_empty() {
            let Some(path) = resolve_safe_path(root, request_path) else {
                return Err(DesktopError::InvalidAssetPath {
                    path: request_path.to_string(),
                });
            };
            return match self.asset_index.get(&path) {
                Some(asset) => read_known_asset_response(
                    root,
                    &asset.path,
                    &asset.content_type,
                    asset.size_bytes,
                    self.max_asset_bytes,
                )
                .map(Some),
                None => Ok(None),
            };
        }

        let Some(path) = resolve_safe_path(root, request_path) else {
            return Err(DesktopError::InvalidAssetPath {
                path: request_path.to_string(),
            });
        };

        read_asset_response(root, path, self.max_asset_bytes)
    }

    fn partial_response(&self, request_path: &str) -> Result<DesktopProtocolResponse> {
        let route_path = route_path(request_path);
        let state = self.state_for_request(route_path)?;
        let partial = self
            .protocol
            .prepare_partial(state, &self.entry, route_path, "")?;
        if !partial.is_match() {
            return Ok(DesktopProtocolResponse::text(404, "Not Found"));
        }
        let body = serde_json::to_vec(&partial).map_err(|source| DesktopError::Serialization {
            context: "serializing desktop router partial".to_string(),
            source,
        })?;
        Ok(DesktopProtocolResponse::new(200, "application/json", body))
    }

    fn state_for_request(&self, request_path: &str) -> Result<Value> {
        state_for_request(StateRequestContext {
            protocol: &self.protocol,
            entry: &self.entry,
            base_state: &self.state,
            registry: &self.route_state,
            token_css: self.token_css.as_ref(),
            request_path,
        })
    }
}

fn build_asset_index(
    asset_root: &Path,
    assets: &[crate::BundleAsset],
    max_asset_bytes: u64,
) -> Result<HashMap<PathBuf, DesktopAssetEntry>> {
    let mut index = HashMap::with_capacity(assets.len());
    for asset in assets {
        let Some(relative) = asset.path.strip_prefix("assets/") else {
            return Err(DesktopError::InvalidAssetPath {
                path: asset.path.clone(),
            });
        };
        let relative = Path::new(relative);
        validate_manifest_relative_path(relative, "asset")?;
        let path = asset_root.join(relative);
        let canonical = path.canonicalize().map_err(|source| DesktopError::Io {
            context: format!("resolving indexed desktop asset {}", path.display()),
            source,
        })?;
        if !canonical.starts_with(asset_root) {
            return Err(DesktopError::InvalidAssetPath {
                path: asset.path.clone(),
            });
        }
        if asset.size_bytes > max_asset_bytes {
            return Err(DesktopError::AssetTooLarge {
                path,
                size: asset.size_bytes,
                max_bytes: max_asset_bytes,
            });
        }
        let content_type = mime_guess::from_path(&path)
            .first_or_octet_stream()
            .to_string();
        index.insert(
            path.clone(),
            DesktopAssetEntry {
                path,
                content_type,
                size_bytes: asset.size_bytes,
            },
        );
    }
    Ok(index)
}

struct StateRequestContext<'a> {
    protocol: &'a Protocol,
    entry: &'a str,
    base_state: &'a Value,
    registry: &'a RouteStateRegistry,
    token_css: Option<&'a HashMap<String, String>>,
    request_path: &'a str,
}

fn state_for_request(context: StateRequestContext<'_>) -> Result<Value> {
    let path = context
        .request_path
        .split_once('?')
        .map_or(context.request_path, |(path, _)| path);
    let mut state = match context.registry.resolve(path, context.base_state)? {
        Some(state) => state,
        None => context.base_state.clone(),
    };

    if let Value::Object(map) = &mut state {
        map.insert("basePath".to_string(), Value::String("/".to_string()));
        if context.token_css.is_none() && !map.contains_key("tokens") {
            if let Some(tokens) = context.base_state.get("tokens") {
                map.insert("tokens".to_string(), tokens.clone());
            }
        }
        let params = webui_handler::route_handler::collect_nested_route_params(
            context.protocol,
            context.entry,
            path,
        );
        for (key, value) in params {
            map.insert(key, Value::String(value));
        }
    }
    if let Some(token_css) = context.token_css {
        webui_tokens::inject_token_css(&mut state, token_css);
    }

    Ok(state)
}

#[cfg(feature = "source")]
fn resolve_config_token_css(
    token_css: Option<HashMap<String, String>>,
    theme: Option<(String, PathBuf)>,
    protocol_tokens: &[String],
) -> Result<Option<HashMap<String, String>>> {
    if token_css.is_some() {
        return Ok(token_css);
    }
    let Some((theme, search_root)) = theme else {
        return Ok(None);
    };
    let theme_path = webui_tokens::resolve_theme_path(&theme, &search_root).map_err(|source| {
        DesktopError::Token {
            context: format!("resolving desktop theme {theme}"),
            source,
        }
    })?;
    let token_file =
        webui_tokens::load_token_file(&theme_path).map_err(|source| DesktopError::Token {
            context: format!("loading desktop theme {}", theme_path.display()),
            source,
        })?;
    let resolved =
        webui_tokens::resolve_tokens(protocol_tokens, &token_file).map_err(|source| {
            DesktopError::Token {
                context: "resolving desktop theme tokens".to_string(),
                source,
            }
        })?;
    Ok(Some(resolved.css))
}

fn read_state(path: Option<&PathBuf>) -> Result<Value> {
    let Some(path) = path else {
        return Ok(Value::Object(serde_json::Map::new()));
    };

    let json = fs::read_to_string(path).map_err(|source| DesktopError::Io {
        context: format!("reading desktop state {}", path.display()),
        source,
    })?;
    serde_json::from_str(&json).map_err(|source| DesktopError::StateJson {
        path: path.clone(),
        source,
    })
}

fn canonical_bundle_root(bundle_dir: &Path) -> Result<PathBuf> {
    bundle_dir
        .canonicalize()
        .map_err(|source| DesktopError::Io {
            context: format!("resolving desktop bundle root {}", bundle_dir.display()),
            source,
        })
}

fn resolve_manifest_path(bundle_root: &Path, path: &Path, label: &str) -> Result<PathBuf> {
    validate_manifest_relative_path(path, label)?;
    let joined = bundle_root.join(path);
    let canonical = joined.canonicalize().map_err(|source| DesktopError::Io {
        context: format!("resolving desktop bundle {label} {}", joined.display()),
        source,
    })?;
    if !canonical.starts_with(bundle_root) {
        return Err(DesktopError::InvalidAssetPath {
            path: format!("{label}: {}", path.display()),
        });
    }
    Ok(canonical)
}

fn validate_manifest_relative_path(path: &Path, label: &str) -> Result<()> {
    for component in path.components() {
        match component {
            Component::Normal(segment)
                if segment
                    .to_str()
                    .is_some_and(|value| !value.contains('\\') && !value.contains('\0')) => {}
            _ => {
                return Err(DesktopError::InvalidAssetPath {
                    path: format!("{label}: {}", path.display()),
                });
            }
        }
    }
    Ok(())
}

#[cfg(feature = "source")]
fn canonical_asset_root(path: Option<&PathBuf>) -> Result<Option<PathBuf>> {
    match path {
        Some(path) => path
            .canonicalize()
            .map(Some)
            .map_err(|source| DesktopError::Io {
                context: format!("resolving desktop asset root {}", path.display()),
                source,
            }),
        None => Ok(None),
    }
}

fn render_html(
    protocol: &Protocol,
    handler: &WebUIHandler,
    entry: &str,
    request_path: &str,
    state: &Value,
) -> Result<String> {
    let mut writer = MemoryWriter::with_capacity(4096);
    handler.render(
        protocol,
        state,
        &RenderOptions::new(entry, request_path),
        &mut writer,
    )?;
    Ok(writer.buf)
}

struct MemoryWriter {
    buf: String,
}

impl MemoryWriter {
    fn with_capacity(capacity: usize) -> Self {
        Self {
            buf: String::with_capacity(capacity),
        }
    }
}

impl ResponseWriter for MemoryWriter {
    fn write(&mut self, content: &str) -> webui_handler::Result<()> {
        self.buf.push_str(content);
        Ok(())
    }

    fn end(&mut self) -> webui_handler::Result<()> {
        Ok(())
    }
}

#[cfg(all(test, feature = "source"))]
#[allow(clippy::disallowed_methods)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn write_file(root: &std::path::Path, path: &str, content: &str) {
        let full = root.join(path);
        if let Some(parent) = full.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(full, content).unwrap();
    }

    fn build_options(app_dir: PathBuf) -> webui::BuildOptions {
        webui::BuildOptions {
            app_dir,
            entry: "index.html".to_string(),
            // Scriptless WebUI fixtures provide exact template-derived
            // navigation keys without requiring a client bundle.
            plugin: Some(webui::Plugin::WebUI),
            ..webui::BuildOptions::default()
        }
    }

    #[test]
    fn renders_startup_html_from_source() {
        let dir = TempDir::new().unwrap();
        write_file(dir.path(), "index.html", "<main>Hello {{name}}</main>");
        write_file(dir.path(), "state.json", r#"{"name":"Desktop"}"#);

        let mut config = DesktopSourceConfig::new(build_options(dir.path().to_path_buf()));
        config.state_file = Some(dir.path().join("state.json"));

        let runtime = DesktopRuntime::from_source(config).unwrap();

        assert!(runtime.startup_html().contains("Hello Desktop"));
    }

    #[test]
    fn source_mode_injects_window_css_like_packaged_mode() {
        let dir = TempDir::new().unwrap();
        write_file(dir.path(), "index.html", "<main>Hello {{name}}</main>");
        write_file(dir.path(), "state.json", r#"{"name":"Desktop"}"#);

        let mut config = DesktopSourceConfig::new(build_options(dir.path().to_path_buf()));
        config.state_file = Some(dir.path().join("state.json"));
        config.window = crate::WindowOptions {
            titlebar: crate::TitlebarStyle::HiddenInset,
            background: Some("#101014".parse().unwrap()),
            ..crate::WindowOptions::default()
        };

        let runtime = DesktopRuntime::from_source(config).unwrap();

        // Development renders must carry the same titlebar custom properties as
        // packaged renders, otherwise a custom titlebar only works once shipped.
        assert!(runtime
            .startup_html()
            .contains("--webui-titlebar-inset-start"));
        assert!(runtime.startup_html().contains("--webui-window-background"));
    }

    #[test]
    fn non_root_route_renders_carry_window_css() {
        let dir = TempDir::new().unwrap();
        write_file(
            dir.path(),
            "index.html",
            "<route path=\"/\" component=\"my-page\"><route path=\"favorites\" component=\"my-page\" exact /></route>",
        );
        write_file(dir.path(), "my-page.html", "<p>{{page}}</p>");

        let mut config = DesktopSourceConfig::new(build_options(dir.path().to_path_buf()));
        config.window = crate::WindowOptions {
            titlebar: crate::TitlebarStyle::HiddenInset,
            ..crate::WindowOptions::default()
        };
        config
            .route_state
            .route("/favorites", |_| {
                Ok(serde_json::json!({ "page": "favorites" }))
            })
            .unwrap();

        let runtime = DesktopRuntime::from_source(config).unwrap();
        let response = runtime
            .handle_request(&DesktopProtocolRequest::get("/favorites"))
            .unwrap();

        assert_eq!(response.status, 200);
        let html = std::str::from_utf8(response.body.as_bytes().unwrap().as_slice()).unwrap();
        // A full page load on a non-root route must carry the same titlebar
        // custom properties as the startup document, or a custom titlebar
        // collapses as soon as the user navigates.
        assert!(html.contains("--webui-titlebar-inset-start"));
    }

    #[test]
    fn serves_static_asset_with_traversal_protection() {
        let dir = TempDir::new().unwrap();
        write_file(dir.path(), "index.html", "<main>Hello</main>");
        write_file(dir.path(), "assets/app.js", "console.log('ok');");
        write_file(dir.path(), "assets/config.json", r#"{"ok":true}"#);

        let mut config = DesktopSourceConfig::new(build_options(dir.path().to_path_buf()));
        config.asset_root = Some(dir.path().join("assets"));
        let runtime = DesktopRuntime::from_source(config).unwrap();

        let response = runtime
            .handle_request(&DesktopProtocolRequest::get("/app.js"))
            .unwrap();
        assert_eq!(response.status, 200);
        assert_eq!(response.content_type, "text/javascript");

        let json_asset = runtime
            .handle_request(&DesktopProtocolRequest {
                method: DesktopHttpMethod::Get,
                path: "/config.json",
                body: &[],
                wants_json: true,
            })
            .unwrap();
        assert_eq!(json_asset.status, 200);
        assert_eq!(json_asset.content_type, "application/json");
        assert_eq!(json_asset.body.into_bytes().unwrap(), br#"{"ok":true}"#);

        let err = runtime
            .handle_request(&DesktopProtocolRequest::get("/%2e%2e/index.html"))
            .unwrap_err();
        assert!(matches!(err, DesktopError::InvalidAssetPath { .. }));
    }

    #[test]
    #[cfg(feature = "application-ipc")]
    fn dispatches_ipc_through_the_owning_frame() {
        let dir = TempDir::new().unwrap();
        write_file(dir.path(), "index.html", "<main>Hello</main>");

        let mut config = DesktopSourceConfig::new(build_options(dir.path().to_path_buf()));
        config.ipc_registry = crate::ipc_test_support::registry();
        let runtime = DesktopRuntime::from_source(config).unwrap();
        assert!(matches!(
            runtime.handle_request(&DesktopProtocolRequest::post(IPC_ENDPOINT, b"unadmitted")),
            Err(DesktopError::UnsupportedRuntime { .. })
        ));
        let frame = crate::DesktopFrame::with_ipc_options(
            Arc::new(runtime),
            crate::WindowOptions::default(),
            crate::ipc_test_support::options(),
        )
        .unwrap();
        crate::ipc_test_support::assert_echo(&frame, b"ping");
    }

    #[test]
    fn dispatches_custom_api_route_before_assets_and_router() {
        let dir = TempDir::new().unwrap();
        write_file(dir.path(), "index.html", "<main>Hello</main>");

        let mut config = DesktopSourceConfig::new(build_options(dir.path().to_path_buf()));
        config
            .api_routes
            .route("/api/contacts/:id", |ctx| {
                assert_eq!(ctx.path, "/api/contacts/42");
                assert_eq!(ctx.param("id"), Some("42"));
                assert_eq!(ctx.method, &DesktopHttpMethod::Post);
                assert_eq!(ctx.body, br#"{"ok":true}"#);
                Ok(DesktopProtocolResponse::new(
                    201,
                    "application/json",
                    br#"{"id":"42"}"#.to_vec(),
                ))
            })
            .unwrap();
        let runtime = DesktopRuntime::from_source(config).unwrap();

        let response = runtime
            .handle_request(&DesktopProtocolRequest {
                method: DesktopHttpMethod::Post,
                path: "/api/contacts/42",
                body: br#"{"ok":true}"#,
                wants_json: true,
            })
            .unwrap();

        assert_eq!(response.status, 201);
        assert_eq!(response.content_type, "application/json");
        assert_eq!(response.body, br#"{"id":"42"}"#);
    }

    #[test]
    fn router_json_request_returns_partial_with_state() {
        let dir = TempDir::new().unwrap();
        write_file(
            dir.path(),
            "index.html",
            "<route path=\"/\" component=\"my-page\"><route path=\"contacts/:id\" component=\"my-page\" exact /></route>",
        );
        write_file(dir.path(), "my-page.html", "<p>{{title}}</p>");
        write_file(dir.path(), "state.json", r#"{"title":"Contact"}"#);

        let mut config = DesktopSourceConfig::new(build_options(dir.path().to_path_buf()));
        config.state_file = Some(dir.path().join("state.json"));
        let runtime = DesktopRuntime::from_source(config).unwrap();

        let response = runtime
            .handle_request(&DesktopProtocolRequest {
                method: DesktopHttpMethod::Get,
                path: "/contacts/42",
                body: &[],
                wants_json: true,
            })
            .unwrap();

        assert_eq!(response.status, 200);
        assert_eq!(response.content_type, "application/json");
        let json: Value = serde_json::from_slice(response.body.as_bytes().unwrap()).unwrap();
        assert_eq!(json["path"], "/contacts/42");
        assert_eq!(json["state"]["title"], "Contact");
    }

    #[test]
    fn bundle_runtime_preserves_rust_route_providers() {
        let dir = TempDir::new().unwrap();
        let app = dir.path().join("app");
        let bundle = dir.path().join("bundle");
        write_file(
            &app,
            "index.html",
            "<route path=\"/\" component=\"my-page\"><route path=\"users/:id\" component=\"my-page\" exact /></route>",
        );
        write_file(&app, "my-page.html", "<p>{{name}}</p>");

        crate::build_desktop_bundle(crate::DesktopBundleOptions {
            build_options: build_options(app),
            out_dir: bundle.clone(),
            state_file: None,
            asset_root: None,
            token_css: None,
            app_id: "com.microsoft.webui.bundle".to_string(),
            app_name: "Bundle Host".to_string(),
            version: "0.0.0".to_string(),
            publisher: "Microsoft".to_string(),
            window: crate::WindowOptions::default(),
            icon_file: None,
            shell: crate::DesktopShellConfig::default(),
            package_targets: Vec::new(),
        })
        .unwrap();

        let mut config = DesktopBundleConfig::new(bundle);
        config
            .route_state
            .route("/users/:id", |ctx| {
                let mut map = serde_json::Map::new();
                map.insert(
                    "name".to_string(),
                    Value::String(ctx.param("id").unwrap_or_default().to_string()),
                );
                Ok(Value::Object(map))
            })
            .unwrap();
        let runtime = DesktopRuntime::from_bundle_config(config).unwrap();

        let state = partial_state(&runtime, "/users/ada");
        assert_eq!(state["name"], "ada");
    }

    #[test]
    fn bundle_runtime_serves_manifest_indexed_assets() {
        let dir = TempDir::new().unwrap();
        let app = dir.path().join("app");
        let bundle = dir.path().join("bundle");
        write_file(&app, "index.html", "<main>Hello</main>");
        write_file(&app, "public/app.js", "console.log('bundle');");

        crate::build_desktop_bundle(crate::DesktopBundleOptions {
            build_options: build_options(app.clone()),
            out_dir: bundle.clone(),
            state_file: None,
            asset_root: Some(app.join("public")),
            token_css: None,
            app_id: "com.microsoft.webui.bundle".to_string(),
            app_name: "Bundle Host".to_string(),
            version: "0.0.0".to_string(),
            publisher: "Microsoft".to_string(),
            window: crate::WindowOptions::default(),
            icon_file: None,
            shell: crate::DesktopShellConfig::default(),
            package_targets: Vec::new(),
        })
        .unwrap();

        let runtime = DesktopRuntime::from_bundle(bundle).unwrap();
        let response = runtime
            .handle_request(&DesktopProtocolRequest::get("/app.js?v=1"))
            .unwrap();

        assert_eq!(response.status, 200);
        assert_eq!(response.content_type, "text/javascript");
        assert_eq!(
            response.body.into_bytes().unwrap(),
            b"console.log('bundle');"
        );

        let err = runtime
            .handle_request(&DesktopProtocolRequest::get("/%2e%2e/protocol.bin"))
            .unwrap_err();
        assert!(matches!(err, DesktopError::InvalidAssetPath { .. }));
    }

    #[test]
    fn bundle_runtime_rejects_manifest_paths_outside_bundle() {
        let dir = TempDir::new().unwrap();
        let app = dir.path().join("app");
        let bundle = dir.path().join("bundle");
        write_file(&app, "index.html", "<main>Hello</main>");

        crate::build_desktop_bundle(crate::DesktopBundleOptions {
            build_options: build_options(app),
            out_dir: bundle.clone(),
            state_file: None,
            asset_root: None,
            token_css: None,
            app_id: "com.microsoft.webui.bundle".to_string(),
            app_name: "Bundle Host".to_string(),
            version: "0.0.0".to_string(),
            publisher: "Microsoft".to_string(),
            window: crate::WindowOptions::default(),
            icon_file: None,
            shell: crate::DesktopShellConfig::default(),
            package_targets: Vec::new(),
        })
        .unwrap();

        let manifest_path = bundle.join("manifest.webui-desktop.json");
        let mut manifest: Value =
            serde_json::from_str(&fs::read_to_string(&manifest_path).unwrap()).unwrap();
        manifest["protocol_path"] = Value::String("../protocol.bin".to_string());
        fs::write(
            &manifest_path,
            serde_json::to_vec_pretty(&manifest).unwrap(),
        )
        .unwrap();

        let err = match DesktopRuntime::from_bundle(bundle) {
            Ok(_) => panic!("expected manifest path validation error"),
            Err(err) => err,
        };
        assert!(matches!(err, DesktopError::InvalidAssetPath { .. }));
    }

    #[test]
    fn file_backed_state_is_not_route_scoped_without_provider() {
        let dir = TempDir::new().unwrap();
        write_file(
            dir.path(),
            "index.html",
            "<route path=\"/\" component=\"my-page\"><route path=\"contacts/:id\" component=\"my-page\" exact /><route path=\"favorites\" component=\"my-page\" exact /><route path=\"groups/:group\" component=\"my-page\" exact /></route>",
        );
        write_file(dir.path(), "my-page.html", "<p>{{page}}</p>");
        write_file(
            dir.path(),
            "state.json",
            r##"{
              "groups":["Work","Friends"],
              "contacts":[
                {"id":"1","firstName":"Ada","lastName":"Lovelace","group":"Work","favorite":true,"initials":"AL","avatarColor":"#fff","email":"ada@example.com","phone":"1","company":"","notes":"","address":""},
                {"id":"2","firstName":"Grace","lastName":"Hopper","group":"Friends","favorite":false,"initials":"GH","avatarColor":"#000","email":"grace@example.com","phone":"2","company":"","notes":"","address":""}
              ]
            }"##,
        );

        let mut config = DesktopSourceConfig::new(build_options(dir.path().to_path_buf()));
        config.state_file = Some(dir.path().join("state.json"));
        let runtime = DesktopRuntime::from_source(config).unwrap();

        let favorites = runtime.state_for_request("/favorites").unwrap();
        assert!(favorites.get("page").is_none());
        assert_eq!(favorites["contacts"].as_array().map(Vec::len), Some(2));
        assert_eq!(partial_state(&runtime, "/favorites"), serde_json::json!({}));
    }

    #[test]
    fn rust_route_provider_overrides_file_backed_state() {
        let dir = TempDir::new().unwrap();
        write_file(
            dir.path(),
            "index.html",
            "<route path=\"/\" component=\"my-page\"><route path=\"contacts/:id\" component=\"my-page\" exact /></route>",
        );
        write_file(
            dir.path(),
            "my-page.html",
            "<p>{{name}} {{id}}</p><a href=\"{{basePath}}\">Home</a>",
        );
        write_file(dir.path(), "state.json", r#"{"name":"base"}"#);

        let mut config = DesktopSourceConfig::new(build_options(dir.path().to_path_buf()));
        config.state_file = Some(dir.path().join("state.json"));
        config
            .route_state
            .route("/contacts/:id", |ctx| {
                Ok(serde_json::json!({
                    "name": "provider",
                    "id": ctx.param("id").unwrap_or("")
                }))
            })
            .unwrap();
        let runtime = DesktopRuntime::from_source(config).unwrap();

        let state = partial_state(&runtime, "/contacts/42");
        assert_eq!(state["name"], "provider");
        assert_eq!(state["id"], "42");
        assert_eq!(state["basePath"], "/");
    }

    #[test]
    fn route_provider_state_preserves_seed_tokens() {
        let dir = TempDir::new().unwrap();
        write_file(
            dir.path(),
            "index.html",
            "<html><head><style>:root{/*{{{tokens.light}}}*/}</style></head><body>\
             <route path=\"/\" component=\"my-page\"><route path=\"contacts\" component=\"my-page\" exact /></route>\
             </body></html>",
        );
        write_file(dir.path(), "my-page.html", "<p>{{page}}</p>");
        write_file(
            dir.path(),
            "state.json",
            r#"{"tokens":{"light":"--font-family-base: system-ui;"}}"#,
        );

        let mut config = DesktopSourceConfig::new(build_options(dir.path().to_path_buf()));
        config.state_file = Some(dir.path().join("state.json"));
        config
            .route_state
            .route("/contacts", |_| {
                let mut map = serde_json::Map::new();
                map.insert("page".to_string(), Value::String("contacts".to_string()));
                Ok(Value::Object(map))
            })
            .unwrap();
        let runtime = DesktopRuntime::from_source(config).unwrap();

        let state = runtime.state_for_request("/contacts").unwrap();

        assert_eq!(
            state["tokens"]["light"],
            Value::String("--font-family-base: system-ui;".to_string())
        );
        assert!(partial_state(&runtime, "/contacts").get("tokens").is_none());
        let response = runtime
            .handle_request(&DesktopProtocolRequest::get("/contacts"))
            .unwrap();
        let html = std::str::from_utf8(response.body.as_bytes().unwrap()).unwrap();
        assert!(html.contains(":root{--font-family-base: system-ui;}"));
    }

    #[test]
    fn source_and_bundle_partials_match_web_projection() {
        let dir = TempDir::new().unwrap();
        let app = dir.path().join("app");
        write_file(
            &app,
            "index.html",
            "<route path=\"/\" component=\"app-shell\">\
             <route path=\"contacts/:id\" component=\"contact-page\" exact />\
             <route path=\"reports\" component=\"report-page\" exact />\
             </route>",
        );
        write_file(
            &app,
            "app-shell.html",
            "<header>{{mode}}</header><outlet />",
        );
        write_file(
            &app,
            "contact-page.html",
            "<p>{{name}} {{id}}</p><if condition=\"expanded\"><p>{{details}}</p></if>",
        );
        write_file(&app, "report-page.html", "<p>{{unusedReports.length}}</p>");
        let state_file = dir.path().join("state.json");
        let seed = serde_json::json!({
            "mode": "desktop", "name": "Ada", "expanded": false, "details": "Ready on demand",
            "unusedReports": ["not needed by this route"],
            "$webui": {"bodyEnd": "<script>host-only</script>"}
        });
        fs::write(&state_file, serde_json::to_vec(&seed).unwrap()).unwrap();

        for bundled in [false, true] {
            let runtime = if bundled {
                let bundle = dir.path().join("bundle");
                crate::build_desktop_bundle(crate::DesktopBundleOptions {
                    build_options: build_options(app.clone()),
                    out_dir: bundle.clone(),
                    state_file: Some(state_file.clone()),
                    asset_root: None,
                    token_css: None,
                    app_id: "webui.test.projection".into(),
                    app_name: "Projection".into(),
                    version: "0.0.0".into(),
                    publisher: "Microsoft".into(),
                    window: crate::WindowOptions::default(),
                    icon_file: None,
                    shell: crate::DesktopShellConfig::default(),
                    package_targets: Vec::new(),
                })
                .unwrap();
                DesktopRuntime::from_bundle(bundle).unwrap()
            } else {
                let mut config = DesktopSourceConfig::new(build_options(app.clone()));
                config.state_file = Some(state_file.clone());
                DesktopRuntime::from_source(config).unwrap()
            };
            let path = "/contacts/42";
            let response = runtime
                .handle_request(&DesktopProtocolRequest {
                    method: DesktopHttpMethod::Get,
                    path,
                    body: &[],
                    wants_json: true,
                })
                .unwrap();
            assert_eq!(response.status, 200);
            let bytes = response.body.as_bytes().unwrap();
            let web = runtime
                .protocol
                .render_partial_json(
                    &runtime.state_for_request(path).unwrap().to_string(),
                    &runtime.entry,
                    path,
                    "",
                )
                .unwrap();
            assert_eq!(bytes, web.as_bytes());
            let wire: Value = serde_json::from_slice(bytes).unwrap();
            assert_eq!(
                wire["state"],
                serde_json::json!({
                    "mode": "desktop", "name": "Ada", "id": "42",
                    "expanded": false, "details": "Ready on demand"
                })
            );
            assert_eq!(wire["chain"][1]["params"]["id"], "42");
            assert!(wire.get("matched").is_none());
        }
    }

    #[test]
    fn missing_script_metadata_keeps_dynamic_fields_but_not_host_injection() {
        let dir = TempDir::new().unwrap();
        write_file(
            dir.path(),
            "index.html",
            "<route path=\"/\" component=\"my-page\" />",
        );
        write_file(dir.path(), "my-page.html", "<p>{{name}}</p>");
        write_file(dir.path(), "my-page.ts", "export {};");
        let mut config = DesktopSourceConfig::new(build_options(dir.path().to_path_buf()));
        config.state = Some(serde_json::json!({
            "name": "Ada", "dynamicField": [1, 2],
            "$webui": {"bodyEnd": "<script>host-only</script>"}
        }));
        let runtime = DesktopRuntime::from_source(config).unwrap();
        let state = partial_state(&runtime, "/");
        assert_eq!(state["name"], "Ada");
        assert_eq!(state["dynamicField"], serde_json::json!([1, 2]));
        assert!(state.get("$webui").is_none());
    }

    #[test]
    fn route_provider_errors_are_returned() {
        let dir = TempDir::new().unwrap();
        write_file(
            dir.path(),
            "index.html",
            "<route path=\"/\" component=\"my-page\"><route path=\"contacts/:id\" component=\"my-page\" exact /></route>",
        );
        write_file(dir.path(), "my-page.html", "<p>{{name}}</p>");

        let mut config = DesktopSourceConfig::new(build_options(dir.path().to_path_buf()));
        config
            .route_state
            .route("/contacts/:id", |_| {
                Err(DesktopError::UnsupportedRuntime {
                    message: "state store unavailable".to_string(),
                    help: "initialize the state store before running the desktop app".to_string(),
                })
            })
            .unwrap();
        let runtime = DesktopRuntime::from_source(config).unwrap();

        let err = runtime
            .handle_request(&DesktopProtocolRequest {
                method: DesktopHttpMethod::Get,
                path: "/contacts/42",
                body: &[],
                wants_json: true,
            })
            .unwrap_err();

        assert!(matches!(err, DesktopError::RouteProvider { .. }));
    }

    #[test]
    fn dotted_route_segments_are_not_treated_as_assets() {
        let dir = TempDir::new().unwrap();
        write_file(
            dir.path(),
            "index.html",
            "<route path=\"/\" component=\"my-page\"><route path=\"users/:id\" component=\"my-page\" exact /></route>",
        );
        write_file(dir.path(), "my-page.html", "<p>{{id}}</p>");

        let mut config = DesktopSourceConfig::new(build_options(dir.path().to_path_buf()));
        config
            .route_state
            .route("/users/:id", |ctx| {
                let mut map = serde_json::Map::new();
                map.insert(
                    "id".to_string(),
                    Value::String(ctx.param("id").unwrap_or_default().to_string()),
                );
                Ok(Value::Object(map))
            })
            .unwrap();
        let runtime = DesktopRuntime::from_source(config).unwrap();

        let html = runtime
            .handle_request(&DesktopProtocolRequest::get("/users/jane.doe"))
            .unwrap();
        assert_eq!(html.status, 200);
        assert_eq!(html.content_type, "text/html; charset=utf-8");

        let missing = runtime
            .handle_request(&DesktopProtocolRequest::get("/missing.js"))
            .unwrap();
        assert_eq!(missing.status, 404);
    }

    #[test]
    fn query_strings_do_not_break_route_matching() {
        let dir = TempDir::new().unwrap();
        write_file(
            dir.path(),
            "index.html",
            "<route path=\"/\" component=\"my-page\"><route path=\"favorites\" component=\"my-page\" exact /></route>",
        );
        write_file(dir.path(), "my-page.html", "<p>{{page}}</p>");

        let mut config = DesktopSourceConfig::new(build_options(dir.path().to_path_buf()));
        config
            .route_state
            .route("/favorites", |_| {
                let mut map = serde_json::Map::new();
                map.insert("page".to_string(), Value::String("favorites".to_string()));
                Ok(Value::Object(map))
            })
            .unwrap();
        let runtime = DesktopRuntime::from_source(config).unwrap();

        let response = runtime
            .handle_request(&DesktopProtocolRequest {
                method: DesktopHttpMethod::Get,
                path: "/favorites?sort=asc",
                body: &[],
                wants_json: true,
            })
            .unwrap();
        assert_eq!(response.status, 200);
        let json: Value = serde_json::from_slice(response.body.as_bytes().unwrap()).unwrap();
        assert_eq!(json["path"], "/favorites");
        assert_eq!(json["state"]["page"], "favorites");
    }

    #[test]
    fn query_strings_do_not_break_html_route_rendering() {
        let dir = TempDir::new().unwrap();
        write_file(
            dir.path(),
            "index.html",
            "<route path=\"/\" component=\"my-page\"><route path=\"favorites\" component=\"my-page\" exact /></route>",
        );
        write_file(dir.path(), "my-page.html", "<p>{{page}}</p>");

        let mut config = DesktopSourceConfig::new(build_options(dir.path().to_path_buf()));
        config
            .route_state
            .route("/favorites", |_| {
                let mut map = serde_json::Map::new();
                map.insert("page".to_string(), Value::String("favorites".to_string()));
                Ok(Value::Object(map))
            })
            .unwrap();
        let runtime = DesktopRuntime::from_source(config).unwrap();

        let response = runtime
            .handle_request(&DesktopProtocolRequest::get("/favorites?sort=asc"))
            .unwrap();

        assert_eq!(response.status, 200);
        let html = std::str::from_utf8(response.body.as_bytes().unwrap().as_slice()).unwrap();
        assert!(html.contains("<p>favorites</p>"));
    }

    fn partial_state(runtime: &DesktopRuntime, path: &str) -> Value {
        let response = runtime
            .handle_request(&DesktopProtocolRequest {
                method: DesktopHttpMethod::Get,
                path,
                body: &[],
                wants_json: true,
            })
            .unwrap();
        let json: Value = serde_json::from_slice(response.body.as_bytes().unwrap()).unwrap();
        json["state"].clone()
    }
}

// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

use std::collections::HashMap;
use std::fs;
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;

use percent_encoding::percent_decode_str;
use serde_json::Value;
use webui::RenderOptions;
use webui_handler::plugin::fast_v2::FastV2HydrationPlugin;
use webui_handler::plugin::fast_v3::FastV3HydrationPlugin;
use webui_handler::plugin::webui::WebUIHydrationPlugin;
use webui_handler::route_matcher::CompiledRouteCache;
use webui_handler::ResponseWriter;
use webui_protocol::WebUIProtocol;

use crate::error::{DesktopError, Result};
use crate::ipc::IpcRegistry;
use crate::path::resolve_safe_path;
use crate::protocol::{
    read_asset_response, read_known_asset_response, DesktopHttpMethod, DesktopProtocolRequest,
    DesktopProtocolResponse, DEFAULT_MAX_ASSET_BYTES, IPC_ENDPOINT,
};

/// Source-backed desktop runtime configuration.
pub struct DesktopSourceConfig {
    /// WebUI build options.
    pub build_options: webui::BuildOptions,
    /// Optional state JSON file.
    pub state_file: Option<PathBuf>,
    /// Optional in-memory startup state supplied by Rust app hosts.
    pub state: Option<Value>,
    /// Optional static asset root.
    pub asset_root: Option<PathBuf>,
    /// Maximum asset bytes read into one protocol response.
    pub max_asset_bytes: u64,
    /// Protobuf IPC registry.
    pub ipc_registry: IpcRegistry,
    /// Rust route state providers.
    pub route_state: RouteStateRegistry,
    /// Rust custom-protocol API handlers.
    pub api_routes: ApiRouteRegistry,
    /// Optional pre-resolved design token CSS keyed by theme name.
    pub token_css: Option<HashMap<String, String>>,
    /// Optional theme value and search root to resolve after protocol build.
    pub theme: Option<(String, PathBuf)>,
}

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
            ipc_registry: IpcRegistry::new(),
            route_state: RouteStateRegistry::new(),
            api_routes: ApiRouteRegistry::new(),
            token_css: None,
            theme: None,
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
    /// Maximum asset bytes read into one protocol response.
    pub max_asset_bytes: u64,
    /// Protobuf IPC registry.
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
            ipc_registry: IpcRegistry::new(),
            route_state: RouteStateRegistry::new(),
            api_routes: ApiRouteRegistry::new(),
            token_css: None,
        }
    }
}

/// Runtime state shared by a desktop webview custom-protocol handler.
pub struct DesktopRuntime {
    protocol: WebUIProtocol,
    entry: String,
    state: Value,
    css_files: HashMap<String, String>,
    asset_root: Option<PathBuf>,
    asset_index: HashMap<String, DesktopAssetEntry>,
    max_asset_bytes: u64,
    startup_html: String,
    ipc_registry: IpcRegistry,
    plugin: Option<webui::Plugin>,
    route_state: RouteStateRegistry,
    api_routes: ApiRouteRegistry,
    token_css: Option<HashMap<String, String>>,
}

type RouteStateHandler = dyn Fn(RouteContext<'_>) -> Result<Value> + Send + Sync;
type ApiHandler = dyn Fn(ApiContext<'_>) -> Result<DesktopProtocolResponse> + Send + Sync;

/// Registry of Rust route state providers.
#[derive(Default)]
pub struct RouteStateRegistry {
    routes: Vec<RouteStateEntry>,
}

/// Registry of Rust custom-protocol API handlers.
#[derive(Default)]
pub struct ApiRouteRegistry {
    routes: Vec<ApiRouteEntry>,
}

impl ApiRouteRegistry {
    /// Create an empty API route registry.
    #[must_use]
    pub fn new() -> Self {
        Self { routes: Vec::new() }
    }

    /// Register an API handler for a URL path pattern.
    ///
    /// # Errors
    ///
    /// Returns [`DesktopError`] if `pattern` is invalid.
    pub fn route<F>(&mut self, pattern: impl AsRef<str>, handler: F) -> Result<()>
    where
        F: Fn(ApiContext<'_>) -> Result<DesktopProtocolResponse> + Send + Sync + 'static,
    {
        self.routes.push(ApiRouteEntry {
            pattern: RoutePattern::parse(pattern.as_ref())?,
            handler: Arc::new(handler),
        });
        Ok(())
    }

    fn resolve(
        &self,
        request: &DesktopProtocolRequest<'_>,
    ) -> Result<Option<DesktopProtocolResponse>> {
        let path = route_path(request.path);
        for entry in self.routes.iter() {
            let Some(params) = entry.pattern.matches(path) else {
                continue;
            };
            let context = ApiContext {
                method: &request.method,
                path,
                params: &params,
                body: request.body,
            };
            return (entry.handler)(context).map(Some);
        }
        Ok(None)
    }
}

#[derive(Clone)]
struct ApiRouteEntry {
    pattern: RoutePattern,
    handler: Arc<ApiHandler>,
}

struct DesktopAssetEntry {
    path: PathBuf,
    content_type: String,
    size_bytes: u64,
}

impl RouteStateRegistry {
    /// Create an empty route state registry.
    #[must_use]
    pub fn new() -> Self {
        Self { routes: Vec::new() }
    }

    /// Register a route state provider.
    ///
    /// # Errors
    ///
    /// Returns [`DesktopError`] if `pattern` is invalid.
    pub fn route<F>(&mut self, pattern: impl AsRef<str>, handler: F) -> Result<()>
    where
        F: Fn(RouteContext<'_>) -> Result<Value> + Send + Sync + 'static,
    {
        self.routes.push(RouteStateEntry {
            pattern: RoutePattern::parse(pattern.as_ref())?,
            handler: Arc::new(handler),
        });
        Ok(())
    }

    fn resolve(&self, path: &str, base_state: &Value) -> Result<Option<Value>> {
        for entry in self.routes.iter() {
            let Some(params) = entry.pattern.matches(path) else {
                continue;
            };
            let context = RouteContext {
                path,
                params: &params,
                base_state,
            };
            return (entry.handler)(context)
                .map(Some)
                .map_err(|err| DesktopError::RouteProvider {
                    path: path.to_string(),
                    message: err.chain_message(),
                });
        }
        Ok(None)
    }
}

#[derive(Clone)]
struct RouteStateEntry {
    pattern: RoutePattern,
    handler: Arc<RouteStateHandler>,
}

#[derive(Clone)]
struct RoutePattern {
    segments: Vec<RouteSegment>,
}

#[derive(Clone)]
enum RouteSegment {
    Literal(String),
    Param(String),
}

/// Context passed to Rust route state providers.
pub struct RouteContext<'a> {
    /// Request path without query string.
    pub path: &'a str,
    params: &'a [(String, String)],
    /// File-backed base state loaded by the desktop runtime.
    pub base_state: &'a Value,
}

/// Context passed to Rust custom-protocol API handlers.
pub struct ApiContext<'a> {
    /// Request method.
    pub method: &'a DesktopHttpMethod,
    /// Request path without query string.
    pub path: &'a str,
    params: &'a [(String, String)],
    /// Request body bytes.
    pub body: &'a [u8],
}

impl<'a> ApiContext<'a> {
    /// Return a route parameter by name.
    #[must_use]
    pub fn param(&self, name: &str) -> Option<&str> {
        self.params
            .iter()
            .find_map(|(key, value)| (key == name).then_some(value.as_str()))
    }
}

impl<'a> RouteContext<'a> {
    /// Return a route parameter by name.
    #[must_use]
    pub fn param(&self, name: &str) -> Option<&str> {
        self.params
            .iter()
            .find_map(|(key, value)| (key == name).then_some(value.as_str()))
    }
}

impl RoutePattern {
    fn parse(pattern: &str) -> Result<Self> {
        if !pattern.starts_with('/') {
            return Err(DesktopError::InvalidRoutePattern {
                pattern: pattern.to_string(),
                help: "desktop route patterns must start with '/', e.g. /contacts/:id".to_string(),
            });
        }

        let trimmed = pattern.trim_matches('/');
        let mut segments = Vec::new();
        if !trimmed.is_empty() {
            for segment in trimmed.split('/') {
                if segment.is_empty() || segment == "." || segment == ".." {
                    return Err(DesktopError::InvalidRoutePattern {
                        pattern: pattern.to_string(),
                        help: "route pattern segments cannot be empty, '.', or '..'".to_string(),
                    });
                }
                if let Some(param) = segment.strip_prefix(':') {
                    if param.is_empty() {
                        return Err(DesktopError::InvalidRoutePattern {
                            pattern: pattern.to_string(),
                            help: "route parameter names cannot be empty".to_string(),
                        });
                    }
                    segments.push(RouteSegment::Param(param.to_string()));
                } else {
                    segments.push(RouteSegment::Literal(segment.to_string()));
                }
            }
        }
        Ok(Self { segments })
    }

    fn matches(&self, path: &str) -> Option<Vec<(String, String)>> {
        let path = path.split_once('?').map_or(path, |(path, _)| path);
        let trimmed = path.trim_matches('/');
        if trimmed.is_empty() {
            return self.segments.is_empty().then(Vec::new);
        }
        if self.segments.is_empty() {
            None
        } else {
            self.matches_non_empty(trimmed)
        }
    }

    fn matches_non_empty(&self, path: &str) -> Option<Vec<(String, String)>> {
        let segment_count = path.split('/').count();
        if segment_count != self.segments.len() {
            return None;
        }
        let mut params = Vec::new();
        for (pattern, raw_segment) in self.segments.iter().zip(path.split('/')) {
            let segment = decode_segment(raw_segment);
            match pattern {
                RouteSegment::Literal(expected) if expected == &segment => {}
                RouteSegment::Literal(_) => return None,
                RouteSegment::Param(name) => params.push((name.clone(), segment)),
            }
        }
        Some(params)
    }
}

impl DesktopRuntime {
    /// Build and render a desktop runtime from source paths.
    ///
    /// # Errors
    ///
    /// Returns [`DesktopError`] if the WebUI build fails, state cannot be read
    /// or parsed, assets cannot be canonicalized, or startup rendering fails.
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
        let startup_state = state_for_request(StateRequestContext {
            protocol: &build_result.protocol,
            entry: &config.build_options.entry,
            base_state: &state,
            registry: &config.route_state,
            token_css: token_css.as_ref(),
            request_path: "/",
        })?;
        let startup_html = render_html(
            &build_result.protocol,
            config.build_options.plugin,
            &config.build_options.entry,
            "/",
            &startup_state,
        )?;

        Ok(Self {
            protocol: build_result.protocol,
            entry: config.build_options.entry,
            state,
            css_files,
            asset_root,
            asset_index: HashMap::new(),
            max_asset_bytes: config.max_asset_bytes,
            startup_html,
            ipc_registry: config.ipc_registry,
            plugin: config.build_options.plugin,
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
        let protocol = WebUIProtocol::from_protobuf(&protocol_bytes)?;
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
        let plugin = parse_plugin(manifest.plugin.as_deref());
        let startup_state = state_for_request(StateRequestContext {
            protocol: &protocol,
            entry: &manifest.entry,
            base_state: &state,
            registry: &config.route_state,
            token_css: config.token_css.as_ref(),
            request_path: "/",
        })?;
        let startup_html = render_html(&protocol, plugin, &manifest.entry, "/", &startup_state)?;

        Ok(Self {
            protocol,
            entry: manifest.entry,
            state,
            css_files: HashMap::new(),
            asset_root: Some(asset_root),
            asset_index,
            max_asset_bytes: config.max_asset_bytes,
            startup_html,
            ipc_registry: config.ipc_registry,
            plugin,
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
        if request.path == IPC_ENDPOINT {
            return self.handle_ipc_request(request);
        }

        if let Some(response) = self.api_routes.resolve(request)? {
            return Ok(response);
        }

        if matches!(request.method, DesktopHttpMethod::Get) {
            let request_path = route_path(request.path);
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
                return Ok(DesktopProtocolResponse::html(
                    self.startup_html.as_bytes().to_vec(),
                ));
            }

            if self.has_route_match(request_path) {
                let html = render_html(
                    &self.protocol,
                    self.plugin,
                    &self.entry,
                    request_path,
                    &self.state_for_request(request_path)?,
                )?;
                return Ok(DesktopProtocolResponse::html(html.into_bytes()));
            }
        }

        Ok(DesktopProtocolResponse::text(404, "Not Found"))
    }

    /// Return the startup HTML rendered for `/`.
    #[must_use]
    pub fn startup_html(&self) -> &str {
        &self.startup_html
    }

    fn handle_ipc_request(
        &self,
        request: &DesktopProtocolRequest<'_>,
    ) -> Result<DesktopProtocolResponse> {
        if request.method != DesktopHttpMethod::Post {
            return Ok(DesktopProtocolResponse::text(
                405,
                "desktop IPC endpoint requires POST",
            ));
        }
        self.ipc_registry
            .dispatch_frame(request.body)
            .map(DesktopProtocolResponse::protobuf)
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
            if resolve_safe_path(root, request_path).is_none() {
                return Err(DesktopError::InvalidAssetPath {
                    path: request_path.to_string(),
                });
            }
            let key = asset_index_key(request_path);
            return match self.asset_index.get(key.as_str()) {
                Some(asset) => read_known_asset_response(
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
        let mut index = webui_handler::route_handler::ProtocolIndex::new(&self.protocol);
        let partial = webui_handler::route_handler::render_partial(
            &self.protocol,
            &self.entry,
            route_path,
            "",
            &mut index,
        )?;
        let mut partial = partial;
        if !partial
            .get("chain")
            .and_then(Value::as_array)
            .is_some_and(|chain| json_route_chain_matches_request(chain, route_path))
        {
            return Ok(DesktopProtocolResponse::text(404, "Not Found"));
        }
        if let Some(obj) = partial.as_object_mut() {
            obj.insert("state".to_string(), self.state_for_request(route_path)?);
        }
        let bytes = serde_json::to_vec(&partial).map_err(|source| DesktopError::Serialization {
            context: "serializing desktop router partial".to_string(),
            source,
        })?;
        Ok(DesktopProtocolResponse::new(200, "application/json", bytes))
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

    fn has_route_match(&self, request_path: &str) -> bool {
        let route_path = route_path(request_path);
        let mut cache = CompiledRouteCache::new();
        let chain = webui_handler::route_handler::collect_route_chain(
            &self.protocol,
            &self.entry,
            route_path,
            &mut cache,
        );
        route_chain_matches_request(&chain, route_path)
    }
}

fn build_asset_index(
    asset_root: &Path,
    assets: &[crate::BundleAsset],
    max_asset_bytes: u64,
) -> Result<HashMap<String, DesktopAssetEntry>> {
    let mut index = HashMap::with_capacity(assets.len());
    for asset in assets {
        let Some(relative) = asset.path.strip_prefix("assets/") else {
            return Err(DesktopError::InvalidAssetPath {
                path: asset.path.clone(),
            });
        };
        let request_path = asset_index_key(relative);
        let Some(path) = resolve_safe_path(asset_root, &request_path) else {
            return Err(DesktopError::InvalidAssetPath {
                path: asset.path.clone(),
            });
        };
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
            request_path,
            DesktopAssetEntry {
                path,
                content_type,
                size_bytes: asset.size_bytes,
            },
        );
    }
    Ok(index)
}

fn asset_index_key(path: &str) -> String {
    let path = path.split_once('?').map_or(path, |(path, _)| path);
    if path.starts_with('/') {
        path.to_string()
    } else {
        let mut key = String::with_capacity(path.len() + 1);
        key.push('/');
        key.push_str(path);
        key
    }
}

fn route_path(request_path: &str) -> &str {
    request_path
        .split_once('?')
        .map_or(request_path, |(path, _)| path)
}

fn route_chain_matches_request(
    chain: &[webui_handler::route_handler::RouteChainEntry],
    request_path: &str,
) -> bool {
    let path = route_path(request_path);
    if path == "/" {
        return !chain.is_empty();
    }
    match chain {
        [] => false,
        [only] => only.path != "/",
        _ => true,
    }
}

fn json_route_chain_matches_request(chain: &[Value], request_path: &str) -> bool {
    let path = route_path(request_path);
    if path == "/" {
        return !chain.is_empty();
    }
    match chain {
        [] => false,
        [only] => only.get("path").and_then(Value::as_str) != Some("/"),
        _ => true,
    }
}

struct StateRequestContext<'a> {
    protocol: &'a WebUIProtocol,
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
        let mut cache = CompiledRouteCache::new();
        let params = webui_handler::route_handler::collect_nested_route_params(
            context.protocol,
            context.entry,
            path,
            &mut cache,
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

fn decode_segment(segment: &str) -> String {
    percent_decode_str(segment)
        .decode_utf8()
        .map(|value| value.into_owned())
        .unwrap_or_else(|_| segment.to_string())
}

fn parse_plugin(plugin: Option<&str>) -> Option<webui::Plugin> {
    match plugin {
        Some("fast") | Some("fast-v2") => Some(webui::Plugin::FastV2),
        Some("fast-v3") => Some(webui::Plugin::FastV3),
        Some("webui") => Some(webui::Plugin::WebUI),
        _ => None,
    }
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
    protocol: &WebUIProtocol,
    plugin: Option<webui::Plugin>,
    entry: &str,
    request_path: &str,
    state: &Value,
) -> Result<String> {
    let handler = create_handler(plugin);
    let mut writer = MemoryWriter::with_capacity(4096);
    handler.handle(
        protocol,
        state,
        &RenderOptions::new(entry, request_path),
        &mut writer,
    )?;
    Ok(writer.buf)
}

fn create_handler(plugin: Option<webui::Plugin>) -> webui::WebUIHandler {
    match plugin {
        Some(webui::Plugin::Fast | webui::Plugin::FastV2) => {
            webui::WebUIHandler::with_plugin(|| Box::new(FastV2HydrationPlugin::new()))
        }
        Some(webui::Plugin::FastV3) => {
            webui::WebUIHandler::with_plugin(|| Box::new(FastV3HydrationPlugin::new()))
        }
        Some(webui::Plugin::WebUI) => {
            webui::WebUIHandler::with_plugin(|| Box::new(WebUIHydrationPlugin::new()))
        }
        None => webui::WebUIHandler::new(),
    }
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

#[cfg(test)]
#[allow(clippy::disallowed_methods)]
mod tests {
    use super::*;
    use prost::Message;
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
        assert_eq!(json_asset.body, br#"{"ok":true}"#);

        let err = runtime
            .handle_request(&DesktopProtocolRequest::get("/%2e%2e/index.html"))
            .unwrap_err();
        assert!(matches!(err, DesktopError::InvalidAssetPath { .. }));
    }

    #[test]
    fn dispatches_ipc_endpoint() {
        let dir = TempDir::new().unwrap();
        write_file(dir.path(), "index.html", "<main>Hello</main>");

        let mut config = DesktopSourceConfig::new(build_options(dir.path().to_path_buf()));
        config
            .ipc_registry
            .register("echo", |payload| Ok(payload.to_vec()));
        let runtime = DesktopRuntime::from_source(config).unwrap();

        let request = crate::ipc::DesktopIpcRequest {
            version: crate::ipc::IPC_VERSION,
            request_id: 7,
            method: "echo".to_string(),
            payload: b"ping".to_vec(),
        };
        let mut body = Vec::new();
        request.encode(&mut body).unwrap();

        let response = runtime
            .handle_request(&DesktopProtocolRequest::post(IPC_ENDPOINT, &body))
            .unwrap();

        assert_eq!(response.status, 200);
        assert_eq!(response.content_type, "application/x-protobuf");
        let decoded = crate::ipc::DesktopIpcResponse::decode(response.body.as_slice()).unwrap();
        assert_eq!(decoded.request_id, 7);
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
        let json: Value = serde_json::from_slice(&response.body).unwrap();
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
        assert_eq!(response.body, b"console.log('bundle');");

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

        let favorites = partial_state(&runtime, "/favorites");
        assert!(favorites.get("page").is_none());
        assert_eq!(favorites["contacts"].as_array().map(Vec::len), Some(2));
    }

    #[test]
    fn rust_route_provider_overrides_file_backed_state() {
        let dir = TempDir::new().unwrap();
        write_file(
            dir.path(),
            "index.html",
            "<route path=\"/\" component=\"my-page\"><route path=\"contacts/:id\" component=\"my-page\" exact /></route>",
        );
        write_file(dir.path(), "my-page.html", "<p>{{name}}</p>");
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
            "<route path=\"/\" component=\"my-page\"><route path=\"contacts\" component=\"my-page\" exact /></route>",
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

        let state = partial_state(&runtime, "/contacts");

        assert_eq!(
            state["tokens"]["light"],
            Value::String("--font-family-base: system-ui;".to_string())
        );
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
        let json: Value = serde_json::from_slice(&response.body).unwrap();
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
        let html = String::from_utf8(response.body).unwrap();
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
        let json: Value = serde_json::from_slice(&response.body).unwrap();
        json["state"].clone()
    }
}

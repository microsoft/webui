// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

use actix_web::{web, App, HttpRequest, HttpResponse, HttpServer};
use anyhow::{Context, Result};
use clap::Args;
use expand_tilde::expand_tilde;
use mime_guess::from_path;
use serde_json::Value;
use std::collections::HashMap;
use std::fs;
use std::io::ErrorKind;
use std::net::{Ipv4Addr, SocketAddrV4, TcpListener};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio_stream::StreamExt;
use webui::streaming::StreamingWriter;
use webui::WebUIHandler;
use webui_dev_server::{spawn_watcher, sse_handler, LiveReload, WatchConfig};
use webui_handler::plugin::fast_v2::FastV2HydrationPlugin;
use webui_handler::plugin::fast_v3::FastV3HydrationPlugin;
use webui_handler::plugin::webui::WebUIHydrationPlugin;
use webui_handler::{RenderOptions, ResponseWriter};
use webui_protocol::WebUIProtocol;

use super::common::*;
use crate::utils::error::CliError;
use crate::utils::output;

#[derive(Args)]
pub struct ServeArgs {
    #[command(flatten)]
    pub app_args: AppArgs,

    /// Port to bind the development server to
    #[arg(long, default_value_t = 3000)]
    pub port: u16,

    /// Path to the JSON state file used for rendering
    #[arg(long)]
    pub state: Option<PathBuf>,

    /// Optional directory to serve static assets from at /*
    #[arg(long)]
    pub servedir: Option<PathBuf>,

    /// Enable file watching + HMR (disabled by default)
    #[arg(long)]
    pub watch: bool,

    /// Port of the user's API server to proxy route requests to
    #[arg(long)]
    pub api_port: Option<u16>,

    /// Design token theme: a path to a JSON file or an npm package name
    /// (e.g., `@microsoft/webui-examples-theme`). Resolved from node_modules
    /// when the value doesn't point to a file on disk.
    #[arg(long)]
    pub theme: Option<String>,

    /// Base path for sub-path deployment (e.g., `/commerce/`).
    /// Emits a `<base href>` tag and makes asset paths relative so the
    /// app can be served behind a reverse proxy under a sub-path.
    #[arg(long)]
    pub base_path: Option<String>,
}

/// Resolved paths for `webui serve`.
#[derive(Clone)]
struct ServePaths {
    app_dir: PathBuf,
    state_file: Option<PathBuf>,
    serve_dir: Option<PathBuf>,
}

impl ServePaths {
    fn from_args(args: &ServeArgs) -> Result<Self> {
        let app_input = expand_tilde(&args.app_args.app)
            .with_context(|| format!("Failed to expand app path: {}", args.app_args.app.display()))?
            .into_owned();

        let app_dir = app_input
            .canonicalize()
            .map_err(|_| CliError::AppFolderNotFound {
                path: args.app_args.app.display().to_string(),
            })?;

        let state_file = match &args.state {
            Some(state_path) => {
                let state_input = expand_tilde(state_path)
                    .with_context(|| {
                        format!("Failed to expand state path: {}", state_path.display())
                    })?
                    .into_owned();

                let canonical =
                    state_input
                        .canonicalize()
                        .map_err(|_| CliError::StateFileNotFound {
                            path: state_path.display().to_string(),
                        })?;

                if !canonical.is_file() {
                    return Err(anyhow::anyhow!(
                        "State path must be a file: {}",
                        canonical.display()
                    ));
                }
                Some(canonical)
            }
            None => None,
        };

        let serve_dir = match &args.servedir {
            Some(serve_arg) => {
                let serve_input = expand_tilde(serve_arg)
                    .with_context(|| {
                        format!(
                            "Failed to expand serve directory path: {}",
                            serve_arg.display()
                        )
                    })?
                    .into_owned();

                let canonical =
                    serve_input
                        .canonicalize()
                        .map_err(|_| CliError::ServeDirNotFound {
                            path: serve_arg.display().to_string(),
                        })?;

                if !canonical.is_dir() {
                    return Err(anyhow::anyhow!(
                        "Serve directory path must be a directory: {}",
                        canonical.display()
                    ));
                }

                Some(canonical)
            }
            None => None,
        };

        if !app_dir.is_dir() {
            return Err(anyhow::anyhow!(
                "App path must be a directory: {}",
                app_dir.display()
            ));
        }

        Ok(Self {
            app_dir,
            state_file,
            serve_dir,
        })
    }

    fn watch_paths(&self) -> Vec<PathBuf> {
        let mut paths = vec![self.app_dir.clone()];

        if let Some(state_file) = &self.state_file {
            paths.push(state_file.clone());
        }

        // Note: `serve_dir` (e.g. `./dist`) is intentionally NOT watched.
        // It is the destination for client bundles that other tools
        // (esbuild, pnpm, E2E harness) write to, and re-reading those
        // writes back through HMR causes spurious page reloads while
        // tests are running. The dev server only needs to react to
        // source changes under `app_dir` and to `state_file`.

        paths
    }
}

/// Thread-safe shared state: the rendered HTML for serving.
struct SharedState {
    rendered_html: String,
    css_files: HashMap<String, String>,
    protocol: Option<WebUIProtocol>,
    state_data: Option<Value>,
    /// Entry fragment ID used for rendering (e.g., "index.html").
    entry: String,
}

/// In-memory writer implementing `ResponseWriter` for the handler.
struct MemoryWriter {
    buf: String,
}

impl MemoryWriter {
    fn with_capacity(cap: usize) -> Self {
        Self {
            buf: String::with_capacity(cap),
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

/// SSE endpoint path. Root-relative so the script works under any
/// `<base href>` and across sub-path deployments.
const HMR_ENDPOINT: &str = "/__webui/livereload";

/// Environment variable that, when set to a non-empty / non-"0" value,
/// suppresses `--watch` mode at runtime. Used by `xtask e2e` so that
/// shared `start:server` package.json scripts (which include `--watch`
/// for dev) don't enable filesystem watching during E2E test runs,
/// where spurious rebuilds reload the page mid-test.
const WATCH_DISABLE_ENV: &str = "WEBUI_NO_WATCH";

fn watch_disabled_by_env() -> bool {
    match std::env::var(WATCH_DISABLE_ENV) {
        Ok(v) => !v.is_empty() && v != "0",
        Err(_) => false,
    }
}

pub fn execute(args: &ServeArgs) -> Result<()> {
    run(args).inspect_err(|err| {
        output::error(err);
        if let Some(cli_err) = err.chain().find_map(|c| c.downcast_ref::<CliError>()) {
            output::hint(cli_err.hint());
        }
        eprintln!();
    })
}

fn run(args: &ServeArgs) -> Result<()> {
    let paths = ServePaths::from_args(args)?;
    // Allow E2E / CI runs to suppress watch mode without editing the
    // package.json `start:server` script that devs share.
    let watch_enabled = args.watch && !watch_disabled_by_env();
    let livereload: Option<LiveReload> = if watch_enabled {
        Some(LiveReload::new(HMR_ENDPOINT))
    } else {
        None
    };

    // Load and resolve theme tokens once at startup
    let token_css: Option<HashMap<String, String>> = match &args.theme {
        Some(theme) => {
            let token_file = load_theme(theme)?;
            // We need the protocol to know which tokens are required.
            // Do a quick build to get protocol.tokens, then resolve.
            let probe_options = args.app_args.to_build_options(&paths.app_dir);
            let probe = webui::build(probe_options).with_context(|| "Build failed")?;
            let resolved = webui_tokens::resolve_tokens(&probe.protocol.tokens, &token_file)
                .with_context(|| "Token resolution failed")?;
            for warning in &resolved.warnings {
                eprintln!(
                    "  {} {}",
                    console::style("⚠").yellow(),
                    console::style(warning).dim()
                );
            }
            Some(resolved.css)
        }
        None => None,
    };

    let render_config = RenderConfig {
        app_args: args.app_args.clone(),
        app_dir: paths.app_dir.clone(),
        state_file: paths.state_file.clone(),
        token_css,
        base_path: args.base_path.clone(),
    };

    output::header("WebUI Dev Server");
    output::field("App", &paths.app_dir.display());
    match &paths.state_file {
        Some(f) => output::field("State", &f.display()),
        None => output::field("State", &"(none)"),
    }
    match &paths.serve_dir {
        Some(serve_dir) => output::field("ServeDir", &serve_dir.display()),
        None => output::field("ServeDir", &"(disabled)"),
    }
    if let Some(ref theme) = args.theme {
        output::field("Theme", theme);
    } else {
        output::field("Theme", &"(none)");
    }
    output::field("Entry", &args.app_args.entry);
    output::field("Port", &args.port);
    output::field("CSS", &args.app_args.css);
    if let Some(api_port) = args.api_port {
        output::field("API Port", &api_port);
    }
    if watch_enabled {
        output::field("HMR", &format!("enabled (SSE {HMR_ENDPOINT})"));
    } else if args.watch {
        output::field("HMR", &"disabled (WEBUI_NO_WATCH)");
    } else {
        output::field("HMR", &"disabled (pass --watch to enable)");
    }
    eprintln!();

    ensure_local_port_available(args.port)?;

    // Initial build + render (uses pre-resolved token_css)
    let initial_result = build_and_render(&render_config, livereload.as_ref())?;
    output::success("Initial build and render complete");

    let state = Arc::new(Mutex::new(SharedState {
        rendered_html: initial_result.html,
        css_files: initial_result.css_files,
        protocol: Some(initial_result.protocol),
        state_data: Some(initial_result.state_data),
        entry: args.app_args.entry.clone(),
    }));

    // The watcher handle must outlive the server; dropping it stops the
    // background watcher thread. We store it in an `Option` so that the
    // `--watch=false` branch is a no-op.
    let _watcher_handle = if let Some(active_lr) = &livereload {
        let mut watch_paths_list = paths.watch_paths();

        // Also watch local path component sources
        for extra_dir in
            webui_discovery::collect_watch_paths(&args.app_args.components, &paths.app_dir)
        {
            watch_paths_list.push(extra_dir);
        }

        let handle = start_file_watcher(WatcherConfig {
            watch_paths: watch_paths_list,
            state: Arc::clone(&state),
            render_config: render_config.clone(),
            livereload: active_lr.clone(),
        })?;
        output::success("File watcher started");
        Some(handle)
    } else {
        None
    };

    let addr = format!("127.0.0.1:{}", args.port);
    let bind_addr = addr.clone();
    let server_port = args.port;

    output::field("URL", &format!("http://{addr}/"));
    output::finish("Server is running \u{2014} press Ctrl+C to stop");

    let server_context = web::Data::new(ServerContext {
        state,
        livereload: livereload.clone(),
        assets_dir: paths.serve_dir,
        api_port: args.api_port,
        plugin: args.app_args.plugin,
        token_css: render_config.token_css,
        base_path: args.base_path.clone(),
        // Pool sized for typical concurrent renders × channel capacity.
        // 256 buffers × 5 KiB ≈ 1.25 MiB peak pool memory — bounded.
        chunk_pool: Arc::new(webui::streaming::ChunkPool::new(
            256,
            StreamingWriter::CHUNK_TARGET + 1024,
        )),
    });
    let lr_data = livereload.map(web::Data::new);

    let has_api_proxy = server_context.api_port.is_some();

    actix_web::rt::System::new()
        .block_on(async move {
            HttpServer::new(move || {
                let mut app = App::new()
                    .app_data(server_context.clone())
                    .route("/", web::get().to(handle_index))
                    .route("/index.html", web::get().to(handle_index));

                if let Some(lr) = &lr_data {
                    app = app
                        .app_data(lr.clone())
                        .route(HMR_ENDPOINT, web::get().to(sse_handler));
                }

                if has_api_proxy {
                    app = app.route("/api/{tail:.*}", web::route().to(handle_api_proxy));
                }

                app = app
                    .route(
                        "/_webui/templates",
                        web::get().to(handle_component_templates),
                    )
                    .route("/{tail:.*}", web::get().to(handle_asset))
                    .default_service(web::route().to(handle_not_found));

                app
            })
            .bind(&bind_addr)
            .map_err(|error| map_bind_error(server_port, &bind_addr, error))?
            .run()
            .await
            .map_err(|e| anyhow::anyhow!("{e}"))
        })
        .with_context(|| format!("Failed to start actix-web server on {addr}"))?;

    Ok(())
}

fn ensure_local_port_available(port: u16) -> Result<()> {
    let bind_addr = SocketAddrV4::new(Ipv4Addr::LOCALHOST, port);
    let listener = TcpListener::bind(bind_addr)
        .map_err(|error| map_bind_error(port, &bind_addr.to_string(), error))?;
    drop(listener);
    Ok(())
}

fn map_bind_error(port: u16, bind_addr: &str, error: std::io::Error) -> anyhow::Error {
    if error.kind() == ErrorKind::AddrInUse {
        return CliError::PortInUse { port }.into();
    }

    anyhow::anyhow!("Failed to bind to {bind_addr}: {error}")
}

/// Load and resolve a theme file from a `--theme` CLI value.
fn load_theme(theme: &str) -> Result<webui_tokens::TokenFile> {
    let cwd = std::env::current_dir().with_context(|| "Failed to determine current directory")?;
    let resolved = webui_tokens::resolve_theme_path(theme, &cwd)
        .with_context(|| format!("Failed to resolve theme: {theme}"))?;
    webui_tokens::load_token_file(&resolved)
        .with_context(|| format!("Failed to load theme file: {}", resolved.display()))
}

#[derive(Clone)]
struct RenderConfig {
    app_args: AppArgs,
    app_dir: PathBuf,
    state_file: Option<PathBuf>,
    /// Pre-resolved per-theme CSS strings. Computed once at startup and reused
    /// for every build-and-render cycle (initial + file-watcher rebuilds).
    token_css: Option<HashMap<String, String>>,
    /// Base path for sub-path deployment (e.g., `/commerce/`).
    base_path: Option<String>,
}

/// Result of a build-and-render cycle.
struct BuildRenderResult {
    html: String,
    css_files: HashMap<String, String>,
    protocol: WebUIProtocol,
    state_data: Value,
}

/// Build the protocol from app templates and render with explicit state data.
fn build_and_render(
    config: &RenderConfig,
    livereload: Option<&LiveReload>,
) -> Result<BuildRenderResult> {
    let build_options = config.app_args.to_build_options(&config.app_dir);
    let build_result = webui::build(build_options).with_context(|| "Build failed")?;

    let mut state: Value = match &config.state_file {
        Some(path) => {
            let json = fs::read_to_string(path)
                .with_context(|| format!("Failed to read {}", path.display()))?;
            serde_json::from_str(&json)
                .with_context(|| format!("Failed to parse state JSON from {}", path.display()))?
        }
        None => Value::Object(serde_json::Map::new()),
    };

    // Inject pre-resolved token CSS into state
    if let Some(ref token_css) = config.token_css {
        webui_tokens::inject_token_css(&mut state, token_css);
    }

    // Inject basePath into state so templates can use {{basePath}}.
    // Default to "/" when no --base-path is set — relative CSS paths
    // like "foo.css" need <base href="/"> to resolve correctly on
    // nested routes (e.g., /contacts/123 → /foo.css, not /contacts/foo.css).
    if let Value::Object(ref mut map) = state {
        let bp = config.base_path.as_deref().unwrap_or("/").to_string();
        map.insert("basePath".into(), Value::String(bp));
    }

    // Render to memory
    let mut writer = MemoryWriter::with_capacity(4096);
    let handler = create_handler(config.app_args.plugin);
    handler.handle(
        &build_result.protocol,
        &state,
        &RenderOptions::new(&config.app_args.entry, "/"),
        &mut writer,
    )?;

    let html = match livereload {
        Some(lr) => lr.inject(&writer.buf),
        None => writer.buf,
    };

    let css_map: HashMap<String, String> = build_result.css_files.into_iter().collect();

    Ok(BuildRenderResult {
        html,
        css_files: css_map,
        protocol: build_result.protocol,
        state_data: state,
    })
}

fn create_handler(plugin: Option<Plugin>) -> WebUIHandler {
    match plugin {
        Some(Plugin::Fast | Plugin::FastV2) => {
            WebUIHandler::with_plugin(|| Box::new(FastV2HydrationPlugin::new()))
        }
        Some(Plugin::FastV3) => {
            WebUIHandler::with_plugin(|| Box::new(FastV3HydrationPlugin::new()))
        }
        Some(Plugin::WebUI) => WebUIHandler::with_plugin(|| Box::new(WebUIHydrationPlugin::new())),
        None => WebUIHandler::new(),
    }
}

// ── Route handlers ──────────────────────────────────────────────────────

struct ServerContext {
    state: Arc<Mutex<SharedState>>,
    livereload: Option<LiveReload>,
    assets_dir: Option<PathBuf>,
    api_port: Option<u16>,
    plugin: Option<Plugin>,
    /// Pre-resolved token CSS keyed by theme name, injected into state at render time.
    token_css: Option<HashMap<String, String>>,
    /// Base path for sub-path deployment.
    base_path: Option<String>,
    /// Shared chunk-buffer pool. One pool per server; recycled across
    /// every streaming render so steady-state RPS does not allocate
    /// fresh chunk buffers per flush.
    chunk_pool: Arc<webui::streaming::ChunkPool>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct RequestPaths {
    route_path: String,
    request_path: String,
}

fn build_request_paths(relative: &str, query: &str) -> RequestPaths {
    let route_path = if relative.is_empty() {
        "/".to_string()
    } else {
        let mut path = String::with_capacity(relative.len() + 1);
        path.push('/');
        path.push_str(relative);
        path
    };

    if query.is_empty() {
        return RequestPaths {
            request_path: route_path.clone(),
            route_path,
        };
    }

    let mut request_path = String::with_capacity(route_path.len() + query.len() + 1);
    request_path.push_str(&route_path);
    request_path.push('?');
    request_path.push_str(query);

    RequestPaths {
        route_path,
        request_path,
    }
}

/// Fetch state from the user's API server for a given request path, including query parameters.
async fn fetch_api_state(api_port: u16, path: &str) -> Result<Value, String> {
    let client = awc::Client::new();
    let url = format!("http://127.0.0.1:{api_port}{path}");
    let mut resp = client
        .get(&url)
        .insert_header(("Accept", "application/json"))
        .send()
        .await
        .map_err(|e| format!("API proxy error: {e}"))?;
    let body = resp
        .body()
        .await
        .map_err(|e| format!("API body error: {e}"))?;
    let json: Value = serde_json::from_slice(&body).map_err(|e| format!("API JSON error: {e}"))?;
    // Expect { "state": { ... } }, fall back to entire response
    Ok(json.get("state").cloned().unwrap_or(json))
}

/// Resolve state for a request: try API proxy first, then fall back to file state.
async fn resolve_state(context: &ServerContext, request_path: &str) -> Value {
    let mut state = if let Some(api_port) = context.api_port {
        match fetch_api_state(api_port, request_path).await {
            Ok(state) => state,
            Err(e) => {
                eprintln!("  {} {e}", console::style("\u{26a0}").yellow());
                context
                    .state
                    .lock()
                    .ok()
                    .and_then(|s| s.state_data.clone())
                    .unwrap_or_else(|| Value::Object(serde_json::Map::new()))
            }
        }
    } else {
        context
            .state
            .lock()
            .ok()
            .and_then(|s| s.state_data.clone())
            .unwrap_or_else(|| Value::Object(serde_json::Map::new()))
    };

    // Inject pre-resolved token CSS into state so signals like
    // /*{{{tokens.light}}}*/ resolve at render time, regardless of
    // whether state came from a static file or the API server.
    if let Some(ref token_css) = context.token_css {
        webui_tokens::inject_token_css(&mut state, token_css);
    }

    // Inject basePath into state — default "/" for correct CSS resolution.
    if let Value::Object(ref mut map) = state {
        let bp = context.base_path.as_deref().unwrap_or("/").to_string();
        map.insert("basePath".into(), Value::String(bp));
    }

    state
}

/// Render a full HTML page using route matching from `route_path` and state lookup from
/// `request_path`, which may include a query string. Streams chunks via
/// [`StreamingWriter`]; when livereload is active, the dev-mode `<script>`
/// is spliced before `</body>` via `RenderOptions::with_body_inject` —
/// the handler emits it at the parser-synthesized `body_end` signal
/// boundary, with zero scan cost and no risk of false-marker mis-fire
/// on `</body>` literals appearing inside HTML comments / `srcdoc`.
async fn render_page_response(
    context: &web::Data<ServerContext>,
    route_path: &str,
    request_path: &str,
) -> HttpResponse {
    let mut state = resolve_state(context, request_path).await;

    let (protocol, entry, plugin) = match context.state.lock() {
        Ok(s) => (s.protocol.clone(), s.entry.clone(), context.plugin),
        Err(_) => return HttpResponse::InternalServerError().body("Internal Server Error"),
    };

    let Some(proto) = protocol else {
        return HttpResponse::InternalServerError().body("Protocol not available");
    };

    // Inject route params (nested) into state for SSR
    if let Value::Object(ref mut map) = state {
        let nested_params = webui_handler::route_handler::collect_nested_route_params(
            &proto,
            &entry,
            route_path,
            &mut webui_handler::route_matcher::CompiledRouteCache::new(),
        );
        for (k, v) in &nested_params {
            map.insert(k.clone(), Value::String(v.clone()));
        }
    }

    // Livereload script as Arc<str> so the producer thread holds a
    // single cheap clone, not a per-request String.
    let livereload_script: Option<Arc<str>> =
        context.livereload.as_ref().map(|lr| lr.client_script_arc());
    let route_path = route_path.to_string();
    let chunk_pool = Arc::clone(&context.chunk_pool);

    // Bounded channel: backpressure when client is slow, no unbounded
    // memory growth. Capacity is in chunks (≈ 4 KB each).
    let (tx, rx) =
        tokio::sync::mpsc::channel::<bytes::Bytes>(StreamingWriter::DEFAULT_CHANNEL_CAPACITY);
    let route_path_for_log = route_path.clone();
    actix_web::rt::task::spawn_blocking(move || {
        // 30 s flush deadline caps slow-loris DoS: an attacker can pin
        // a render thread for at most 30 s per chunk, then we abort
        // and free the thread.
        // Pool-acquired chunk buffers recycle across requests — steady-
        // state RPS does not allocate fresh chunk Vec per flush.
        let mut writer = StreamingWriter::new_pooled(tx, chunk_pool)
            .with_flush_timeout(std::time::Duration::from_secs(30));
        // Build RenderOptions with optional body_inject for livereload.
        // The handler emits the inject string at the structural
        // body_end boundary identified by the parser — zero scan cost,
        // no risk of false-marker mis-firing on `</body>` literals
        // appearing inside HTML comments / srcdoc / inline scripts.
        let opts_owner = RenderOptions::new(&entry, &route_path);
        let opts = match livereload_script.as_deref() {
            Some(script) => opts_owner.with_body_inject(script),
            None => opts_owner,
        };
        let handler = create_handler(plugin);
        if let Err(e) = handler.handle(&proto, &state, &opts, &mut writer) {
            // Status 200 + headers are already on the wire — we cannot
            // return an HTTP error. Log the detail so ops sees it;
            // emit a fixed HTML comment so an attacker-controlled
            // error message cannot break out of the comment via `-->`.
            log::error!("render failed for {route_path_for_log}: {e}");
            let _ = ResponseWriter::write(&mut writer, "<!-- webui: render error -->");
        }
        // `end()` now returns the typed error from the final flush
        // (`ClientDisconnected` / `StreamTimeout`) rather than
        // silently swallowing it. Log truncated-response cases at
        // debug so they're visible to operators without spamming
        // production logs — these are normal "browser navigated away
        // during a long-tail render" events.
        if let Err(e) = ResponseWriter::end(&mut writer) {
            log::debug!("render stream truncated for {route_path_for_log}: {e}");
        }
    });

    // Zero-overhead Stream adapter (no async_stream! coroutine).
    let stream =
        tokio_stream::wrappers::ReceiverStream::new(rx).map(Ok::<bytes::Bytes, actix_web::Error>);
    HttpResponse::Ok()
        .content_type("text/html; charset=utf-8")
        // Streaming responses with attacker-influencable timing should
        // not be cached by intermediaries; the body may be partial on
        // error paths.
        .insert_header(("Cache-Control", "no-store"))
        .streaming(stream)
}

async fn handle_index(req: HttpRequest, context: web::Data<ServerContext>) -> HttpResponse {
    // JSON partial render for client-side navigation
    if wants_json(&req) {
        return handle_json_partial(&req, &context, "").await;
    }

    // With API proxy, render on-the-fly with fresh state
    if context.api_port.is_some() {
        let paths = build_request_paths("", req.query_string());
        return render_page_response(&context, &paths.route_path, &paths.request_path).await;
    }

    // Without API proxy, serve pre-rendered HTML
    let html = match context.state.lock() {
        Ok(s) => s.rendered_html.clone(),
        Err(_) => return HttpResponse::InternalServerError().body("Internal Server Error"),
    };

    HttpResponse::Ok()
        .content_type("text/html; charset=utf-8")
        .body(html)
}

async fn handle_component_templates(
    req: HttpRequest,
    context: web::Data<ServerContext>,
) -> HttpResponse {
    let qs = req.query_string();
    let mut tags: Vec<&str> = Vec::new();
    let mut inv = String::new();
    for pair in qs.split('&') {
        if let Some((k, v)) = pair.split_once('=') {
            match k {
                "t" => tags.extend(v.split(',')),
                "inv" => inv = v.to_string(),
                _ => {}
            }
        }
    }
    if tags.is_empty() {
        return HttpResponse::BadRequest()
            .content_type("application/json")
            .body(r#"{"error":"missing ?t= parameter"}"#);
    }
    let Ok(state) = context.state.lock() else {
        return HttpResponse::InternalServerError()
            .content_type("application/json")
            .body(r#"{"error":"lock poisoned"}"#);
    };
    let Some(ref protocol) = state.protocol else {
        return HttpResponse::InternalServerError()
            .content_type("application/json")
            .body(r#"{"error":"no protocol"}"#);
    };
    // Per-request index — see ProtocolIndex doc for caching guidance.
    let index = webui_handler::route_handler::ProtocolIndex::new(protocol);
    let result = match webui_handler::route_handler::render_component_templates(
        protocol, &tags, &inv, &index,
    ) {
        Ok(v) => v,
        Err(e) => {
            return HttpResponse::InternalServerError()
                .content_type("application/json")
                .body(format!(r#"{{"error":"{}"}}"#, e));
        }
    };
    HttpResponse::Ok()
        .content_type("application/json")
        .json(result)
}

async fn handle_asset(
    req: HttpRequest,
    path: web::Path<String>,
    context: web::Data<ServerContext>,
) -> HttpResponse {
    let relative = path.into_inner();

    // Check in-memory CSS files first (generated by build_protocol)
    if let Ok(s) = context.state.lock() {
        if let Some(css) = s.css_files.get(&relative) {
            return HttpResponse::Ok()
                .content_type("text/css; charset=utf-8")
                .body(css.clone());
        }
    }

    let Some(assets_dir) = &context.assets_dir else {
        // No assets dir — try SPA fallback for paths without file extensions
        return spa_fallback(&req, &context, &relative).await;
    };

    let asset_path = assets_dir.join(&relative);

    let canonical = match asset_path.canonicalize() {
        Ok(p) => p,
        Err(_) => {
            // File not found — SPA fallback for paths without file extensions
            return spa_fallback(&req, &context, &relative).await;
        }
    };

    if !canonical.starts_with(assets_dir) {
        return HttpResponse::Forbidden().body("Forbidden");
    }

    let body = match fs::read(&canonical) {
        Ok(bytes) => bytes,
        Err(_) => return spa_fallback(&req, &context, &relative).await,
    };

    let content_type = from_path(&canonical).first_or_octet_stream();

    HttpResponse::Ok()
        .content_type(content_type.as_ref())
        .body(body)
}

/// Check if the request accepts JSON (for partial render).
fn wants_json(req: &HttpRequest) -> bool {
    req.headers()
        .get("accept")
        .and_then(|v| v.to_str().ok())
        .is_some_and(|v| v.contains("application/json"))
}

/// SPA fallback: serve HTML or JSON partial depending on Accept header.
/// Activates for paths that look like route paths (no file extension).
async fn spa_fallback(
    req: &HttpRequest,
    context: &web::Data<ServerContext>,
    relative: &str,
) -> HttpResponse {
    // Only serve fallback for paths without file extensions (likely route paths)
    if relative.contains('.') {
        return HttpResponse::NotFound().body("Not Found");
    }

    let paths = build_request_paths(relative, req.query_string());

    // JSON partial render: return { state, templates } for client-side navigation
    if wants_json(req) {
        return handle_json_partial(req, context, relative).await;
    }

    render_page_response(context, &paths.route_path, &paths.request_path).await
}

/// Handle a JSON partial render request for client-side navigation.
///
/// Returns `{ state, templates, inventory, path, chain }` where:
/// - `templates` only includes f-templates the client doesn't already have
/// - `inventory` is the updated hex bitmask including the new templates
async fn handle_json_partial(
    req: &HttpRequest,
    context: &web::Data<ServerContext>,
    relative: &str,
) -> HttpResponse {
    let paths = build_request_paths(relative, req.query_string());

    let mut state_data = resolve_state(context, &paths.request_path).await;

    // Clone protocol from shared state (release lock quickly)
    let (protocol, entry) = match context.state.lock() {
        Ok(s) => (s.protocol.clone(), s.entry.clone()),
        Err(_) => {
            return HttpResponse::InternalServerError().body(r#"{"error":"Internal server error"}"#)
        }
    };

    // Inject route params into state from walking the fragment graph.
    if let Value::Object(ref mut map) = state_data {
        if let Some(proto) = &protocol {
            let nested_params = webui_handler::route_handler::collect_nested_route_params(
                proto,
                &entry,
                &paths.route_path,
                &mut webui_handler::route_matcher::CompiledRouteCache::new(),
            );
            for (k, v) in &nested_params {
                map.insert(k.clone(), Value::String(v.clone()));
            }
        }
    }

    // Get needed component templates via the handler's graph walk + inventory filter
    let client_inv_hex = req
        .headers()
        .get("x-webui-inventory")
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default()
        .to_string();

    // Build the complete partial response (templateStyles, templates, inventory, path, chain)
    let partial = if let Some(proto) = &protocol {
        // Per-request index — ideally cached alongside protocol for server lifetime.
        let mut index = webui_handler::route_handler::ProtocolIndex::new(proto);
        let mut p = match webui_handler::route_handler::render_partial(
            proto,
            &entry,
            &paths.route_path,
            &client_inv_hex,
            &mut index,
        ) {
            Ok(v) => v,
            Err(e) => {
                return HttpResponse::InternalServerError()
                    .content_type("application/json")
                    .body(format!(r#"{{"error":"{}"}}"#, e));
            }
        };
        if let Some(obj) = p.as_object_mut() {
            obj.insert("state".into(), state_data);
        }
        p
    } else {
        Value::Object(serde_json::Map::new())
    };

    HttpResponse::Ok()
        .content_type("application/json")
        .json(partial)
}

#[cfg(test)]
fn collect_needed_template_names(
    protocol: &WebUIProtocol,
    entry_fragment_id: &str,
    request_path: &str,
    inventory_hex: &str,
) -> (Vec<String>, String) {
    let component_index = webui_handler::route_handler::build_component_index(protocol);
    webui_handler::route_handler::get_needed_components_for_request(
        protocol,
        entry_fragment_id,
        request_path,
        inventory_hex,
        &component_index,
    )
    .unwrap()
}

/// Forward requests under `/api/*` to the user's API server.
async fn handle_api_proxy(
    req: HttpRequest,
    path: web::Path<String>,
    body: web::Bytes,
    context: web::Data<ServerContext>,
) -> HttpResponse {
    let Some(api_port) = context.api_port else {
        return HttpResponse::NotFound().body("Not Found");
    };

    let tail = path.into_inner();
    let query = req.query_string();
    let url = if query.is_empty() {
        format!("http://127.0.0.1:{api_port}/api/{tail}")
    } else {
        let mut u = String::with_capacity(30 + tail.len() + query.len());
        u.push_str("http://127.0.0.1:");
        u.push_str(&api_port.to_string());
        u.push_str("/api/");
        u.push_str(&tail);
        u.push('?');
        u.push_str(query);
        u
    };

    let client = awc::Client::new();
    let mut proxy_req = client.request(req.method().clone(), &url);

    // Forward content-type header if present
    if let Some(ct) = req.headers().get("content-type") {
        proxy_req = proxy_req.insert_header(("content-type", ct.clone()));
    }

    let result = if body.is_empty() {
        proxy_req.send().await
    } else {
        proxy_req.send_body(body).await
    };

    match result {
        Ok(mut resp) => {
            let status = resp.status();
            match resp.body().await {
                Ok(response_body) => {
                    let mut builder = HttpResponse::build(status);
                    if let Some(ct) = resp.headers().get("content-type") {
                        builder.insert_header(("content-type", ct.clone()));
                    }
                    builder.body(response_body)
                }
                Err(e) => HttpResponse::BadGateway().body(format!("API proxy body error: {e}")),
            }
        }
        Err(e) => HttpResponse::BadGateway().body(format!("API proxy error: {e}")),
    }
}

async fn handle_not_found() -> HttpResponse {
    HttpResponse::NotFound().body("Not Found")
}

// ── File watcher ────────────────────────────────────────────────────────

/// Filesystem-event debounce window. Editors often save in multiple
/// bursts; coalescing into one rebuild per burst feels right.
const WATCH_DEBOUNCE: Duration = Duration::from_millis(50);

/// Configuration for the file watcher.
struct WatcherConfig {
    watch_paths: Vec<PathBuf>,
    state: Arc<Mutex<SharedState>>,
    render_config: RenderConfig,
    livereload: LiveReload,
}

/// Start a debounced filesystem watcher that rebuilds and re-renders
/// when template, data, or asset files change. The returned handle owns
/// the background watcher thread; it must be kept alive for the lifetime
/// of the server.
fn start_file_watcher(config: WatcherConfig) -> Result<webui_dev_server::WatcherHandle> {
    let WatcherConfig {
        watch_paths,
        state,
        render_config,
        livereload,
    } = config;

    // The shared rebuild worker handles tick coalescing, success/error
    // reporting (rolling line + timestamps), and livereload broadcast.
    // The closure here is just the cli-specific render-and-update step.
    let lr_for_inject = livereload.clone();
    let tick_tx = webui_dev_server::spawn_rebuild_worker(livereload, move || {
        let result = build_and_render(&render_config, Some(&lr_for_inject)).map_err(|err| {
            let (display, message) = crate::utils::output::build_error_renderings(&err);
            webui_dev_server::RebuildError::new(display, message)
        })?;
        match state.lock() {
            Ok(mut s) => {
                s.rendered_html = result.html;
                s.css_files = result.css_files;
                s.protocol = Some(result.protocol);
                s.state_data = Some(result.state_data);
                Ok(())
            }
            Err(_) => Err(webui_dev_server::RebuildError::plain(
                "shared state mutex poisoned".to_owned(),
            )),
        }
    });

    let mut ignore = webui_dev_server::default_ignore_paths();
    // Also ignore the build output dir if it lives under a watched root.
    if let Ok(out_dir) = std::env::current_dir() {
        ignore.push(out_dir.join("dist"));
    }

    spawn_watcher(
        WatchConfig {
            paths: watch_paths,
            ignore,
            debounce: WATCH_DEBOUNCE,
        },
        move |_paths: Vec<std::path::PathBuf>| {
            // webui-cli rebuild is a single full-app rebuild — it doesn't
            // need per-path classification, so the paths are discarded.
            // If the worker thread has already terminated, ignore send errors.
            let _ = tick_tx.try_send(());
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use actix_web::http::StatusCode;
    use actix_web::test as actix_test;
    use tempfile::TempDir;
    use webui_protocol::{FragmentList, WebUIFragment, WebUIProtocol, WebUiFragmentRoute};

    fn create_app_dir(files: &[(&str, &str)]) -> TempDir {
        let dir = TempDir::new().unwrap();
        for (name, content) in files {
            let path = dir.path().join(name);
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent).unwrap();
            }
            fs::write(&path, content).unwrap();
        }
        dir
    }

    #[test]
    fn test_build_and_render_simple() {
        let app = create_app_dir(&[("index.html", "<h1>Hello</h1>"), ("state.json", "{}")]);
        let config = RenderConfig {
            app_args: AppArgs {
                app: app.path().to_path_buf(),
                entry: "index.html".to_string(),
                css: CssStrategy::Link,
                dom: DomStrategy::Shadow,
                plugin: None,
                components: Vec::new(),
                css_file_name_template: DEFAULT_CSS_FILE_NAME_TEMPLATE.to_string(),
                css_public_base: None,
                legal_comments: LegalComments::Inline,
            },
            app_dir: app.path().to_path_buf(),
            state_file: Some(app.path().join("state.json")),
            token_css: None,
            base_path: None,
        };
        let hmr = LiveReload::new(HMR_ENDPOINT);
        let BuildRenderResult { html, .. } = build_and_render(&config, Some(&hmr)).unwrap();
        assert!(html.contains("<h1>Hello</h1>"));
    }

    #[test]
    fn test_build_and_render_with_state() {
        let app = create_app_dir(&[
            ("index.html", "<p>{{name}}</p>"),
            ("state.json", r#"{"name":"WebUI"}"#),
        ]);
        let config = RenderConfig {
            app_args: AppArgs {
                app: app.path().to_path_buf(),
                entry: "index.html".to_string(),
                css: CssStrategy::Link,
                dom: DomStrategy::Shadow,
                plugin: None,
                components: Vec::new(),
                css_file_name_template: DEFAULT_CSS_FILE_NAME_TEMPLATE.to_string(),
                css_public_base: None,
                legal_comments: LegalComments::Inline,
            },
            app_dir: app.path().to_path_buf(),
            state_file: Some(app.path().join("state.json")),
            token_css: None,
            base_path: None,
        };
        let hmr = LiveReload::new(HMR_ENDPOINT);
        let BuildRenderResult { html, .. } = build_and_render(&config, Some(&hmr)).unwrap();
        assert!(html.contains("<p>WebUI</p>"));
    }

    #[test]
    fn test_build_and_render_selects_fast_plugin_versions() {
        let app = create_app_dir(&[
            ("index.html", "<my-card></my-card>"),
            ("my-card.html", "<span>{{name}}</span>"),
            ("state.json", r#"{"name":"Alice"}"#),
        ]);

        let render = |plugin| {
            let config = RenderConfig {
                app_args: AppArgs {
                    app: app.path().to_path_buf(),
                    entry: "index.html".to_string(),
                    css: CssStrategy::Link,
                    dom: DomStrategy::Shadow,
                    plugin,
                    components: Vec::new(),
                    css_file_name_template: DEFAULT_CSS_FILE_NAME_TEMPLATE.to_string(),
                    css_public_base: None,
                    legal_comments: LegalComments::Inline,
                },
                app_dir: app.path().to_path_buf(),
                state_file: Some(app.path().join("state.json")),
                token_css: None,
                base_path: None,
            };
            build_and_render(&config, None).unwrap().html
        };

        let fast = render(Some(Plugin::Fast));
        assert!(fast.contains("<!--fe-b$$start$$0$$name$$fe-b-->"));
        assert!(!fast.contains("<!--fe:b-->"));

        let fast_v2 = render(Some(Plugin::FastV2));
        assert!(fast_v2.contains("<!--fe-b$$start$$0$$name$$fe-b-->"));
        assert!(!fast_v2.contains("<!--fe:b-->"));

        let fast_v3 = render(Some(Plugin::FastV3));
        assert!(fast_v3.contains("<!--fe:b-->"));
        assert!(!fast_v3.contains("<!--fe-b$$start$$"));
    }

    #[test]
    fn test_build_and_render_without_watch_has_no_hmr_script() {
        let app = create_app_dir(&[("index.html", "<h1>Hello</h1>"), ("state.json", "{}")]);
        let config = RenderConfig {
            app_args: AppArgs {
                app: app.path().to_path_buf(),
                entry: "index.html".to_string(),
                css: CssStrategy::Link,
                dom: DomStrategy::Shadow,
                plugin: None,
                components: Vec::new(),
                css_file_name_template: DEFAULT_CSS_FILE_NAME_TEMPLATE.to_string(),
                css_public_base: None,
                legal_comments: LegalComments::Inline,
            },
            app_dir: app.path().to_path_buf(),
            state_file: Some(app.path().join("state.json")),
            token_css: None,
            base_path: None,
        };
        let BuildRenderResult { html, .. } = build_and_render(&config, None).unwrap();
        assert!(!html.contains(HMR_ENDPOINT));
        assert!(!html.contains("EventSource"));
    }

    #[test]
    fn test_ensure_local_port_available_reports_port_in_use() {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        let port = listener.local_addr().unwrap().port();

        let error = ensure_local_port_available(port).unwrap_err();

        assert!(
            format!("{error:#}").contains(&format!("Port {port} on 127.0.0.1 is already in use"))
        );
        drop(listener);
    }

    #[test]
    fn test_build_and_render_missing_state_file() {
        let app = create_app_dir(&[("index.html", "<h1>No State</h1>")]);
        let config = RenderConfig {
            app_args: AppArgs {
                app: app.path().to_path_buf(),
                entry: "index.html".to_string(),
                css: CssStrategy::Link,
                dom: DomStrategy::Shadow,
                plugin: None,
                components: Vec::new(),
                css_file_name_template: DEFAULT_CSS_FILE_NAME_TEMPLATE.to_string(),
                css_public_base: None,
                legal_comments: LegalComments::Inline,
            },
            app_dir: app.path().to_path_buf(),
            state_file: Some(app.path().join("state.json")),
            token_css: None,
            base_path: None,
        };
        let hmr = LiveReload::new(HMR_ENDPOINT);
        let result = build_and_render(&config, Some(&hmr));
        assert!(result.is_err());
    }

    #[test]
    fn test_build_and_render_no_state_file() {
        let app = create_app_dir(&[("index.html", "<h1>Hello</h1>")]);
        let config = RenderConfig {
            app_args: AppArgs {
                app: app.path().to_path_buf(),
                entry: "index.html".to_string(),
                css: CssStrategy::Link,
                dom: DomStrategy::Shadow,
                plugin: None,
                components: Vec::new(),
                css_file_name_template: DEFAULT_CSS_FILE_NAME_TEMPLATE.to_string(),
                css_public_base: None,
                legal_comments: LegalComments::Inline,
            },
            app_dir: app.path().to_path_buf(),
            state_file: None,
            token_css: None,
            base_path: None,
        };
        let result = build_and_render(&config, None).unwrap();
        assert!(result.html.contains("<h1>Hello</h1>"));
    }

    #[test]
    fn test_build_and_render_missing_template() {
        let app = create_app_dir(&[("state.json", "{}")]);
        let config = RenderConfig {
            app_args: AppArgs {
                app: app.path().to_path_buf(),
                entry: "index.html".to_string(),
                css: CssStrategy::Link,
                dom: DomStrategy::Shadow,
                plugin: None,
                components: Vec::new(),
                css_file_name_template: DEFAULT_CSS_FILE_NAME_TEMPLATE.to_string(),
                css_public_base: None,
                legal_comments: LegalComments::Inline,
            },
            app_dir: app.path().to_path_buf(),
            state_file: Some(app.path().join("state.json")),
            token_css: None,
            base_path: None,
        };
        let hmr = LiveReload::new(HMR_ENDPOINT);
        let result = build_and_render(&config, Some(&hmr));
        assert!(result.is_err());
    }

    #[test]
    fn test_build_request_paths_preserves_query_string() {
        assert_eq!(
            build_request_paths("search", "q=shirt&sort=price-desc"),
            RequestPaths {
                route_path: "/search".to_string(),
                request_path: "/search?q=shirt&sort=price-desc".to_string(),
            }
        );
    }

    #[test]
    fn test_build_request_paths_handles_root_query() {
        assert_eq!(
            build_request_paths("", "q=shirt"),
            RequestPaths {
                route_path: "/".to_string(),
                request_path: "/?q=shirt".to_string(),
            }
        );
    }

    #[test]
    fn test_collect_needed_template_names_follows_active_route_chain() {
        let mut fragments = HashMap::new();
        fragments.insert(
            "index.html".to_string(),
            FragmentList {
                fragments: vec![WebUIFragment::component("mp-app")],
            },
        );
        fragments.insert(
            "mp-app".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::component("mp-category-nav"),
                    WebUIFragment::route_from(WebUiFragmentRoute {
                        path: "/search/:category".to_string(),
                        fragment_id: "mp-page-search".to_string(),
                        exact: true,
                        keep_alive: false,
                        ..Default::default()
                    }),
                    WebUIFragment::route_from(WebUiFragmentRoute {
                        path: "/product/:handle".to_string(),
                        fragment_id: "mp-page-product".to_string(),
                        exact: true,
                        keep_alive: false,
                        ..Default::default()
                    }),
                ],
            },
        );
        fragments.insert(
            "mp-category-nav".to_string(),
            FragmentList {
                fragments: vec![WebUIFragment::raw("<nav></nav>")],
            },
        );
        fragments.insert(
            "mp-page-search".to_string(),
            FragmentList {
                fragments: vec![WebUIFragment::component("mp-product-grid")],
            },
        );
        fragments.insert(
            "mp-product-grid".to_string(),
            FragmentList {
                fragments: vec![WebUIFragment::raw("<div></div>")],
            },
        );
        fragments.insert(
            "mp-page-product".to_string(),
            FragmentList {
                fragments: vec![WebUIFragment::component("mp-product-detail")],
            },
        );
        fragments.insert(
            "mp-product-detail".to_string(),
            FragmentList {
                fragments: vec![WebUIFragment::raw("<article></article>")],
            },
        );

        let mut protocol = WebUIProtocol::with_tokens(fragments, Vec::new());
        protocol
            .components
            .entry("mp-page-search".to_string())
            .or_default()
            .template = "<f-template id=search></f-template>".to_string();
        protocol
            .components
            .entry("mp-page-product".to_string())
            .or_default()
            .template = "<f-template id=product></f-template>".to_string();
        let (needed, inventory) =
            collect_needed_template_names(&protocol, "index.html", "/search/shirts", "");

        assert!(needed.contains(&"mp-app".to_string()));
        assert!(needed.contains(&"mp-page-search".to_string()));
        assert!(needed.contains(&"mp-product-grid".to_string()));
        assert!(needed.contains(&"mp-category-nav".to_string()));
        assert!(!needed.contains(&"mp-page-product".to_string()));
        assert!(!needed.contains(&"mp-product-detail".to_string()));
        assert!(!inventory.is_empty());
    }

    #[actix_web::test]
    async fn test_fetch_api_state_preserves_query_string() {
        let listener = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let port = listener.local_addr().unwrap().port();

        let server = HttpServer::new(|| {
            App::new().route(
                "/search",
                web::get().to(|query: web::Query<HashMap<String, String>>| async move {
                    HttpResponse::Ok().json(serde_json::json!({
                        "state": {
                            "query": query.get("q").cloned().unwrap_or_default(),
                            "sort": query.get("sort").cloned().unwrap_or_default(),
                        }
                    }))
                }),
            )
        })
        .listen(listener)
        .unwrap()
        .run();

        let handle = server.handle();
        actix_web::rt::spawn(server);

        let state = fetch_api_state(port, "/search?q=shirt&sort=price-desc")
            .await
            .unwrap();

        assert_eq!(state["query"], "shirt");
        assert_eq!(state["sort"], "price-desc");

        handle.stop(true).await;
    }

    #[test]
    fn test_hmr_script_is_injected_when_livereload_present() {
        let app = create_app_dir(&[("index.html", "<h1>Hi</h1>"), ("state.json", "{}")]);
        let config = RenderConfig {
            app_args: AppArgs {
                app: app.path().to_path_buf(),
                entry: "index.html".to_string(),
                css: CssStrategy::Link,
                dom: DomStrategy::Shadow,
                plugin: None,
                components: Vec::new(),
                css_file_name_template: DEFAULT_CSS_FILE_NAME_TEMPLATE.to_string(),
                css_public_base: None,
                legal_comments: LegalComments::Inline,
            },
            app_dir: app.path().to_path_buf(),
            state_file: Some(app.path().join("state.json")),
            token_css: None,
            base_path: None,
        };
        let lr = LiveReload::new(HMR_ENDPOINT);
        let BuildRenderResult { html, .. } = build_and_render(&config, Some(&lr)).unwrap();
        // The injected SSE bootstrap should reference the endpoint.
        assert!(html.contains(HMR_ENDPOINT));
        assert!(html.contains("EventSource"));
    }

    #[test]
    fn test_watch_paths_excludes_servedir() {
        // serve_dir is intentionally NOT watched: it is the destination
        // for client bundles written by other tools (esbuild, pnpm,
        // E2E harness). Watching it would cause spurious livereload
        // broadcasts during E2E runs.
        let dir = create_app_dir(&[("state.json", "{}"), ("public/x.css", "")]);
        let paths = ServePaths {
            app_dir: dir.path().to_path_buf(),
            state_file: Some(dir.path().join("state.json")),
            serve_dir: Some(dir.path().join("public")),
        };
        let watched = paths.watch_paths();
        assert_eq!(watched.len(), 2);
        assert!(watched.contains(&dir.path().to_path_buf()));
        assert!(watched.contains(&dir.path().join("state.json")));
        assert!(!watched.contains(&dir.path().join("public")));
    }

    #[test]
    fn test_build_and_render_hello_world_example() {
        let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let app_dir = manifest_dir.join("../../examples/app/hello-world/src");
        let config = RenderConfig {
            app_args: AppArgs {
                app: app_dir.clone(),
                entry: "index.html".to_string(),
                css: CssStrategy::Link,
                dom: DomStrategy::Shadow,
                plugin: None,
                components: Vec::new(),
                css_file_name_template: DEFAULT_CSS_FILE_NAME_TEMPLATE.to_string(),
                css_public_base: None,
                legal_comments: LegalComments::Inline,
            },
            app_dir,
            state_file: Some(manifest_dir.join("../../examples/app/hello-world/data/state.json")),
            token_css: None,
            base_path: None,
        };
        let lr = LiveReload::new(HMR_ENDPOINT);
        let BuildRenderResult { html, .. } = build_and_render(&config, Some(&lr)).unwrap();
        assert!(html.contains("Hello, WebUI!"));
        assert!(html.contains("Ali"));
        assert!(html.contains("Mohamed Mansour"));
        // HMR script should be injected
        assert!(html.contains(HMR_ENDPOINT));
    }

    #[actix_web::test]
    async fn test_route_precedence_over_asset_catch_all() {
        let livereload = LiveReload::new(HMR_ENDPOINT);
        let lr_endpoint = livereload.endpoint().to_string();
        let context = web::Data::new(ServerContext {
            state: Arc::new(Mutex::new(SharedState {
                rendered_html: "<html><body>ok</body></html>".to_string(),
                css_files: HashMap::new(),
                protocol: None,
                state_data: None,
                entry: "index.html".to_string(),
            })),
            livereload: Some(livereload.clone()),
            assets_dir: None,
            api_port: None,
            plugin: None,
            token_css: None,
            base_path: None,
            chunk_pool: Arc::new(webui::streaming::ChunkPool::new(
                4,
                StreamingWriter::CHUNK_TARGET + 1024,
            )),
        });

        let app = actix_test::init_service(
            App::new()
                .app_data(context.clone())
                .app_data(web::Data::new(livereload))
                .route("/", web::get().to(handle_index))
                .route("/index.html", web::get().to(handle_index))
                .route(&lr_endpoint, web::get().to(sse_handler))
                .route("/{tail:.*}", web::get().to(handle_asset))
                .default_service(web::route().to(handle_not_found)),
        )
        .await;

        // SSE endpoint takes precedence over the catch-all asset route.
        let lr_response = actix_test::call_service(
            &app,
            actix_test::TestRequest::get()
                .uri(HMR_ENDPOINT)
                .to_request(),
        )
        .await;
        assert_eq!(lr_response.status(), StatusCode::OK);
        let content_type = lr_response
            .headers()
            .get("content-type")
            .map(|v| v.to_str().unwrap_or("").to_string())
            .unwrap_or_default();
        assert!(
            content_type.starts_with("text/event-stream"),
            "expected SSE content-type, got {content_type:?}"
        );

        let index_response = actix_test::call_service(
            &app,
            actix_test::TestRequest::get()
                .uri("/index.html")
                .to_request(),
        )
        .await;
        assert_eq!(index_response.status(), StatusCode::OK);

        let index_body = actix_test::read_body(index_response).await;
        assert_eq!(
            index_body,
            web::Bytes::from_static(b"<html><body>ok</body></html>")
        );
    }
}

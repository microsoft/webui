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
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, SystemTime};
use webui::WebUIHandler;
use webui_handler::plugin::FastHydrationPlugin;
use webui_handler::{RenderOptions, ResponseWriter};
use webui_protocol::WebUIProtocol;

use super::common::*;
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
            .with_context(|| format!("App folder not found: {}", args.app_args.app.display()))?;

        let state_file = match &args.state {
            Some(state_path) => {
                let state_input = expand_tilde(state_path)
                    .with_context(|| {
                        format!("Failed to expand state path: {}", state_path.display())
                    })?
                    .into_owned();

                let canonical = state_input
                    .canonicalize()
                    .with_context(|| format!("State file not found: {}", state_path.display()))?;

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

                let canonical = serve_input.canonicalize().with_context(|| {
                    format!("Serve directory not found: {}", serve_arg.display())
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

    fn watch_targets(&self) -> Vec<WatchTarget> {
        let mut targets = vec![WatchTarget::Directory(self.app_dir.clone())];

        if let Some(state_file) = &self.state_file {
            targets.push(WatchTarget::File(state_file.clone()));
        }

        if let Some(serve_dir) = &self.serve_dir {
            targets.push(WatchTarget::Directory(serve_dir.clone()));
        }

        targets
    }
}

/// Thread-safe shared state: the rendered HTML for serving.
struct SharedState {
    rendered_html: String,
    hmr_version: u64,
    css_files: HashMap<String, String>,
    protocol: Option<WebUIProtocol>,
    state_data: Option<Value>,
    component_templates: HashMap<String, String>,
    /// Entry fragment ID used for rendering (e.g., "index.html").
    entry: String,
}

impl SharedState {
    fn bump_version(&mut self) {
        self.hmr_version = self.hmr_version.wrapping_add(1);
    }
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

trait HmrBackend: Send + Sync {
    fn endpoint_path(&self) -> &str;
    fn inject(&self, html: &str) -> String;
    fn version_payload(&self, state: &SharedState) -> String;
}

struct PollingHmrBackend {
    endpoint: &'static str,
    interval_ms: u64,
}

impl PollingHmrBackend {
    fn new(endpoint: &'static str, interval_ms: u64) -> Self {
        Self {
            endpoint,
            interval_ms,
        }
    }

    fn script(&self) -> String {
        format!(
            r#"<script>
(function(){{
  var v=null;
  function c(){{
    var x=new XMLHttpRequest();
    x.open("GET","{}",true);
    x.onload=function(){{
      if(x.status===200){{
        var t=x.responseText.trim();
        if(v===null){{v=t}}
        else if(v!==t){{location.reload();return}}
      }}
      setTimeout(c,{});
    }};
    x.onerror=function(){{setTimeout(c,{})}};
    x.send();
  }}
  if(document.readyState==="loading"){{
    document.addEventListener("DOMContentLoaded",c);
  }}else{{c()}}
}})();
</script>"#,
            self.endpoint, self.interval_ms, self.interval_ms
        )
    }
}

impl HmrBackend for PollingHmrBackend {
    fn endpoint_path(&self) -> &str {
        self.endpoint
    }

    fn inject(&self, html: &str) -> String {
        inject_script_before_body_close(html, &self.script())
    }

    fn version_payload(&self, state: &SharedState) -> String {
        state.hmr_version.to_string()
    }
}

pub fn execute(args: &ServeArgs) -> Result<()> {
    run(args).map_err(|err| {
        output::error(&err);

        let err_msg = format!("{:#}", err);
        if err_msg.contains("App folder not found") {
            output::hint("Check that the app folder path exists");
        } else if err_msg.contains("State file not found") {
            output::hint("Pass a valid --state path to a JSON file");
        } else if err_msg.contains("Serve directory not found") {
            output::hint("Pass a valid --servedir path for static assets");
        }
        eprintln!();
        err
    })
}

fn run(args: &ServeArgs) -> Result<()> {
    let paths = ServePaths::from_args(args)?;
    let hmr_backend: Option<Arc<dyn HmrBackend>> = if args.watch {
        Some(Arc::new(PollingHmrBackend::new("/hmr", 1000)))
    } else {
        None
    };

    let render_config = RenderConfig {
        app_args: args.app_args.clone(),
        app_dir: paths.app_dir.clone(),
        state_file: paths.state_file.clone(),
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
    output::field("Entry", &args.app_args.entry);
    output::field("Port", &args.port);
    output::field("CSS", &format!("{:?}", args.app_args.css));
    if let Some(api_port) = args.api_port {
        output::field("API Port", &api_port);
    }
    if args.watch {
        output::field("HMR", &"enabled (polling /hmr)");
    } else {
        output::field("HMR", &"disabled (pass --watch to enable)");
    }
    eprintln!();

    // Initial build + render
    let initial_result = build_and_render(&render_config, hmr_backend.as_deref())?;
    output::success("Initial build and render complete");

    let state = Arc::new(Mutex::new(SharedState {
        rendered_html: initial_result.html,
        hmr_version: 1,
        css_files: initial_result.css_files,
        protocol: Some(initial_result.protocol),
        state_data: Some(initial_result.state_data),
        component_templates: initial_result.component_templates,
        entry: args.app_args.entry.clone(),
    }));

    if let Some(active_hmr_backend) = &hmr_backend {
        let mut watch_targets = paths.watch_targets();

        // Also watch local path component sources
        for extra_dir in
            webui_discovery::collect_watch_paths(&args.app_args.components, &paths.app_dir)
        {
            watch_targets.push(WatchTarget::Directory(extra_dir));
        }

        start_file_watcher(WatcherConfig {
            watch_targets,
            state: Arc::clone(&state),
            render_config: render_config.clone(),
            hmr_backend: Arc::clone(active_hmr_backend),
        });
        output::success("File watcher started");
    }

    let addr = format!("127.0.0.1:{}", args.port);
    let bind_addr = addr.clone();

    output::field("URL", &format!("http://{addr}/"));
    output::finish("Server is running \u{2014} press Ctrl+C to stop");

    let server_context = web::Data::new(ServerContext {
        state,
        hmr_backend,
        assets_dir: paths.serve_dir,
        api_port: args.api_port,
        plugin: args.app_args.plugin.clone(),
    });

    let has_api_proxy = server_context.api_port.is_some();

    actix_web::rt::System::new()
        .block_on(async move {
            let hmr_endpoint = server_context
                .hmr_backend
                .as_ref()
                .map(|backend| backend.endpoint_path().to_string());

            HttpServer::new(move || {
                let mut app = App::new()
                    .app_data(server_context.clone())
                    .route("/", web::get().to(handle_index))
                    .route("/index.html", web::get().to(handle_index))
                    .route("/hmr", web::get().to(handle_hmr));

                if has_api_proxy {
                    app = app.route("/api/{tail:.*}", web::route().to(handle_api_proxy));
                }

                app = app
                    .route("/{tail:.*}", web::get().to(handle_asset))
                    .default_service(web::route().to(handle_not_found));

                if let Some(endpoint) = hmr_endpoint.as_ref() {
                    if endpoint != "/hmr" {
                        app = app.route(endpoint, web::get().to(handle_hmr));
                    }
                }

                app
            })
            .bind(&bind_addr)
            .with_context(|| format!("Failed to bind to {bind_addr}"))?
            .run()
            .await
            .map_err(|e| anyhow::anyhow!("{e}"))
        })
        .with_context(|| format!("Failed to start actix-web server on {addr}"))?;

    Ok(())
}

#[derive(Clone)]
struct RenderConfig {
    app_args: AppArgs,
    app_dir: PathBuf,
    state_file: Option<PathBuf>,
}

/// Result of a build-and-render cycle.
struct BuildRenderResult {
    html: String,
    css_files: HashMap<String, String>,
    protocol: WebUIProtocol,
    state_data: Value,
    component_templates: HashMap<String, String>,
}

/// Build the protocol from app templates and render with explicit state data.
fn build_and_render(
    config: &RenderConfig,
    hmr_backend: Option<&dyn HmrBackend>,
) -> Result<BuildRenderResult> {
    let build_options = config.app_args.to_build_options(&config.app_dir);
    let build_result = webui::build(build_options).with_context(|| "Build failed")?;

    let state: Value = match &config.state_file {
        Some(path) => {
            let json = fs::read_to_string(path)
                .with_context(|| format!("Failed to read {}", path.display()))?;
            serde_json::from_str(&json)
                .with_context(|| format!("Failed to parse state JSON from {}", path.display()))?
        }
        None => Value::Object(serde_json::Map::new()),
    };

    // Render to memory
    let mut writer = MemoryWriter::with_capacity(4096);
    let handler = match config.app_args.plugin.as_deref() {
        Some("fast") => WebUIHandler::with_plugin(|| Box::new(FastHydrationPlugin::new())),
        _ => WebUIHandler::new(),
    };
    handler.handle(
        &build_result.protocol,
        &state,
        &RenderOptions::new(&config.app_args.entry, "/"),
        &mut writer,
    )?;

    let html = match hmr_backend {
        Some(backend) => backend.inject(&writer.buf),
        None => writer.buf,
    };

    let css_map: HashMap<String, String> = build_result.css_files.into_iter().collect();
    let template_map: HashMap<String, String> =
        build_result.component_templates.into_iter().collect();

    Ok(BuildRenderResult {
        html,
        css_files: css_map,
        protocol: build_result.protocol,
        state_data: state,
        component_templates: template_map,
    })
}

fn inject_script_before_body_close(html: &str, script: &str) -> String {
    // Insert before </body> if found, otherwise append
    if let Some(pos) = html.rfind("</body>") {
        let mut result = String::with_capacity(html.len() + script.len() + 1);
        result.push_str(&html[..pos]);
        result.push('\n');
        result.push_str(script);
        result.push('\n');
        result.push_str(&html[pos..]);
        result
    } else {
        let mut result = String::with_capacity(html.len() + script.len());
        result.push_str(html);
        result.push_str(script);
        result
    }
}

// ── Route handlers ──────────────────────────────────────────────────────

struct ServerContext {
    state: Arc<Mutex<SharedState>>,
    hmr_backend: Option<Arc<dyn HmrBackend>>,
    assets_dir: Option<PathBuf>,
    api_port: Option<u16>,
    plugin: Option<String>,
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
    if let Some(api_port) = context.api_port {
        match fetch_api_state(api_port, request_path).await {
            Ok(state) => return state,
            Err(e) => {
                eprintln!("  {} {e}", console::style("\u{26a0}").yellow());
            }
        }
    }

    context
        .state
        .lock()
        .ok()
        .and_then(|s| s.state_data.clone())
        .unwrap_or_else(|| Value::Object(serde_json::Map::new()))
}

/// Render a full HTML page using route matching from `route_path` and state lookup from
/// `request_path`, which may include a query string.
async fn render_page_response(
    context: &web::Data<ServerContext>,
    route_path: &str,
    request_path: &str,
) -> HttpResponse {
    let mut state = resolve_state(context, request_path).await;

    let (protocol, entry, plugin) = match context.state.lock() {
        Ok(s) => (s.protocol.clone(), s.entry.clone(), context.plugin.clone()),
        Err(_) => return HttpResponse::InternalServerError().body("Internal Server Error"),
    };

    let Some(proto) = protocol else {
        return HttpResponse::InternalServerError().body("Protocol not available");
    };

    // Inject route params (nested) into state for SSR
    if let Value::Object(ref mut map) = state {
        let nested_params =
            webui_handler::route_handler::collect_nested_route_params(&proto, &entry, route_path);
        for (k, v) in &nested_params {
            map.insert(k.clone(), Value::String(v.clone()));
        }
    }

    let mut writer = MemoryWriter::with_capacity(4096);
    let handler = match plugin.as_deref() {
        Some("fast") => WebUIHandler::with_plugin(|| Box::new(FastHydrationPlugin::new())),
        _ => WebUIHandler::new(),
    };

    if let Err(e) = handler.handle(
        &proto,
        &state,
        &RenderOptions::new(&entry, route_path),
        &mut writer,
    ) {
        return HttpResponse::InternalServerError().body(format!("Render error: {e}"));
    }

    let html = match &context.hmr_backend {
        Some(backend) => backend.inject(&writer.buf),
        None => writer.buf,
    };

    HttpResponse::Ok()
        .content_type("text/html; charset=utf-8")
        .body(html)
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

async fn handle_hmr(context: web::Data<ServerContext>) -> HttpResponse {
    let Some(hmr_backend) = context.hmr_backend.as_ref() else {
        return HttpResponse::Ok()
            .insert_header(("Cache-Control", "no-cache, no-store, must-revalidate"))
            .insert_header(("Pragma", "no-cache"))
            .insert_header(("Expires", "0"))
            .content_type("text/plain; charset=utf-8")
            .body("0");
    };

    let version = match context.state.lock() {
        Ok(s) => hmr_backend.version_payload(&s),
        Err(_) => "0".to_string(),
    };

    HttpResponse::Ok()
        .insert_header(("Cache-Control", "no-cache, no-store, must-revalidate"))
        .insert_header(("Pragma", "no-cache"))
        .insert_header(("Expires", "0"))
        .content_type("text/plain; charset=utf-8")
        .body(version)
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

    // Clone protocol and templates from shared state (release lock quickly)
    let (protocol, component_templates, entry) = match context.state.lock() {
        Ok(s) => (
            s.protocol.clone(),
            s.component_templates.clone(),
            s.entry.clone(),
        ),
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

    // Build the complete partial response (state, templates, inventory, path, chain) in one call
    let partial = if let Some(proto) = &protocol {
        webui_handler::route_handler::render_partial(
            proto,
            state_data,
            &entry,
            &paths.route_path,
            &client_inv_hex,
        )
    } else {
        Value::Object(serde_json::Map::new())
    };

    // The partial contains f-template HTML strings; the CLI dev server stores
    // templates separately in component_templates, so map from names to those.
    // Re-derive needed names for the template lookup against the local map.
    let (needed_names, inv_hex) = if let Some(proto) = &protocol {
        collect_needed_template_names(proto, &entry, &paths.route_path, &client_inv_hex)
    } else {
        (Vec::new(), client_inv_hex)
    };

    let templates: Vec<Value> = needed_names
        .iter()
        .filter_map(|name| component_templates.get(name))
        .map(|t| Value::String(t.clone()))
        .collect();

    // Start from the partial (which has state, path, chain) and override
    // templates/inventory with the CLI dev server's local template map.
    let mut resp = match partial {
        Value::Object(m) => m,
        _ => serde_json::Map::new(),
    };
    resp.insert("templates".into(), Value::Array(templates));
    resp.insert("inventory".into(), Value::String(inv_hex));

    HttpResponse::Ok()
        .content_type("application/json")
        .json(Value::Object(resp))
}

fn collect_needed_template_names(
    protocol: &WebUIProtocol,
    entry_fragment_id: &str,
    request_path: &str,
    inventory_hex: &str,
) -> (Vec<String>, String) {
    webui_handler::route_handler::get_needed_components_for_request(
        protocol,
        entry_fragment_id,
        request_path,
        inventory_hex,
    )
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

/// Collect modification times for all files under a directory, iteratively.
fn collect_directory_file_times(root: &Path, file_times: &mut HashMap<PathBuf, SystemTime>) {
    if let Ok(meta) = fs::metadata(root) {
        if let Ok(modified) = meta.modified() {
            file_times.insert(root.to_path_buf(), modified);
        }
    }

    let mut stack = Vec::with_capacity(8);
    stack.push(root.to_path_buf());

    while let Some(dir) = stack.pop() {
        let entries = match fs::read_dir(&dir) {
            Ok(e) => e,
            Err(_) => continue,
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
            } else if let Ok(meta) = fs::metadata(&path) {
                if let Ok(modified) = meta.modified() {
                    file_times.insert(path, modified);
                }
            }
        }
    }
}

#[derive(Clone)]
enum WatchTarget {
    Directory(PathBuf),
    File(PathBuf),
}

fn collect_watch_times(targets: &[WatchTarget]) -> HashMap<PathBuf, SystemTime> {
    let mut file_times = HashMap::new();

    for target in targets {
        match target {
            WatchTarget::Directory(path) => {
                collect_directory_file_times(path, &mut file_times);
            }
            WatchTarget::File(path) => {
                if let Ok(meta) = fs::metadata(path) {
                    if let Ok(modified) = meta.modified() {
                        file_times.insert(path.clone(), modified);
                    }
                }
            }
        }
    }

    file_times
}

/// Detect whether any files were added, modified, or deleted.
fn has_changes(
    current: &HashMap<PathBuf, SystemTime>,
    previous: &HashMap<PathBuf, SystemTime>,
) -> bool {
    if current.len() != previous.len() {
        return true;
    }
    for (path, time) in current {
        match previous.get(path) {
            Some(prev_time) if prev_time == time => {}
            _ => return true,
        }
    }
    false
}

/// Configuration for the file watcher background thread.
struct WatcherConfig {
    watch_targets: Vec<WatchTarget>,
    state: Arc<Mutex<SharedState>>,
    render_config: RenderConfig,
    hmr_backend: Arc<dyn HmrBackend>,
}

/// Start a background file-watcher thread that rebuilds and re-renders
/// when template, data, or asset files change.
fn start_file_watcher(config: WatcherConfig) {
    thread::spawn(move || {
        let mut last_times = collect_watch_times(&config.watch_targets);

        loop {
            thread::sleep(Duration::from_millis(500));

            let current_times = collect_watch_times(&config.watch_targets);

            if has_changes(&current_times, &last_times) {
                match build_and_render(&config.render_config, Some(config.hmr_backend.as_ref())) {
                    Ok(result) => {
                        if let Ok(mut s) = config.state.lock() {
                            s.rendered_html = result.html;
                            s.css_files = result.css_files;
                            s.protocol = Some(result.protocol);
                            s.state_data = Some(result.state_data);
                            s.component_templates = result.component_templates;
                            s.bump_version();
                        }
                        eprintln!(
                            "  {} Rebuilt and re-rendered (HMR version updated)",
                            console::style("\u{21bb}").green()
                        );
                    }
                    Err(err) => {
                        eprintln!(
                            "  {} Rebuild failed: {err:#}",
                            console::style("\u{2718}").red().bold()
                        );
                    }
                }

                last_times = current_times;
            }
        }
    });
}

#[cfg(test)]
#[allow(clippy::disallowed_methods)]
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
                plugin: None,
                components: Vec::new(),
            },
            app_dir: app.path().to_path_buf(),
            state_file: Some(app.path().join("state.json")),
        };
        let hmr = PollingHmrBackend::new("/hmr", 1000);
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
                plugin: None,
                components: Vec::new(),
            },
            app_dir: app.path().to_path_buf(),
            state_file: Some(app.path().join("state.json")),
        };
        let hmr = PollingHmrBackend::new("/hmr", 1000);
        let BuildRenderResult { html, .. } = build_and_render(&config, Some(&hmr)).unwrap();
        assert!(html.contains("<p>WebUI</p>"));
    }

    #[test]
    fn test_build_and_render_without_watch_has_no_hmr_script() {
        let app = create_app_dir(&[("index.html", "<h1>Hello</h1>"), ("state.json", "{}")]);
        let config = RenderConfig {
            app_args: AppArgs {
                app: app.path().to_path_buf(),
                entry: "index.html".to_string(),
                css: CssStrategy::Link,
                plugin: None,
                components: Vec::new(),
            },
            app_dir: app.path().to_path_buf(),
            state_file: Some(app.path().join("state.json")),
        };
        let BuildRenderResult { html, .. } = build_and_render(&config, None).unwrap();
        assert!(!html.contains("/hmr"));
    }

    #[test]
    fn test_build_and_render_missing_state_file() {
        let app = create_app_dir(&[("index.html", "<h1>No State</h1>")]);
        let config = RenderConfig {
            app_args: AppArgs {
                app: app.path().to_path_buf(),
                entry: "index.html".to_string(),
                css: CssStrategy::Link,
                plugin: None,
                components: Vec::new(),
            },
            app_dir: app.path().to_path_buf(),
            state_file: Some(app.path().join("state.json")),
        };
        let hmr = PollingHmrBackend::new("/hmr", 1000);
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
                plugin: None,
                components: Vec::new(),
            },
            app_dir: app.path().to_path_buf(),
            state_file: None,
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
                plugin: None,
                components: Vec::new(),
            },
            app_dir: app.path().to_path_buf(),
            state_file: Some(app.path().join("state.json")),
        };
        let hmr = PollingHmrBackend::new("/hmr", 1000);
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
                        ..Default::default()
                    }),
                    WebUIFragment::route_from(WebUiFragmentRoute {
                        path: "/product/:handle".to_string(),
                        fragment_id: "mp-page-product".to_string(),
                        exact: true,
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
        protocol.component_templates.insert(
            "mp-page-search".to_string(),
            "<f-template id=search></f-template>".to_string(),
        );
        protocol.component_templates.insert(
            "mp-page-product".to_string(),
            "<f-template id=product></f-template>".to_string(),
        );
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
    fn test_hmr_script_injection() {
        let html = "<html><body><p>Hello</p></body></html>";
        let script = "<script>console.log('hmr')</script>";
        let injected = inject_script_before_body_close(html, script);
        assert!(injected.contains("<script>"));
        // Script should be before </body>
        let script_pos = injected.find("<script>").unwrap();
        let body_pos = injected.rfind("</body>").unwrap();
        assert!(script_pos < body_pos);
    }

    #[test]
    fn test_hmr_script_injection_no_body() {
        let html = "<h1>Hello</h1>";
        let script = "<script>console.log('hmr')</script>";
        let injected = inject_script_before_body_close(html, script);
        assert!(injected.contains("<script>"));
        assert!(injected.starts_with("<h1>Hello</h1>"));
    }

    #[test]
    fn test_collect_watch_times_iterative() {
        let dir = create_app_dir(&[
            ("a.txt", "hello"),
            ("sub/b.txt", "world"),
            ("sub/deep/c.txt", "nested"),
        ]);
        let targets = vec![WatchTarget::Directory(dir.path().to_path_buf())];
        let times = collect_watch_times(&targets);
        assert!(times.contains_key(&dir.path().join("a.txt")));
        assert!(times.contains_key(&dir.path().join("sub/b.txt")));
        assert!(times.contains_key(&dir.path().join("sub/deep/c.txt")));
    }

    #[test]
    fn test_collect_watch_times_empty_directory() {
        let dir = TempDir::new().unwrap();
        let targets = vec![WatchTarget::Directory(dir.path().to_path_buf())];
        let times = collect_watch_times(&targets);
        assert!(!times.is_empty());
    }

    #[test]
    fn test_collect_watch_times_file_target() {
        let dir = create_app_dir(&[("state.json", "{}")]);
        let file_path = dir.path().join("state.json");
        let targets = vec![WatchTarget::File(file_path.clone())];
        let times = collect_watch_times(&targets);
        assert!(times.contains_key(&file_path));
    }

    #[test]
    fn test_has_changes_identical() {
        let mut a = HashMap::new();
        let t = SystemTime::UNIX_EPOCH;
        a.insert(PathBuf::from("a.txt"), t);
        assert!(!has_changes(&a, &a));
    }

    #[test]
    fn test_has_changes_new_file() {
        let t = SystemTime::UNIX_EPOCH;
        let mut a = HashMap::new();
        a.insert(PathBuf::from("a.txt"), t);
        let mut b = HashMap::new();
        b.insert(PathBuf::from("a.txt"), t);
        b.insert(PathBuf::from("b.txt"), t);
        assert!(has_changes(&b, &a));
    }

    #[test]
    fn test_has_changes_modified() {
        let t1 = SystemTime::UNIX_EPOCH;
        let t2 = SystemTime::UNIX_EPOCH + Duration::from_secs(1);
        let mut a = HashMap::new();
        a.insert(PathBuf::from("a.txt"), t1);
        let mut b = HashMap::new();
        b.insert(PathBuf::from("a.txt"), t2);
        assert!(has_changes(&b, &a));
    }

    #[test]
    fn test_has_changes_deleted() {
        let t = SystemTime::UNIX_EPOCH;
        let mut a = HashMap::new();
        a.insert(PathBuf::from("a.txt"), t);
        a.insert(PathBuf::from("b.txt"), t);
        let mut b = HashMap::new();
        b.insert(PathBuf::from("a.txt"), t);
        assert!(has_changes(&b, &a));
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
                plugin: None,
                components: Vec::new(),
            },
            app_dir,
            state_file: Some(manifest_dir.join("../../examples/app/hello-world/data/state.json")),
        };
        let hmr = PollingHmrBackend::new("/hmr", 1000);
        let BuildRenderResult { html, .. } = build_and_render(&config, Some(&hmr)).unwrap();
        assert!(html.contains("Hello, WebUI!"));
        assert!(html.contains("Ali"));
        assert!(html.contains("Mohamed Mansour"));
        // HMR script should be injected
        assert!(html.contains("/hmr"));
    }

    #[test]
    fn test_polling_hmr_backend_uses_version_counter() {
        let backend = PollingHmrBackend::new("/hmr", 1000);
        let state = SharedState {
            rendered_html: "<p>Hello</p>".to_string(),
            hmr_version: 42,
            css_files: HashMap::new(),
            protocol: None,
            state_data: None,
            component_templates: HashMap::new(),
            entry: "index.html".to_string(),
        };
        assert_eq!(backend.version_payload(&state), "42");
    }

    #[actix_web::test]
    async fn test_route_precedence_over_asset_catch_all() {
        let hmr_backend: Arc<dyn HmrBackend> = Arc::new(PollingHmrBackend::new("/hmr", 1000));
        let hmr_endpoint = hmr_backend.endpoint_path().to_string();
        let context = web::Data::new(ServerContext {
            state: Arc::new(Mutex::new(SharedState {
                rendered_html: "<html><body>ok</body></html>".to_string(),
                hmr_version: 7,
                css_files: HashMap::new(),
                protocol: None,
                state_data: None,
                component_templates: HashMap::new(),
                entry: "index.html".to_string(),
            })),
            hmr_backend: Some(hmr_backend),
            assets_dir: None,
            api_port: None,
            plugin: None,
        });

        let app = actix_test::init_service(
            App::new()
                .app_data(context.clone())
                .route("/", web::get().to(handle_index))
                .route("/index.html", web::get().to(handle_index))
                .route(&hmr_endpoint, web::get().to(handle_hmr))
                .route("/{tail:.*}", web::get().to(handle_asset))
                .default_service(web::route().to(handle_not_found)),
        )
        .await;

        let hmr_response = actix_test::call_service(
            &app,
            actix_test::TestRequest::get().uri("/hmr").to_request(),
        )
        .await;
        assert_eq!(hmr_response.status(), StatusCode::OK);

        let hmr_body = actix_test::read_body(hmr_response).await;
        assert_eq!(hmr_body, web::Bytes::from_static(b"7"));

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

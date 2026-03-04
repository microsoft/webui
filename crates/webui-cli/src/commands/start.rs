use actix_web::{web, App, HttpResponse, HttpServer};
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
use webui_handler::plugin::FastHydrationPlugin;
use webui_handler::{ResponseWriter, WebUIHandler};
use webui_parser::plugin::FastParserPlugin;
use webui_parser::{CssStrategy, HtmlParser};
use webui_protocol::WebUIProtocol;

use super::build::CssMode;
use crate::utils::output::Printer;

#[derive(Args)]
pub struct StartArgs {
    /// Path to the template/component directory (defaults to current directory)
    #[arg(default_value = ".")]
    pub app: PathBuf,

    /// Port to bind the development server to
    #[arg(long, default_value_t = 3000)]
    pub port: u16,

    /// Entry HTML file name (defaults to index.html)
    #[arg(long, default_value = "index.html")]
    pub entry: String,

    /// CSS delivery strategy for component stylesheets
    #[arg(long, value_enum, default_value_t = CssMode::External)]
    pub css: CssMode,

    /// Path to the JSON state file used for rendering
    #[arg(long)]
    pub state: PathBuf,

    /// Optional directory to serve static assets from at /*
    #[arg(long)]
    pub servedir: Option<PathBuf>,

    /// Enable file watching + HMR (disabled by default)
    #[arg(long)]
    pub watch: bool,

    /// Parser/handler plugin to load (e.g., "fast" for FAST-HTML hydration)
    #[arg(long)]
    pub plugin: Option<String>,
}

/// Resolved paths for `webui start`.
#[derive(Clone)]
struct StartPaths {
    app_dir: PathBuf,
    state_file: PathBuf,
    serve_dir: Option<PathBuf>,
}

impl StartPaths {
    fn from_args(args: &StartArgs) -> Result<Self> {
        let app_input = expand_tilde(&args.app)
            .with_context(|| format!("Failed to expand app path: {}", args.app.display()))?
            .into_owned();
        let state_input = expand_tilde(&args.state)
            .with_context(|| format!("Failed to expand state path: {}", args.state.display()))?
            .into_owned();

        let app_dir = app_input
            .canonicalize()
            .with_context(|| format!("App folder not found: {}", args.app.display()))?;

        let state_file = state_input
            .canonicalize()
            .with_context(|| format!("State file not found: {}", args.state.display()))?;

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

        if !state_file.is_file() {
            return Err(anyhow::anyhow!(
                "State path must be a file: {}",
                state_file.display()
            ));
        }

        Ok(Self {
            app_dir,
            state_file,
            serve_dir,
        })
    }

    fn watch_targets(&self) -> Vec<WatchTarget> {
        let mut targets = vec![
            WatchTarget::Directory(self.app_dir.clone()),
            WatchTarget::File(self.state_file.clone()),
        ];

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

pub fn execute(args: &StartArgs) -> Result<()> {
    run(args).map_err(|err| {
        let printer = Printer::new();
        printer.error(&err);

        let err_msg = format!("{:#}", err);
        if err_msg.contains("App folder not found") {
            printer.hint("Check that the app folder path exists");
        } else if err_msg.contains("State file not found") {
            printer.hint("Pass a valid --state path to a JSON file");
        } else if err_msg.contains("Serve directory not found") {
            printer.hint("Pass a valid --servedir path for static assets");
        }
        eprintln!();
        err
    })
}

fn run(args: &StartArgs) -> Result<()> {
    let printer = Printer::new();
    let paths = StartPaths::from_args(args)?;
    let hmr_backend: Option<Arc<dyn HmrBackend>> = if args.watch {
        Some(Arc::new(PollingHmrBackend::new("/hmr", 1000)))
    } else {
        None
    };

    let render_config = RenderConfig {
        app_dir: paths.app_dir.clone(),
        state_file: paths.state_file.clone(),
        entry: args.entry.clone(),
        css_strategy: args.css.into(),
        plugin: args.plugin.clone(),
    };

    printer.header("WebUI Dev Server");
    printer.field("App", &paths.app_dir.display());
    printer.field("State", &paths.state_file.display());
    match &paths.serve_dir {
        Some(serve_dir) => printer.field("ServeDir", &serve_dir.display()),
        None => printer.field("ServeDir", &"(disabled)"),
    }
    printer.field("Entry", &args.entry);
    printer.field("Port", &args.port);
    printer.field("CSS", &format!("{:?}", args.css));
    if args.watch {
        printer.field("HMR", &"enabled (polling /hmr)");
    } else {
        printer.field("HMR", &"disabled (pass --watch to enable)");
    }
    eprintln!();

    // Initial build + render
    let initial_html = build_and_render(&render_config, hmr_backend.as_deref())?;
    printer.success("Initial build and render complete");

    let state = Arc::new(Mutex::new(SharedState {
        rendered_html: initial_html,
        hmr_version: 1,
    }));

    if let Some(active_hmr_backend) = &hmr_backend {
        start_file_watcher(WatcherConfig {
            watch_targets: paths.watch_targets(),
            state: Arc::clone(&state),
            render_config: render_config.clone(),
            hmr_backend: Arc::clone(active_hmr_backend),
        });
        printer.success("File watcher started");
    }

    let addr = format!("127.0.0.1:{}", args.port);
    let bind_addr = addr.clone();

    printer.field("URL", &format!("http://{addr}/"));
    printer.finish("Server is running \u{2014} press Ctrl+C to stop");

    let server_context = web::Data::new(ServerContext {
        state,
        hmr_backend,
        assets_dir: paths.serve_dir,
    });

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
                    .route("/hmr", web::get().to(handle_hmr))
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
    app_dir: PathBuf,
    state_file: PathBuf,
    entry: String,
    css_strategy: CssStrategy,
    plugin: Option<String>,
}

/// Build the protocol from app templates and render with explicit state data.
fn build_and_render(config: &RenderConfig, hmr_backend: Option<&dyn HmrBackend>) -> Result<String> {
    let mut parser = match config.plugin.as_deref() {
        Some("fast") => HtmlParser::with_plugin(Box::new(FastParserPlugin::new())),
        Some(unknown) => anyhow::bail!("Unknown plugin: {unknown}"),
        None => HtmlParser::new(),
    };
    parser.set_css_strategy(config.css_strategy);
    parser
        .component_registry_mut()
        .register_from_paths(&[&config.app_dir])
        .context("Failed to register components")?;

    let entry_path = config.app_dir.join(&config.entry);
    let html_content = fs::read_to_string(&entry_path)
        .with_context(|| format!("Failed to read {}", entry_path.display()))?;

    parser
        .parse(&config.entry, &html_content)
        .context("Failed to parse HTML")?;

    let fragments = parser.into_fragment_records();
    let protocol = WebUIProtocol { fragments };

    let json = fs::read_to_string(&config.state_file)
        .with_context(|| format!("Failed to read {}", config.state_file.display()))?;
    let state: Value = serde_json::from_str(&json).with_context(|| {
        format!(
            "Failed to parse state JSON from {}",
            config.state_file.display()
        )
    })?;

    // Render to memory
    let mut writer = MemoryWriter::with_capacity(4096);
    let mut handler = match config.plugin.as_deref() {
        Some("fast") => WebUIHandler::with_plugin(Box::new(FastHydrationPlugin::new())),
        _ => WebUIHandler::new(),
    };
    handler.handle(&protocol, &state, &mut writer)?;

    let html = match hmr_backend {
        Some(backend) => backend.inject(&writer.buf),
        None => writer.buf,
    };

    Ok(html)
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
}

async fn handle_index(context: web::Data<ServerContext>) -> HttpResponse {
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
            .content_type("text/plain; charset=utf-8")
            .body("0");
    };

    let version = match context.state.lock() {
        Ok(s) => hmr_backend.version_payload(&s),
        Err(_) => "0".to_string(),
    };

    HttpResponse::Ok()
        .content_type("text/plain; charset=utf-8")
        .body(version)
}

async fn handle_asset(path: web::Path<String>, context: web::Data<ServerContext>) -> HttpResponse {
    let Some(assets_dir) = &context.assets_dir else {
        return HttpResponse::NotFound().body("Not Found");
    };

    let relative = path.into_inner();
    let asset_path = assets_dir.join(relative);

    let canonical = match asset_path.canonicalize() {
        Ok(p) => p,
        Err(_) => return HttpResponse::NotFound().body("Not Found"),
    };

    if !canonical.starts_with(assets_dir) {
        return HttpResponse::Forbidden().body("Forbidden");
    }

    let body = match fs::read(&canonical) {
        Ok(bytes) => bytes,
        Err(_) => return HttpResponse::NotFound().body("Not Found"),
    };

    let content_type = from_path(&canonical).first_or_octet_stream();

    HttpResponse::Ok()
        .content_type(content_type.as_ref())
        .body(body)
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
                    Ok(html) => {
                        if let Ok(mut s) = config.state.lock() {
                            s.rendered_html = html;
                            s.bump_version();
                        }
                        eprintln!("  \u{21bb} Rebuilt and re-rendered (HMR version updated)");
                    }
                    Err(err) => {
                        eprintln!("  \u{2718} Rebuild failed: {err:#}");
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
            app_dir: app.path().to_path_buf(),
            state_file: app.path().join("state.json"),
            entry: "index.html".to_string(),
            css_strategy: CssStrategy::External,
            plugin: None,
        };
        let hmr = PollingHmrBackend::new("/hmr", 1000);
        let html = build_and_render(&config, Some(&hmr)).unwrap();
        assert!(html.contains("<h1>Hello</h1>"));
    }

    #[test]
    fn test_build_and_render_with_state() {
        let app = create_app_dir(&[
            ("index.html", "<p>{{name}}</p>"),
            ("state.json", r#"{"name":"WebUI"}"#),
        ]);
        let config = RenderConfig {
            app_dir: app.path().to_path_buf(),
            state_file: app.path().join("state.json"),
            entry: "index.html".to_string(),
            css_strategy: CssStrategy::External,
            plugin: None,
        };
        let hmr = PollingHmrBackend::new("/hmr", 1000);
        let html = build_and_render(&config, Some(&hmr)).unwrap();
        assert!(html.contains("<p>WebUI</p>"));
    }

    #[test]
    fn test_build_and_render_without_watch_has_no_hmr_script() {
        let app = create_app_dir(&[("index.html", "<h1>Hello</h1>"), ("state.json", "{}")]);
        let config = RenderConfig {
            app_dir: app.path().to_path_buf(),
            state_file: app.path().join("state.json"),
            entry: "index.html".to_string(),
            css_strategy: CssStrategy::External,
            plugin: None,
        };
        let html = build_and_render(&config, None).unwrap();
        assert!(!html.contains("/hmr"));
    }

    #[test]
    fn test_build_and_render_missing_state_file() {
        let app = create_app_dir(&[("index.html", "<h1>No State</h1>")]);
        let config = RenderConfig {
            app_dir: app.path().to_path_buf(),
            state_file: app.path().join("state.json"),
            entry: "index.html".to_string(),
            css_strategy: CssStrategy::External,
            plugin: None,
        };
        let hmr = PollingHmrBackend::new("/hmr", 1000);
        let result = build_and_render(&config, Some(&hmr));
        assert!(result.is_err());
    }

    #[test]
    fn test_build_and_render_missing_template() {
        let app = create_app_dir(&[("state.json", "{}")]);
        let config = RenderConfig {
            app_dir: app.path().to_path_buf(),
            state_file: app.path().join("state.json"),
            entry: "index.html".to_string(),
            css_strategy: CssStrategy::External,
            plugin: None,
        };
        let hmr = PollingHmrBackend::new("/hmr", 1000);
        let result = build_and_render(&config, Some(&hmr));
        assert!(result.is_err());
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
            app_dir,
            state_file: manifest_dir.join("../../examples/app/hello-world/data/state.json"),
            entry: "index.html".to_string(),
            css_strategy: CssStrategy::External,
            plugin: None,
        };
        let hmr = PollingHmrBackend::new("/hmr", 1000);
        let html = build_and_render(&config, Some(&hmr)).unwrap();
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
            })),
            hmr_backend: Some(hmr_backend),
            assets_dir: None,
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

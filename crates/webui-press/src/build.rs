// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

//! Build orchestrator: content pipeline → protocol build → parallel render → output.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicU64;
use std::time::Instant;

use console::style;
use rayon::prelude::*;
use serde_json::{Map, Value};
use webui::BuildOptions;
use webui_handler::{RenderOptions, ResponseWriter, WebUIHandler};
use webui_tokens::TokenFile;

use crate::content::process_content;
use crate::error::{Error, Result};
use crate::markdown::Highlighter;
use crate::types::{BuildStats, BundlerConfig, DocsConfig};

/// A page-level esbuild entry generated from local component scripts and
/// explicit `<script bundle>` sources.
#[derive(Debug, Clone)]
struct PageBundleEntry {
    /// Unique numeric ID used in the generated output filename.
    id: usize,
    /// The page path this bundle belongs to (retained for diagnostics).
    page_path: String,
    /// Local component scripts used by this page.
    component_scripts: Vec<PathBuf>,
    /// Explicit scripts from `<script bundle>` tags or custom page `scriptFile`.
    explicit_scripts: Vec<ScriptSource>,
}

/// Root esbuild entry for template chrome scripts shared by every page.
#[derive(Debug, Clone)]
struct RootBundleEntry {
    /// Template-level script such as `template/index.ts`.
    script_path: Option<PathBuf>,
    /// Local component scripts used by the template chrome.
    component_scripts: Vec<PathBuf>,
}

/// Local component script discovered from a component HTML file and optional
/// sibling TypeScript file.
#[derive(Debug, Clone)]
struct ComponentScript {
    html_content: String,
    script_path: PathBuf,
}

struct BundleThread {
    handle: Option<std::thread::JoinHandle<Result<BundleResult>>>,
}

impl BundleThread {
    fn spawn<F>(f: F) -> Self
    where
        F: FnOnce() -> Result<BundleResult> + Send + 'static,
    {
        Self {
            handle: Some(std::thread::spawn(f)),
        }
    }

    fn join(mut self) -> Result<BundleResult> {
        let Some(handle) = self.handle.take() else {
            return Err(Error::Build("Bundle thread already joined".to_string()));
        };
        handle
            .join()
            .map_err(|_| Error::Build("Bundle thread panicked".to_string()))?
    }
}

impl Drop for BundleThread {
    fn drop(&mut self) {
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}

/// Source of a bundleable script.
#[derive(Debug, Clone)]
enum ScriptSource {
    /// Inline script content to bundle.
    Inline(String),
    /// A `src` attribute path (resolved relative to the config dir).
    File(String),
}

/// Persistent state held by the dev server across rebuilds. The dev
/// server always performs a full rebuild on every watcher tick — the
/// previous incremental machinery proved too complex for the marginal
/// time savings — so this struct exists solely to amortize startup
/// costs that don't depend on user-edited content.
///
/// Today that's just the syntect `Highlighter` (the syntax + theme
/// load is ~30-50ms). One-shot CLI builds construct a fresh
/// `BuildCache::default()` and discard it.
#[derive(Default)]
pub struct BuildCache {
    /// Syntect highlighter — kept alive across rebuilds so we don't pay
    /// the syntax/theme load cost on every keystroke.
    pub(crate) highlighter: Option<Highlighter>,
    /// Suppress per-step terminal output (`✔ Rendered 31 pages`, etc.).
    /// The dev server flips this on after the first build so subsequent
    /// rebuilds collapse into the rolling rebuild line.
    pub quiet: bool,
    /// Skip the `remove_dir_all(out_dir)` step. Files are overwritten
    /// in place. Dev-mode optimization — saves the macOS-ENOTEMPTY
    /// retry path and shaves disk I/O off every rebuild. Stale files
    /// from deleted source pages survive until the next process restart.
    pub skip_clean: bool,
    /// Dev-mode flag: when true, the bundler skips minification for
    /// faster rebuilds during `webui-press serve`.
    pub dev_mode: bool,
}

impl BuildCache {
    /// Create an empty cache with verbose output and clean-then-build
    /// semantics. Subsequent calls to [`build_docs_with_cache`] will
    /// populate it on first use.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Switch the cache into dev-server rebuild mode: suppresses
    /// per-step output banners and skips wiping the output directory.
    pub fn set_dev_rebuild(&mut self) {
        self.quiet = true;
        self.skip_clean = true;
        self.dev_mode = true;
    }
}

fn load_theme(theme: &str, config_dir: &Path) -> Result<TokenFile> {
    let config_relative = config_dir.join(theme);
    let resolved = if config_relative.exists() {
        config_relative
            .canonicalize()
            .map_err(|e| Error::Build(format!("Failed to canonicalize {theme}: {e}")))?
    } else {
        webui_tokens::resolve_theme_path(theme, config_dir)
            .map_err(|e| Error::Build(format!("Failed to resolve theme {theme}: {e}")))?
    };
    webui_tokens::load_token_file(&resolved)
        .map_err(|e| Error::Build(format!("Failed to load theme {}: {e}", resolved.display())))
}

fn inject_theme_tokens(
    state: &mut Value,
    token_file: &TokenFile,
    protocol_tokens: &[String],
) -> Result<()> {
    let resolved = webui_tokens::resolve_tokens(protocol_tokens, token_file)
        .map_err(|e| Error::Build(format!("Failed to resolve theme tokens: {e}")))?;
    webui_tokens::inject_into_state(state, &resolved);
    Ok(())
}

/// Resolve a configured component source for the per-page builds.
///
/// Local paths are made absolute against `cwd` (the project root) because
/// `webui-discovery` resolves relative paths against the synthesized per-page
/// app directory, not the project. npm package names and scopes (e.g.
/// `@mai-ui`) are left bare so discovery resolves them from `node_modules`.
fn resolve_config_component_source(source: &str, cwd: &Path) -> String {
    if webui_discovery::is_local_source(source) {
        cwd.join(source).to_string_lossy().to_string()
    } else {
        source.to_string()
    }
}

// ── Output helpers ──────────────────────────────────────────────
//
// Mirrors the styling vocabulary in `crates/webui-cli/src/utils/output.rs`
// so webui-press feels at home in a workspace where users may also see
// `webui build` / `webui serve` output.

/// Monotonic counter used to produce unique per-rebuild temp-dir names.
/// Combined with the process id and a fxhash of the page path so two
/// rebuilds running in the same process can never collide on the
/// per-page scratch dir.
static REBUILD_NONCE: AtomicU64 = AtomicU64::new(0);

fn print_header(cache: &BuildCache, title: &str) {
    if cache.quiet {
        return;
    }
    eprintln!(
        "\n  {} {}",
        style("⚡").cyan().bold(),
        style(title).cyan().bold()
    );
}

fn print_success(cache: &BuildCache, message: &str) {
    if cache.quiet {
        return;
    }
    eprintln!("  {} {message}", style("✔").green());
}

fn print_finish(cache: &BuildCache, message: &str) {
    if cache.quiet {
        return;
    }
    eprintln!("\n  {} {message}\n", style("✨").green());
}

/// Build a JSON object from key-value pairs without using `json!` (which calls `unwrap`).
fn json_obj<const N: usize>(entries: [(&str, Value); N]) -> Value {
    let mut map = Map::with_capacity(N);
    for (k, v) in entries {
        map.insert(k.to_string(), v);
    }
    Value::Object(map)
}

/// A writer that collects rendered HTML into a String buffer.
struct StringWriter {
    buf: String,
}

impl StringWriter {
    fn with_capacity(cap: usize) -> Self {
        Self {
            buf: String::with_capacity(cap),
        }
    }
}

impl ResponseWriter for StringWriter {
    fn write(&mut self, content: &str) -> webui_handler::Result<()> {
        self.buf.push_str(content);
        Ok(())
    }

    fn end(&mut self) -> webui_handler::Result<()> {
        Ok(())
    }
}

/// Build a documentation site from the given configuration.
///
/// `config_dir` is the directory containing `config.json`. It is used to
/// resolve relative paths declared in the config (such as a custom page's
/// `state_file`).
///
/// One-shot variant for the CLI build command. The dev server uses
/// [`build_docs_with_cache`] so the syntect highlighter survives across
/// rebuilds.
pub fn build_docs(
    config: &DocsConfig,
    config_dir: &Path,
    template_dir: &Path,
) -> Result<BuildStats> {
    let mut cache = BuildCache::new();
    build_docs_with_cache(config, config_dir, template_dir, &mut cache)
}

/// Build a documentation site, reusing the supplied [`BuildCache`] across
/// invocations.
///
/// **Always performs a full rebuild.** The previous incremental machinery
/// (per-page state hashes, component-bundle signatures, descriptor cache,
/// search-index sig, etc.) was removed because the resulting invariants
/// were complex for a dev-server-only optimization. The rebuild is
/// already fast in practice and a from-scratch run keeps the output
/// guaranteed-consistent with the source.
///
/// The cache exists today purely to amortize startup costs that are
/// independent of user-edited content (the syntect highlighter load).
pub fn build_docs_with_cache(
    config: &DocsConfig,
    config_dir: &Path,
    template_dir: &Path,
    cache: &mut BuildCache,
) -> Result<BuildStats> {
    let start = Instant::now();
    let base_path = &config.base_path;
    let out_dir = Path::new(&config.out_dir);

    // Flat output: site files live at the root of `out_dir`. URLs in HTML
    // include `basePath` via `<base href>` and link rewriting; the dev server
    // (`webui-press serve`) maps URL `<basePath>/foo` → file `out_dir/foo`.
    // For deploys (e.g. GitHub Pages project sites), the host mounts `out_dir`
    // at `<basePath>`, so the same flat layout works.
    let site_dir = out_dir.to_path_buf();

    // Read custom CSS
    let custom_css = config
        .css
        .as_ref()
        .and_then(|p| fs::read_to_string(p).ok())
        .unwrap_or_default();
    let token_file = match &config.theme {
        Some(theme) => Some(load_theme(theme, config_dir)?),
        None => None,
    };

    print_header(cache, &config.site.title);
    eprintln!();

    // Compute the `<head>` injection string up-front, BEFORE any
    // filesystem mutations. Including it here makes it cheap to bake
    // into each page's state inside `process_content` (avoiding a
    // post-process_content mutation pass that would force-clone every
    // cached descriptor) and lets the content cache invalidate when
    // the link tags change. We don't write the CSS files yet — that
    // happens after content processing succeeds, so a content failure
    // can't corrupt the previous valid output.
    let base_css_src = template_dir.join("docs.css");
    let has_base_css = base_css_src.exists();
    let base_css_link = if has_base_css {
        format!("<link rel=\"stylesheet\" href=\"{base_path}docs.css\">")
    } else {
        String::new()
    };
    let has_theme_css = !custom_css.is_empty();
    let theme_css_link = if has_theme_css {
        format!("<link rel=\"stylesheet\" href=\"{base_path}theme.css\">")
    } else {
        String::new()
    };
    let head_injection = {
        let mut parts: Vec<String> = Vec::with_capacity(2 + config.head.len());
        if !base_css_link.is_empty() {
            parts.push(base_css_link.clone());
        }
        if !theme_css_link.is_empty() {
            parts.push(theme_css_link.clone());
        }
        for tag in &config.head {
            parts.push(tag.to_html());
        }
        parts.join("\n  ")
    };

    // Step 1: Process content. Highlighter is cached across rebuilds —
    // syntect's default syntax/theme load is ~30-50ms which we don't want
    // to pay every keystroke.
    let highlighter = cache.highlighter.take().unwrap_or_default();

    let pages = process_content(config, config_dir, &highlighter, &head_injection)?;

    // Restore the highlighter for the next rebuild.
    cache.highlighter = Some(highlighter);

    // Step 2: Resolve component sources for the per-page builds
    let mut component_sources: Vec<String> = Vec::new();
    let cwd = std::env::current_dir().unwrap_or_default();
    // Built-in component library (e.g. crates/webui-press/components/)
    let builtin_components = template_dir.parent().map(|p| p.join("components"));
    if let Some(ref bc) = builtin_components {
        if bc.exists() {
            component_sources.push(bc.to_string_lossy().to_string());
        }
    }
    // User component sources from config. Local paths are resolved against
    // the current project root; npm package names/scopes must stay bare so
    // webui-discovery resolves them from node_modules.
    if let Some(ref user_sources) = config.components {
        for source in user_sources {
            component_sources.push(resolve_config_component_source(source, &cwd));
        }
    }
    // Template-local components (e.g. docs-search, docs-theme-toggle living
    // beside the template's index.html).
    component_sources.push(template_dir.to_string_lossy().to_string());

    let template_html = fs::read_to_string(template_dir.join("index.html"))
        .map_err(|e| Error::Build(format!("Failed to read template: {e}")))?;
    let component_script_index = discover_component_scripts(&component_sources)?;

    // Step 3: Wipe the previous output and recreate the site root.
    // Always done — a from-scratch run guarantees the output matches
    // the current source with no possibility of stale per-page dirs,
    // stale CSS files, or orphaned assets surviving a config change.
    if out_dir.exists() && !cache.skip_clean {
        remove_dir_all_retry(out_dir)
            .map_err(|e| Error::Io(format!("Cannot clean output dir: {e}")))?;
    }
    fs::create_dir_all(&site_dir).map_err(|e| Error::Io(format!("Cannot create site dir: {e}")))?;

    // Materialize the CSS files referenced by the head injection. Done
    // after content processing succeeds so a content failure can't
    // corrupt the previous valid output (the wipe above still happens
    // because we clean before processing — but that order means a
    // failure leaves no output at all, never half-output).
    if has_base_css {
        fs::copy(&base_css_src, site_dir.join("docs.css"))
            .map_err(|e| Error::Io(format!("Cannot copy docs.css: {e}")))?;
    }
    if has_theme_css {
        fs::write(site_dir.join("theme.css"), &custom_css)
            .map_err(|e| Error::Io(format!("Cannot write theme.css: {e}")))?;
    }

    let handler = WebUIHandler::with_plugin(|| {
        Box::new(webui_handler::plugin::webui::WebUIHydrationPlugin::new())
    });

    // Pre-create per-page output directories sequentially (avoids races and
    // cheap fs::create_dir_all calls competing on the same parent paths).
    for page in &pages {
        let page_dir = site_dir.join(page.path.strip_prefix(base_path).unwrap_or(&page.path));
        fs::create_dir_all(&page_dir)
            .map_err(|e| Error::Io(format!("Cannot create dir {}: {e}", page_dir.display())))?;
    }

    // Step 2b: Extract <script bundle> tags from page content BEFORE
    // rendering. Page scripts become imports in the page's virtual esbuild
    // entry; the source HTML keeps plain, non-bundled scripts untouched.
    //
    // Also collect `scriptFile` entries from customPages config. These
    // produce per-page scripts without polluting the HTML string.
    let mut explicit_page_scripts: HashMap<String, Vec<ScriptSource>> = HashMap::new();
    let mut page_contents_with_scripts: HashMap<String, String> = HashMap::new();

    for page in &pages {
        let content = page.state["page"]["content"].as_str().unwrap_or("");
        if content.contains("<script") && content.contains("bundle") {
            let (modified, scripts) = extract_bundle_scripts(content);
            if !scripts.is_empty() {
                page_contents_with_scripts.insert(page.path.clone(), modified);
                explicit_page_scripts
                    .entry(page.path.clone())
                    .or_default()
                    .extend(scripts);
            }
        }
    }

    // Collect scriptFile from customPages config.
    for (link, custom_page) in &config.custom_pages {
        if let Some(script_path) = custom_page.script_file() {
            // Build the full page path matching how process_content constructs
            // URL paths: base_path + link (minus leading slash).
            let normalized = if link.ends_with('/') {
                link.clone()
            } else {
                format!("{link}/")
            };
            let page_path = format!("{}{}", base_path, &normalized[1..]);
            explicit_page_scripts
                .entry(page_path)
                .or_default()
                .push(ScriptSource::File(script_path.to_string()));
        }
    }

    let not_found_content = format!(
        "<h1>404 — Page Not Found</h1>\
         <p>The page you're looking for doesn't exist or has been moved.</p>\
         <p><a href=\"{base_path}\">← Back to Home</a></p>"
    );

    let template_script_path = template_dir.join("index.ts");
    let template_script = if template_script_path.exists() {
        Some(
            template_script_path
                .canonicalize()
                .unwrap_or(template_script_path),
        )
    } else {
        None
    };
    let root_component_scripts =
        collect_component_scripts_for_html(&[template_html.as_str()], &component_script_index)?;
    let root_bundle = if template_script.is_some() || !root_component_scripts.is_empty() {
        Some(RootBundleEntry {
            script_path: template_script,
            component_scripts: root_component_scripts,
        })
    } else {
        None
    };

    // Build virtual entries only for pages with content-specific scripts.
    // Template/chrome scripts are loaded once through the root entry, so pages
    // that only use template components do not get duplicate tiny page-N files.
    let mut page_bundles: Vec<PageBundleEntry> = Vec::new();
    let mut page_bundle_ids: HashMap<String, usize> = HashMap::with_capacity(pages.len());
    let mut page_bundle_signatures: HashMap<String, usize> = HashMap::new();
    for page in &pages {
        let content = page_contents_with_scripts
            .get(&page.path)
            .map(String::as_str)
            .unwrap_or_else(|| page.state["page"]["content"].as_str().unwrap_or(""));
        let mut component_scripts =
            collect_component_scripts_for_html(&[content], &component_script_index)?;
        if let Some(root) = &root_bundle {
            component_scripts.retain(|path| !root.component_scripts.contains(path));
        }
        let explicit_scripts = explicit_page_scripts.remove(&page.path).unwrap_or_default();
        if component_scripts.is_empty() && explicit_scripts.is_empty() {
            continue;
        }

        let signature = page_bundle_signature(&component_scripts, &explicit_scripts, config_dir);
        if let Some(&id) = page_bundle_signatures.get(&signature) {
            page_bundle_ids.insert(page.path.clone(), id);
            continue;
        }

        let id = page_bundles.len();
        page_bundle_signatures.insert(signature, id);
        page_bundle_ids.insert(page.path.clone(), id);
        page_bundles.push(PageBundleEntry {
            id,
            page_path: page.path.clone(),
            component_scripts,
            explicit_scripts,
        });
    }

    let not_found_component_scripts =
        collect_component_scripts_for_html(&[not_found_content.as_str()], &component_script_index)?;
    let not_found_bundle_id = if not_found_component_scripts.is_empty() {
        None
    } else {
        let signature = page_bundle_signature(&not_found_component_scripts, &[], config_dir);
        if let Some(&id) = page_bundle_signatures.get(&signature) {
            Some(id)
        } else {
            let id = page_bundles.len();
            page_bundle_signatures.insert(signature, id);
            page_bundles.push(PageBundleEntry {
                id,
                page_path: format!("{base_path}404/"),
                component_scripts: not_found_component_scripts,
                explicit_scripts: Vec::new(),
            });
            Some(id)
        }
    };

    let root_bundle_clone = root_bundle.clone();

    // Kick off TypeScript bundling on a background thread. esbuild is
    // independent of the render pipeline, so we overlap it with the
    // per-page protocol build + render.

    let site_dir_clone = site_dir.clone();
    let page_bundles_clone = page_bundles.clone();
    let bundler_config = config.bundler.clone();
    let dev_mode = cache.dev_mode;
    let config_dir_owned = config_dir.to_path_buf();
    let node_modules_owned = resolve_node_modules(config_dir)?;
    let bundle_thread = BundleThread::spawn(move || -> Result<BundleResult> {
        bundle_assets(&BundleOptions {
            site_dir: &site_dir_clone,
            node_modules: &node_modules_owned,
            root_bundle: root_bundle_clone.as_ref(),
            page_bundles: &page_bundles_clone,
            bundler_config: bundler_config.as_ref(),
            dev_mode,
            config_dir: &config_dir_owned,
        })
    });

    // Per-page build + render + write in parallel.
    //
    // Each page is its own complete app: we substitute `{{{page.content}}}`
    // in the template with the page's actual HTML (including any custom
    // elements like <code-block>) and run `webui::build()` on the result.
    // This means the build pipeline naturally discovers the components used
    // on this page, expands their declarative shadow DOM, and emits their
    // template metadata to `window.__webui.templates` for client-side
    // hydration — no manual registration tricks required.
    let total_bytes = std::sync::atomic::AtomicUsize::new(0);
    let component_css: std::sync::Mutex<HashMap<String, String>> =
        std::sync::Mutex::new(HashMap::new());

    pages.par_iter().try_for_each(|page| -> Result<()> {
        let page_dir = site_dir.join(page.path.strip_prefix(base_path).unwrap_or(&page.path));
        let target = page_dir.join("index.html");

        // Use modified content (with bundled script tags removed) if scripts were extracted.
        let content = page_contents_with_scripts
            .get(&page.path)
            .map(|s| s.as_str())
            .unwrap_or_else(|| page.state["page"]["content"].as_str().unwrap_or(""));

        // Protect <pre> blocks from HTML parser whitespace normalization.
        let (protected, pre_blocks) = protect_pre_blocks(content);

        // Substitute the raw signal in the template with the literal HTML.
        let page_html = template_html.replace("{{{page.content}}}", &protected);

        // Per-page temp dir holding only this page's index.html — components
        // come exclusively from `component_sources`, which already includes
        // the template dir (for docs-search/docs-theme-toggle) plus any
        // configured component libraries.
        //
        // Name includes pid + per-rebuild nonce so two parallel page
        // builds (and successive rebuilds) can never collide and wipe
        // each other's in-progress files.
        let nonce = REBUILD_NONCE.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let page_tmp = std::env::temp_dir().join(format!(
            "webui-press-page-{}-{:x}-{nonce:x}",
            std::process::id(),
            fxhash(&page.path),
        ));
        if page_tmp.exists() {
            fs::remove_dir_all(&page_tmp).ok();
        }
        fs::create_dir_all(&page_tmp)
            .map_err(|e| Error::Io(format!("Cannot create page temp: {e}")))?;
        fs::write(page_tmp.join("index.html"), &page_html)
            .map_err(|e| Error::Io(format!("Cannot write page temp: {e}")))?;

        let build_result = webui::build(BuildOptions {
            app_dir: page_tmp.clone(),
            entry: "index.html".to_string(),
            plugin: Some(webui::Plugin::WebUI),
            components: component_sources.clone(),
            theme: token_file.clone(),
            ..BuildOptions::default()
        })
        .map_err(|e| Error::Build(format!("{}: {e}", page.path)))?;

        total_bytes.fetch_add(
            build_result.protocol_bytes.len(),
            std::sync::atomic::Ordering::Relaxed,
        );

        // Collect per-component CSS files (Link strategy emits one .css
        // per component used on the page). Multiple pages share components,
        // so we dedupe by filename in a shared map and write once after
        // the parallel pass completes.
        if !build_result.css_files.is_empty() {
            let mut css_map = component_css
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            for (name, content) in build_result.css_files {
                css_map.entry(name).or_insert(content);
            }
        }

        let mut themed_state;
        let render_state = if let Some(token_file) = token_file.as_ref() {
            themed_state = page.state.clone();
            inject_theme_tokens(&mut themed_state, token_file, &build_result.protocol.tokens)?;
            &themed_state
        } else {
            &page.state
        };

        let mut writer = StringWriter::with_capacity(8192);
        handler
            .render(
                &build_result.protocol,
                render_state,
                &RenderOptions::new("index.html", &page.path),
                &mut writer,
            )
            .map_err(|e| Error::Render(format!("{}: {e}", page.path)))?;

        // Restore the protected <pre> blocks via a single-pass scan.
        let html = restore_pre_blocks(&writer.buf, &pre_blocks);

        // Write directly inside the parallel closure.
        fs::write(&target, &html)
            .map_err(|e| Error::Io(format!("Cannot write {}: {e}", page.path)))?;

        fs::remove_dir_all(&page_tmp).ok();
        Ok(())
    })?;

    print_success(cache, &format!("Rendered {} pages", pages.len()));

    // Step 4: Search index. Strip rendered HTML to plain text and emit
    // an entry per non-home page. Done from-scratch every build.
    let search_path = site_dir.join("search-index.json");
    let search_entries: Vec<Value> = pages
        .iter()
        .filter(|p| !p.is_home)
        .map(|p| {
            let html = p.state["page"]["content"].as_str().unwrap_or("");
            let title = p.state["page"]["title"].as_str().unwrap_or("");
            build_search_entry(title, &p.path, html)
        })
        .collect();
    fs::write(
        &search_path,
        serde_json::to_string(&search_entries)
            .map_err(|e| Error::Build(format!("JSON error: {e}")))?,
    )
    .map_err(|e| Error::Io(e.to_string()))?;
    print_success(
        cache,
        &format!("Indexed {} pages for search", search_entries.len()),
    );

    // Step 5: Copy static public assets.
    let public_dir = Path::new(&config.public_dir);
    if public_dir.exists() {
        copy_dir(public_dir, &site_dir)?;
    }

    // Step 6: Generate 404 page
    let nav_val = pages
        .first()
        .map(|p| p.state["navigation"].clone())
        .unwrap_or_default();

    let footer_val = config
        .footer
        .as_ref()
        .map(|f| json_obj([("html", Value::String(f.html.clone()))]))
        .unwrap_or(Value::Null);

    let mut not_found_state = json_obj([
        (
            "site",
            json_obj([
                ("title", Value::String(config.site.title.clone())),
                ("base", Value::String(base_path.to_string())),
            ]),
        ),
        ("navigation", nav_val),
        ("sidebar", json_obj([("sections", Value::Array(vec![]))])),
        (
            "page",
            json_obj([
                ("title", Value::String("Page Not Found".to_string())),
                (
                    "description",
                    Value::String("The page you're looking for doesn't exist.".to_string()),
                ),
                ("content", Value::String(not_found_content.clone())),
                ("isHome", Value::Bool(false)),
                ("layout", Value::String("doc".to_string())),
            ]),
        ),
        ("hero", Value::Null),
        ("footer", footer_val),
        ("prev", Value::Null),
        ("next", Value::Null),
    ]);
    not_found_state["headTags"] = Value::String(head_injection);
    not_found_state["label"] = Value::String("Copy".to_string());
    not_found_state["icon"] = Value::String("🌙".to_string());

    let not_found_html = template_html.replace("{{{page.content}}}", &not_found_content);
    let nf_tmp = std::env::temp_dir().join(format!(
        "webui-press-404-{}-{:x}",
        std::process::id(),
        REBUILD_NONCE.fetch_add(1, std::sync::atomic::Ordering::Relaxed),
    ));
    if nf_tmp.exists() {
        fs::remove_dir_all(&nf_tmp).ok();
    }
    fs::create_dir_all(&nf_tmp).map_err(|e| Error::Io(e.to_string()))?;
    fs::write(nf_tmp.join("index.html"), &not_found_html).map_err(|e| Error::Io(e.to_string()))?;

    let nf_build = webui::build(BuildOptions {
        app_dir: nf_tmp.clone(),
        entry: "index.html".to_string(),
        plugin: Some(webui::Plugin::WebUI),
        components: component_sources.clone(),
        theme: token_file.clone(),
        ..BuildOptions::default()
    })
    .map_err(|e| Error::Build(format!("404 build failed: {e}")))?;

    if let Some(token_file) = token_file.as_ref() {
        inject_theme_tokens(&mut not_found_state, token_file, &nf_build.protocol.tokens)?;
    }

    // Fold the 404 page's component CSS into the shared map, then write
    // all per-component stylesheets at site root in one pass. The handler
    // emits relative hrefs like `<link rel="stylesheet" href="code-block.css">`;
    // the template's <base href="{site.base}"> resolves them against site root.
    {
        let mut css_map = component_css
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        for (name, content) in &nf_build.css_files {
            css_map
                .entry(name.clone())
                .or_insert_with(|| content.clone());
        }
    }

    let mut writer_404 = StringWriter::with_capacity(4096);
    handler
        .render(
            &nf_build.protocol,
            &not_found_state,
            &RenderOptions::new("index.html", &format!("{base_path}404/")),
            &mut writer_404,
        )
        .map_err(|e| Error::Render(format!("404: {e}")))?;

    fs::write(site_dir.join("404.html"), writer_404.buf).map_err(|e| Error::Io(e.to_string()))?;
    fs::remove_dir_all(&nf_tmp).ok();
    print_success(cache, "Generated 404 page");

    // Step 7: Write per-component stylesheets collected during the
    // parallel page builds (and the 404 build above). Sort by name for
    // deterministic output.
    let css_map_local = component_css
        .into_inner()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let mut css_names: Vec<&String> = css_map_local.keys().collect();
    css_names.sort();
    let css_count = css_names.len();
    for name in css_names {
        fs::write(site_dir.join(name), &css_map_local[name])
            .map_err(|e| Error::Io(format!("Cannot write {name}: {e}")))?;
    }
    if css_count > 0 {
        print_success(cache, &format!("Wrote {css_count} component stylesheets"));
    }

    // Step 8: Wait for the background bundling thread.
    let bundle_result = bundle_thread.join()?;
    print_success(
        cache,
        &format!(
            "Bundled {} component script{} into {} root script{} and {} page script group{}",
            bundle_result.component_count,
            if bundle_result.component_count == 1 {
                ""
            } else {
                "s"
            },
            usize::from(bundle_result.root_script.is_some()),
            if bundle_result.root_script.is_some() {
                ""
            } else {
                "s"
            },
            bundle_result.page_entry_count,
            if bundle_result.page_entry_count == 1 {
                ""
            } else {
                "s"
            }
        ),
    );

    // Step 8b: Inject page script <script> tags into rendered pages.
    if bundle_result.root_script.is_some() || !bundle_result.script_map.is_empty() {
        let mut linked_count = 0usize;
        for page in &pages {
            let page_rel_paths = if let Some(bundle_id) = page_bundle_ids.get(&page.path) {
                let rel_paths = bundle_result.script_map.get(bundle_id).ok_or_else(|| {
                    Error::Build(format!(
                        "Missing bundled script for page {} entry {}",
                        page.path, bundle_id
                    ))
                })?;
                if rel_paths.is_empty() {
                    None
                } else {
                    Some(rel_paths)
                }
            } else {
                None
            };
            if bundle_result.root_script.is_none() && page_rel_paths.is_none() {
                continue;
            }
            let page_dir = site_dir.join(page.path.strip_prefix(base_path).unwrap_or(&page.path));
            let target = page_dir.join("index.html");
            let mut html = fs::read_to_string(&target)
                .map_err(|e| Error::Io(format!("Cannot read {}: {e}", target.display())))?;

            if let Some(rel_path) = bundle_result.root_script.as_ref() {
                let tag = module_script_tag(base_path, rel_path);
                inject_script_tag(&mut html, &tag);
                linked_count += 1;
            }
            if let Some(rel_paths) = page_rel_paths {
                linked_count += inject_module_script_tags(&mut html, base_path, rel_paths);
            }

            fs::write(&target, &html)
                .map_err(|e| Error::Io(format!("Cannot rewrite {}: {e}", page.path)))?;
        }
        if let Some(bundle_id) = not_found_bundle_id {
            let rel_paths = bundle_result.script_map.get(&bundle_id).ok_or_else(|| {
                Error::Build(format!(
                    "Missing bundled script for 404 page entry {bundle_id}"
                ))
            })?;
            let target = site_dir.join("404.html");
            let mut html = fs::read_to_string(&target)
                .map_err(|e| Error::Io(format!("Cannot read {}: {e}", target.display())))?;
            if let Some(rel_path) = bundle_result.root_script.as_ref() {
                let tag = module_script_tag(base_path, rel_path);
                inject_script_tag(&mut html, &tag);
                linked_count += 1;
            }
            linked_count += inject_module_script_tags(&mut html, base_path, rel_paths);
            fs::write(&target, &html)
                .map_err(|e| Error::Io(format!("Cannot rewrite 404.html: {e}")))?;
        } else if let Some(rel_path) = bundle_result.root_script.as_ref() {
            let target = site_dir.join("404.html");
            let mut html = fs::read_to_string(&target)
                .map_err(|e| Error::Io(format!("Cannot read {}: {e}", target.display())))?;
            let tag = module_script_tag(base_path, rel_path);
            inject_script_tag(&mut html, &tag);
            fs::write(&target, &html)
                .map_err(|e| Error::Io(format!("Cannot rewrite 404.html: {e}")))?;
            linked_count += 1;
        }
        if linked_count > 0 {
            print_success(
                cache,
                &format!(
                    "Linked {} script tag{}",
                    linked_count,
                    if linked_count == 1 { "" } else { "s" }
                ),
            );
        }
    }

    let elapsed = start.elapsed();
    let total_bytes = total_bytes.load(std::sync::atomic::Ordering::Relaxed);
    print_finish(cache, &format!("Built in {:.1}s", elapsed.as_secs_f64()));

    Ok(BuildStats {
        pages: pages.len(),
        protocol_bytes: total_bytes,
    })
}

/// Recursively remove a directory, retrying briefly on `ENOTEMPTY`.
///
/// On macOS (and occasionally Linux), `fs::remove_dir_all` can fail
/// with `Directory not empty` when something writes a file into a
/// subdirectory between the moment we walked it and the moment we try
/// to `rmdir` it. Likely culprits during a dev-server rebuild:
///
///  - Spotlight / `mds` writing index hints,
///  - Finder dropping a `.DS_Store`,
///  - the user's editor reindexing the project,
///  - a back-to-back save firing another rebuild before the first one
///    finished cleanup (the worker coalesces, but a stray write into
///    `out_dir` from a parallel `cargo` or another tool is still
///    possible).
///
/// We retry up to a handful of times with short backoff. After that the
/// error propagates so the user sees a real failure rather than an
/// infinite loop.
fn remove_dir_all_retry(path: &Path) -> std::io::Result<()> {
    const MAX_ATTEMPTS: u32 = 5;
    let mut delay = std::time::Duration::from_millis(20);
    for attempt in 1..=MAX_ATTEMPTS {
        match fs::remove_dir_all(path) {
            Ok(()) => return Ok(()),
            Err(e) if attempt < MAX_ATTEMPTS && is_dir_not_empty(&e) => {
                std::thread::sleep(delay);
                delay = delay.saturating_mul(2);
            }
            Err(e) => return Err(e),
        }
    }
    // Loop guarantees a return on each iteration; this is unreachable
    // but `for` returning `()` makes the type-checker happy without it.
    Ok(())
}

/// Returns true if the error is `ENOTEMPTY` / `EEXIST` style "directory
/// not empty". On macOS this is errno 66; on Linux it's 39. We also
/// match by `ErrorKind` once Rust exposes one (currently `Other`).
fn is_dir_not_empty(e: &std::io::Error) -> bool {
    match e.raw_os_error() {
        Some(66) => true,  // macOS / BSD: ENOTEMPTY
        Some(39) => true,  // Linux: ENOTEMPTY
        Some(145) => true, // Windows: ERROR_DIR_NOT_EMPTY
        _ => false,
    }
}

/// Iteratively copy a directory tree (BFS via stack — no recursion).
fn copy_dir(src: &Path, dest: &Path) -> Result<()> {
    if !src.exists() {
        return Ok(());
    }
    fs::create_dir_all(dest).map_err(|e| Error::Io(e.to_string()))?;
    let mut stack: Vec<(std::path::PathBuf, std::path::PathBuf)> =
        vec![(src.to_path_buf(), dest.to_path_buf())];
    while let Some((s, d)) = stack.pop() {
        for entry in fs::read_dir(&s).map_err(|e| Error::Io(e.to_string()))? {
            let entry = entry.map_err(|e| Error::Io(e.to_string()))?;
            let dest_path = d.join(entry.file_name());
            let ft = entry.file_type().map_err(|e| Error::Io(e.to_string()))?;
            if ft.is_dir() {
                fs::create_dir_all(&dest_path).map_err(|e| Error::Io(e.to_string()))?;
                stack.push((entry.path(), dest_path));
            } else {
                fs::copy(entry.path(), &dest_path).map_err(|e| Error::Io(e.to_string()))?;
            }
        }
    }
    Ok(())
}

/// Truncate a string to at most `max` bytes at a valid UTF-8 boundary.
pub(crate) fn truncate_utf8(s: &str, max: usize) -> &str {
    if s.len() <= max {
        return s;
    }
    let mut end = max;
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    &s[..end]
}

#[derive(Default)]
struct SearchText {
    content: String,
    headings: Vec<SearchHeading>,
}

/// Heading metadata emitted into `search-index.json`.
///
/// The client search UI uses this to create separate, anchor-linked rows for
/// matching sections (`Page > Heading`) without re-parsing page HTML at runtime.
struct SearchHeading {
    text: String,
    anchor: String,
    level: u8,
}

/// Mutable heading capture state while scanning rendered HTML.
struct CurrentHeading {
    text: String,
    anchor: String,
    level: u8,
}

struct HtmlTag<'a> {
    name: &'a str,
    is_end: bool,
}

/// Build a single search-index entry for a page. Strips HTML tags,
/// decodes escaped text, captures headings for weighted ranking,
/// collapses whitespace, and truncates body content to 500 bytes.
pub(crate) fn build_search_entry(title: &str, path: &str, html: &str) -> Value {
    let search_text = extract_search_text(html);
    let content = normalize_search_text(&search_text.content);
    let truncated = truncate_utf8(&content, 500);
    let headings: Vec<Value> = search_text
        .headings
        .into_iter()
        .map(|heading| {
            json_obj([
                ("text", Value::String(heading.text)),
                ("anchor", Value::String(heading.anchor)),
                (
                    "level",
                    Value::Number(serde_json::Number::from(u64::from(heading.level))),
                ),
            ])
        })
        .collect();
    json_obj([
        ("title", Value::String(title.to_string())),
        ("path", Value::String(path.to_string())),
        ("content", Value::String(truncated.to_string())),
        ("headings", Value::Array(headings)),
    ])
}

fn extract_search_text(html: &str) -> SearchText {
    let mut text = SearchText {
        content: String::with_capacity(html.len() / 2),
        headings: Vec::with_capacity(8),
    };
    let mut current_heading: Option<CurrentHeading> = None;
    let mut skip_header_anchor = 0usize;
    let mut cursor = 0;

    while cursor < html.len() {
        let remaining = &html[cursor..];
        let Some(tag_offset) = remaining.find('<') else {
            push_search_text(
                &mut text,
                current_heading.as_mut(),
                skip_header_anchor,
                remaining,
            );
            break;
        };
        let text_end = cursor + tag_offset;
        push_search_text(
            &mut text,
            current_heading.as_mut(),
            skip_header_anchor,
            &html[cursor..text_end],
        );
        let tag_start = text_end + 1;
        let Some(tag_len) = html[tag_start..].find('>') else {
            push_search_text(
                &mut text,
                current_heading.as_mut(),
                skip_header_anchor,
                &html[text_end..],
            );
            break;
        };
        let tag_end = tag_start + tag_len;
        handle_search_tag(
            &html[tag_start..tag_end],
            &mut text,
            &mut current_heading,
            &mut skip_header_anchor,
        );
        cursor = tag_end + 1;
    }

    text
}

fn handle_search_tag(
    raw_tag: &str,
    text: &mut SearchText,
    current_heading: &mut Option<CurrentHeading>,
    skip_header_anchor: &mut usize,
) {
    if let Some(tag) = parse_html_tag(raw_tag) {
        if tag.is_end {
            handle_end_tag(tag.name, text, current_heading, skip_header_anchor);
        } else {
            handle_start_tag(tag.name, raw_tag, current_heading, skip_header_anchor);
        }
    }
    push_search_space(text, current_heading.as_mut(), *skip_header_anchor);
}

fn handle_start_tag(
    name: &str,
    raw_tag: &str,
    current_heading: &mut Option<CurrentHeading>,
    skip_header_anchor: &mut usize,
) {
    if let Some(level) = heading_level(name) {
        *current_heading = Some(CurrentHeading {
            text: String::with_capacity(64),
            anchor: attr_value(raw_tag, "id").unwrap_or("").to_string(),
            level,
        });
    } else if name.eq_ignore_ascii_case("a") && raw_tag.contains("header-anchor") {
        *skip_header_anchor += 1;
    }
}

fn handle_end_tag(
    name: &str,
    text: &mut SearchText,
    current_heading: &mut Option<CurrentHeading>,
    skip_header_anchor: &mut usize,
) {
    if name.eq_ignore_ascii_case("a") && *skip_header_anchor > 0 {
        *skip_header_anchor -= 1;
    }
    if heading_level(name).is_some() {
        if let Some(raw_heading) = current_heading.take() {
            let heading_text = normalize_search_text(&raw_heading.text);
            if !heading_text.is_empty() {
                text.headings.push(SearchHeading {
                    text: heading_text,
                    anchor: raw_heading.anchor,
                    level: raw_heading.level,
                });
            }
        }
    }
}

fn parse_html_tag(raw: &str) -> Option<HtmlTag<'_>> {
    let mut tag = raw.trim_start();
    if tag.starts_with('!') || tag.starts_with('?') {
        return None;
    }
    let is_end = tag.starts_with('/');
    if is_end {
        tag = tag[1..].trim_start();
    }

    let mut name_end = 0;
    for (idx, byte) in tag.bytes().enumerate() {
        if byte.is_ascii_alphanumeric() || byte == b'-' {
            name_end = idx + 1;
        } else {
            break;
        }
    }
    if name_end == 0 {
        return None;
    }
    Some(HtmlTag {
        name: &tag[..name_end],
        is_end,
    })
}

/// Return a heading level for `h1` through `h6`.
///
/// The scanner works on tag names from already-rendered markdown, so an ASCII
/// byte check is sufficient and avoids allocating a lowercased copy.
fn heading_level(name: &str) -> Option<u8> {
    let bytes = name.as_bytes();
    if bytes.len() == 2 && bytes[0].eq_ignore_ascii_case(&b'h') && (b'1'..=b'6').contains(&bytes[1])
    {
        Some(bytes[1] - b'0')
    } else {
        None
    }
}

/// Extract an HTML attribute value from a rendered start tag without a DOM
/// parser.
///
/// Search indexing runs once per page at build time, but it still avoids regex
/// and allocation-heavy parsing. This helper handles quoted and unquoted values
/// because heading IDs are generated as ordinary HTML attributes.
fn attr_value<'a>(raw_tag: &'a str, attr_name: &str) -> Option<&'a str> {
    let bytes = raw_tag.as_bytes();
    let mut cursor = 0;

    while cursor < bytes.len() {
        while bytes.get(cursor).is_some_and(u8::is_ascii_whitespace) {
            cursor += 1;
        }

        let name_start = cursor;
        while bytes.get(cursor).is_some_and(|b| is_attr_name_byte(*b)) {
            cursor += 1;
        }
        if name_start == cursor {
            cursor += 1;
            continue;
        }
        let name = &raw_tag[name_start..cursor];

        while bytes.get(cursor).is_some_and(u8::is_ascii_whitespace) {
            cursor += 1;
        }
        if bytes.get(cursor) != Some(&b'=') {
            continue;
        }
        cursor += 1;
        while bytes.get(cursor).is_some_and(u8::is_ascii_whitespace) {
            cursor += 1;
        }

        let (value_start, value_end) = attr_value_range(raw_tag, cursor);
        if name.eq_ignore_ascii_case(attr_name) {
            return Some(&raw_tag[value_start..value_end]);
        }
        cursor = value_end.saturating_add(1);
    }

    None
}

fn is_attr_name_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b':')
}

fn attr_value_range(raw_tag: &str, cursor: usize) -> (usize, usize) {
    let bytes = raw_tag.as_bytes();
    match bytes.get(cursor).copied() {
        Some(b'"' | b'\'') => {
            let quote = bytes[cursor];
            let value_start = cursor + 1;
            let value_end = raw_tag[value_start..]
                .find(char::from(quote))
                .map_or(raw_tag.len(), |offset| value_start + offset);
            (value_start, value_end)
        }
        Some(_) => {
            let value_start = cursor;
            let value_end = raw_tag[value_start..]
                .find(char::is_whitespace)
                .map_or(raw_tag.len(), |offset| value_start + offset);
            (value_start, value_end)
        }
        None => (raw_tag.len(), raw_tag.len()),
    }
}

fn push_search_text(
    text: &mut SearchText,
    current_heading: Option<&mut CurrentHeading>,
    skip_header_anchor: usize,
    value: &str,
) {
    if skip_header_anchor > 0 {
        return;
    }
    text.content.push_str(value);
    if let Some(heading) = current_heading {
        heading.text.push_str(value);
    }
}

fn push_search_space(
    text: &mut SearchText,
    current_heading: Option<&mut CurrentHeading>,
    skip_header_anchor: usize,
) {
    if skip_header_anchor > 0 {
        return;
    }
    text.content.push(' ');
    if let Some(heading) = current_heading {
        heading.text.push(' ');
    }
}

fn normalize_search_text(raw: &str) -> String {
    let decoded = html_escape::decode_html_entities(raw);
    let mut normalized = String::with_capacity(decoded.len());
    let mut first = true;
    for token in decoded.split_whitespace() {
        if !first {
            normalized.push(' ');
        }
        normalized.push_str(token);
        first = false;
    }
    normalized
}

fn is_component_html_file(path: &Path) -> bool {
    path.extension()
        .is_some_and(|extension| extension == "html")
        && path
            .file_stem()
            .and_then(|name| name.to_str())
            .is_some_and(|name| name.contains('-'))
}

/// Discover local component scripts from component sources.
///
/// A script is considered the client entry for a component when a
/// `tag-name.html` file has a sibling `tag-name.ts` file. npm component
/// sources are skipped here because package custom-element imports are
/// explicit page script responsibility.
fn discover_component_scripts(
    component_sources: &[String],
) -> Result<BTreeMap<String, ComponentScript>> {
    let mut scripts = BTreeMap::new();
    for source in component_sources {
        let root = Path::new(source);
        if !root.is_dir() {
            continue;
        }
        collect_component_scripts(root, &mut scripts)?;
    }
    Ok(scripts)
}

/// Collect component script pairs from a directory tree (iterative).
fn collect_component_scripts(
    dir: &Path,
    scripts: &mut BTreeMap<String, ComponentScript>,
) -> Result<()> {
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        let entries = fs::read_dir(&d)
            .map_err(|e| Error::Io(format!("Cannot read component dir {}: {e}", d.display())))?;
        for entry in entries {
            let entry = entry.map_err(|e| Error::Io(e.to_string()))?;
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
            } else if is_component_html_file(&path) {
                let Some(tag) = path.file_stem().and_then(|name| name.to_str()) else {
                    continue;
                };
                let script_path = path.with_extension("ts");
                if script_path.exists() {
                    let script_path = script_path.canonicalize().unwrap_or(script_path);
                    let html_content = fs::read_to_string(&path).map_err(|e| {
                        Error::Io(format!(
                            "Cannot read component template {}: {e}",
                            path.display()
                        ))
                    })?;
                    scripts.entry(tag.to_string()).or_insert(ComponentScript {
                        html_content,
                        script_path,
                    });
                }
            }
        }
    }
    Ok(())
}

fn collect_component_scripts_for_html(
    html_fragments: &[&str],
    component_scripts: &BTreeMap<String, ComponentScript>,
) -> Result<Vec<PathBuf>> {
    if component_scripts.is_empty() {
        return Ok(Vec::new());
    }

    let mut pending = Vec::new();
    for html in html_fragments {
        push_custom_element_tags(html, component_scripts, &mut pending);
    }

    let mut seen_tags = HashSet::with_capacity(pending.len());
    let mut script_paths = Vec::new();
    let mut cursor = 0;
    while cursor < pending.len() {
        let tag = pending[cursor].clone();
        cursor += 1;
        if !seen_tags.insert(tag.clone()) {
            continue;
        }
        let Some(component) = component_scripts.get(&tag) else {
            continue;
        };
        script_paths.push(component.script_path.clone());
        push_custom_element_tags(&component.html_content, component_scripts, &mut pending);
    }

    Ok(script_paths)
}

fn page_bundle_signature(
    component_scripts: &[PathBuf],
    explicit_scripts: &[ScriptSource],
    config_dir: &Path,
) -> String {
    let mut signature =
        String::with_capacity(component_scripts.len() * 96 + explicit_scripts.len() * 96);
    for path in component_scripts {
        signature.push_str("component:");
        signature.push_str(&path_for_js(path));
        signature.push('\n');
    }
    for source in explicit_scripts {
        match source {
            ScriptSource::Inline(code) => {
                signature.push_str("inline:");
                signature.push_str(&format!("{:x}", fxhash_bytes(code.as_bytes())));
                signature.push('\n');
            }
            ScriptSource::File(path) => {
                let resolved = config_dir.join(path);
                let resolved = resolved.canonicalize().unwrap_or(resolved);
                signature.push_str("file:");
                signature.push_str(&path_for_js(&resolved));
                signature.push('\n');
            }
        }
    }
    signature
}

fn push_custom_element_tags(
    html: &str,
    component_scripts: &BTreeMap<String, ComponentScript>,
    tags: &mut Vec<String>,
) {
    let bytes = html.as_bytes();
    let mut cursor = 0;
    while cursor < bytes.len() {
        let Some(rel) = html[cursor..].find('<') else {
            break;
        };
        let tag_start = cursor + rel;
        let name_start = tag_start + 1;
        let Some(&first) = bytes.get(name_start) else {
            break;
        };

        if first == b'!' {
            if bytes.get(name_start + 1) == Some(&b'-') && bytes.get(name_start + 2) == Some(&b'-')
            {
                if let Some(end_rel) = html[name_start + 3..].find("-->") {
                    cursor = name_start + 3 + end_rel + 3;
                    continue;
                }
                break;
            }
            cursor = name_start + 1;
            continue;
        }

        if matches!(first, b'/' | b'?') {
            cursor = name_start + 1;
            continue;
        }

        let mut name_end = name_start;
        let mut has_hyphen = false;
        while name_end < bytes.len() && is_tag_name_byte(bytes[name_end]) {
            if bytes[name_end] == b'-' {
                has_hyphen = true;
            }
            name_end += 1;
        }

        if name_end == name_start {
            cursor = name_start + 1;
            continue;
        }

        let tag = &html[name_start..name_end];
        if tag == "script" || tag == "style" {
            let close = if tag == "script" {
                "</script>"
            } else {
                "</style>"
            };
            if let Some(end_rel) = html[name_end..].find(close) {
                cursor = name_end + end_rel + close.len();
                continue;
            }
            break;
        }

        if has_hyphen && component_scripts.contains_key(tag) {
            tags.push(tag.to_string());
        }

        cursor = name_end;
    }
}

fn is_tag_name_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || byte == b'-'
}

/// Bundle result returned by [`bundle_assets`].
struct BundleResult {
    /// Root script shared by every page, when the template has browser code.
    root_script: Option<String>,
    /// Number of local component scripts imported by page entries.
    component_count: usize,
    /// Number of page-specific import groups bundled.
    page_entry_count: usize,
    /// Map from page-bundle ID to relative output paths.
    ///
    /// Import-only esbuild entry wrappers are flattened to the chunks they
    /// import so pages do not pay a request for a `page-N.js` file that only
    /// forwards to shared chunks.
    script_map: HashMap<usize, Vec<String>>,
}

/// Configuration for the [`bundle_assets`] function.
struct BundleOptions<'a> {
    site_dir: &'a Path,
    node_modules: &'a Path,
    root_bundle: Option<&'a RootBundleEntry>,
    page_bundles: &'a [PageBundleEntry],
    bundler_config: Option<&'a BundlerConfig>,
    dev_mode: bool,
    config_dir: &'a Path,
}

fn path_for_js(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

fn push_import_once(entry: &mut String, imports: &mut HashSet<String>, specifier: &str) {
    if imports.insert(specifier.to_string()) {
        entry.push_str("import \"");
        entry.push_str(specifier);
        entry.push_str("\";\n");
    }
}

fn push_external_args(
    args: &mut Vec<String>,
    external: &[String],
    aliases: &BTreeMap<String, String>,
) {
    for ext in external {
        if aliases.contains_key(ext.as_str()) {
            continue;
        }
        args.push(format!("--external:{ext}"));
    }
}

fn file_version(path: &Path) -> Result<String> {
    let bytes =
        fs::read(path).map_err(|e| Error::Io(format!("Cannot read {}: {e}", path.display())))?;
    Ok(format!("{:x}", fxhash_bytes(&bytes)))
}

fn versioned_asset_path(rel_path: &str, full_path: &Path) -> Result<String> {
    let version = file_version(full_path)?;
    Ok(format!("{rel_path}?v={version}"))
}

fn versioned_script_paths(site_dir: &Path, rel_paths: &[String]) -> Result<Vec<String>> {
    let mut versioned = Vec::with_capacity(rel_paths.len());
    for rel_path in rel_paths {
        let full_path = site_dir.join(rel_path);
        if !full_path.exists() {
            return Err(Error::Build(format!(
                "Bundled chunk output missing: {}",
                full_path.display()
            )));
        }
        versioned.push(versioned_asset_path(rel_path, &full_path)?);
    }
    Ok(versioned)
}

fn skip_js_whitespace(input: &str, mut cursor: usize) -> usize {
    let bytes = input.as_bytes();
    while cursor < bytes.len() && bytes[cursor].is_ascii_whitespace() {
        cursor += 1;
    }
    cursor
}

fn skip_js_trivia(input: &str, mut cursor: usize) -> usize {
    let bytes = input.as_bytes();
    loop {
        while cursor < bytes.len() && (bytes[cursor].is_ascii_whitespace() || bytes[cursor] == b';')
        {
            cursor += 1;
        }

        if cursor + 1 >= bytes.len() || bytes[cursor] != b'/' {
            return cursor;
        }

        if bytes[cursor + 1] == b'/' {
            cursor += 2;
            while cursor < bytes.len() && bytes[cursor] != b'\n' {
                cursor += 1;
            }
            continue;
        }

        if bytes[cursor + 1] == b'*' {
            if let Some(end_rel) = input[cursor + 2..].find("*/") {
                cursor += end_rel + 4;
                continue;
            }
            return input.len();
        }

        return cursor;
    }
}

fn quoted_js_string_at(input: &str, cursor: usize) -> Option<(&str, usize)> {
    let bytes = input.as_bytes();
    let quote = *bytes.get(cursor)?;
    if quote != b'"' && quote != b'\'' {
        return None;
    }

    let start = cursor + 1;
    let mut end = start;
    while end < bytes.len() {
        if bytes[end] == b'\\' {
            end += 2;
            continue;
        }
        if bytes[end] == quote {
            return Some((&input[start..end], end + 1));
        }
        end += 1;
    }
    None
}

fn next_leading_import_statement<'a>(input: &'a str, cursor: &mut usize) -> Option<&'a str> {
    let start = skip_js_trivia(input, *cursor);
    if !input[start..].starts_with("import") {
        *cursor = start;
        return None;
    }

    let after_import = start + "import".len();
    if let Some(byte) = input.as_bytes().get(after_import) {
        if byte.is_ascii_alphanumeric() || *byte == b'_' || *byte == b'$' {
            *cursor = start;
            return None;
        }
    }

    let end_rel = input[after_import..].find(';')?;
    let end = after_import + end_rel;
    *cursor = end + 1;
    Some(&input[start..=end])
}

fn side_effect_import_specifier(statement: &str) -> Option<&str> {
    let cursor = skip_js_whitespace(statement, "import".len());
    quoted_js_string_at(statement, cursor).map(|(specifier, _)| specifier)
}

fn static_import_specifier(statement: &str) -> Option<&str> {
    if let Some(specifier) = side_effect_import_specifier(statement) {
        return Some(specifier);
    }

    let from_pos = statement.rfind("from")?;
    let cursor = skip_js_whitespace(statement, from_pos + "from".len());
    quoted_js_string_at(statement, cursor).map(|(specifier, _)| specifier)
}

fn resolve_js_import_path(importer_rel: &str, specifier: &str) -> Option<String> {
    if !specifier.starts_with("./") && !specifier.starts_with("../") {
        return None;
    }

    let parent = importer_rel
        .rsplit_once('/')
        .map_or("", |(parent, _)| parent);
    let mut parts = Vec::new();
    for part in parent.split('/') {
        if !part.is_empty() {
            parts.push(part);
        }
    }
    for part in specifier.split('/') {
        match part {
            "" | "." => {}
            ".." => {
                parts.pop();
            }
            _ => parts.push(part),
        }
    }
    Some(parts.join("/"))
}

fn collect_leading_relative_imports(
    input: &str,
    importer_rel: &str,
    imports: &mut HashSet<String>,
) {
    let mut cursor = 0;
    while let Some(statement) = next_leading_import_statement(input, &mut cursor) {
        if let Some(specifier) = static_import_specifier(statement) {
            if let Some(rel_path) = resolve_js_import_path(importer_rel, specifier) {
                imports.insert(rel_path);
            }
        }
    }
}

fn import_only_relative_imports(input: &str, importer_rel: &str) -> Option<Vec<String>> {
    let mut cursor = 0;
    let mut imports = Vec::new();
    loop {
        cursor = skip_js_trivia(input, cursor);
        if cursor >= input.len() {
            return if imports.is_empty() {
                None
            } else {
                Some(imports)
            };
        }

        let statement = next_leading_import_statement(input, &mut cursor)?;
        let specifier = side_effect_import_specifier(statement)?;
        let rel_path = resolve_js_import_path(importer_rel, specifier)?;
        imports.push(rel_path);
    }
}

fn prune_redundant_imports(
    site_dir: &Path,
    root_imports: &HashSet<String>,
    imports: &[String],
) -> Result<Vec<String>> {
    let mut transitive_imports = HashSet::new();
    for rel_path in imports {
        let full_path = site_dir.join(rel_path);
        if !full_path.exists() {
            return Err(Error::Build(format!(
                "Bundled chunk output missing: {}",
                full_path.display()
            )));
        }
        let contents = fs::read_to_string(&full_path)
            .map_err(|e| Error::Io(format!("Cannot read {}: {e}", full_path.display())))?;
        collect_leading_relative_imports(&contents, rel_path, &mut transitive_imports);
    }

    let mut seen = HashSet::with_capacity(imports.len());
    let mut retained = Vec::with_capacity(imports.len());
    for rel_path in imports {
        if root_imports.contains(rel_path) || transitive_imports.contains(rel_path) {
            continue;
        }
        if seen.insert(rel_path.clone()) {
            retained.push(rel_path.clone());
        }
    }
    Ok(retained)
}

fn page_script_paths(
    site_dir: &Path,
    output_file: &str,
    full_path: &Path,
    root_imports: &HashSet<String>,
) -> Result<Vec<String>> {
    let contents = fs::read_to_string(full_path)
        .map_err(|e| Error::Io(format!("Cannot read {}: {e}", full_path.display())))?;
    if let Some(imports) = import_only_relative_imports(&contents, output_file) {
        let rel_paths = prune_redundant_imports(site_dir, root_imports, &imports)?;
        fs::remove_file(full_path).map_err(|e| {
            Error::Io(format!(
                "Cannot remove import-only script wrapper {}: {e}",
                full_path.display()
            ))
        })?;
        versioned_script_paths(site_dir, &rel_paths)
    } else {
        Ok(vec![versioned_asset_path(output_file, full_path)?])
    }
}

fn next_rebuild_nonce_hex() -> String {
    format!(
        "{:x}",
        REBUILD_NONCE.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    )
}

fn resolve_node_modules(config_dir: &Path) -> Result<PathBuf> {
    let start = config_dir.canonicalize().map_err(|e| {
        Error::Build(format!(
            "Cannot resolve config directory {} while locating node_modules: {e}",
            config_dir.display()
        ))
    })?;

    for dir in start.ancestors() {
        let node_modules = dir.join("node_modules");
        if node_modules.exists() {
            return Ok(node_modules);
        }
    }

    Err(Error::Build(format!(
        "Cannot find node_modules in {} or its ancestors. Run pnpm install from the docs project root before building bundled scripts.",
        start.display()
    )))
}

fn default_framework_alias(node_modules: &Path) -> Option<PathBuf> {
    let pkg = node_modules.join("@microsoft").join("webui-framework");
    let dist = pkg.join("dist").join("index.js");
    if dist.exists() {
        return Some(dist);
    }

    let src = pkg.join("src").join("index.ts");
    if src.exists() {
        return Some(src);
    }

    None
}

fn normalized_alias_target(config_dir: &Path, target: &str) -> String {
    let path = Path::new(target);
    if path.is_absolute() {
        return target.replace('\\', "/");
    }

    if target.starts_with('.') {
        return path_for_js(&config_dir.join(path));
    }

    target.replace('\\', "/")
}

fn build_aliases(opts: &BundleOptions<'_>) -> BTreeMap<String, String> {
    let mut aliases: BTreeMap<String, String> = BTreeMap::new();
    if let Some(path) = default_framework_alias(opts.node_modules) {
        aliases.insert("@microsoft/webui-framework".to_string(), path_for_js(&path));
    }

    if let Some(cfg) = opts.bundler_config {
        for (from, to) in &cfg.alias {
            aliases.insert(from.clone(), normalized_alias_target(opts.config_dir, to));
        }
    }

    aliases
}

fn push_alias_args(args: &mut Vec<String>, aliases: &BTreeMap<String, String>) {
    for (from, to) in aliases {
        args.push(format!("--alias:{from}={to}"));
    }
}

fn push_define_args(args: &mut Vec<String>, cfg: &BundlerConfig) {
    for (key, value) in &cfg.define {
        args.push(format!("--define:{key}={value}"));
    }
}

fn esbuild_args(
    opts: &BundleOptions<'_>,
    entry_files: &[(String, PathBuf)],
    bundle_tmp: &Path,
) -> Vec<String> {
    let aliases = build_aliases(opts);
    let target = opts
        .bundler_config
        .and_then(|cfg| cfg.target.as_deref())
        .unwrap_or("es2022");
    let mut args = Vec::with_capacity(14 + entry_files.len());
    args.push("--bundle".to_string());
    args.push("--platform=browser".to_string());
    args.push("--format=esm".to_string());
    args.push("--splitting".to_string());
    args.push(format!("--target={target}"));
    args.push(format!("--outdir={}", path_for_js(opts.site_dir)));
    args.push(format!("--outbase={}", path_for_js(bundle_tmp)));
    args.push("--entry-names=[dir]/[name]".to_string());
    args.push("--chunk-names=assets/[name]-[hash]".to_string());
    args.push("--loader:.html=text".to_string());
    args.push("--loader:.css=text".to_string());
    args.push("--log-level=warning".to_string());
    if !opts.dev_mode {
        args.push("--minify".to_string());
    }
    if let Some(cfg) = opts.bundler_config {
        push_external_args(&mut args, &cfg.external, &aliases);
        push_define_args(&mut args, cfg);
    }
    push_alias_args(&mut args, &aliases);
    for (_, path) in entry_files {
        args.push(path_for_js(path));
    }
    args
}

/// Bundle page-scoped scripts via esbuild.
///
/// Uses a single esbuild invocation with one virtual entry per page for
/// optimal code splitting. Each page entry imports local component scripts
/// discovered from that page's HTML plus any explicit `<script bundle>` or
/// `scriptFile` sources.
///
/// Returns a [`BundleResult`] with the component script count and a mapping
/// from page-bundle IDs to their output file paths.
fn bundle_assets(opts: &BundleOptions<'_>) -> Result<BundleResult> {
    if opts.root_bundle.is_none() && opts.page_bundles.is_empty() {
        return Ok(BundleResult {
            root_script: None,
            component_count: 0,
            page_entry_count: 0,
            script_map: HashMap::new(),
        });
    }

    // Create a temp directory for the bundler entry files.
    let nonce = next_rebuild_nonce_hex();
    let bundle_tmp =
        std::env::temp_dir().join(format!("webui-press-bundle-{}-{nonce}", std::process::id(),));
    if bundle_tmp.exists() {
        fs::remove_dir_all(&bundle_tmp).ok();
    }
    fs::create_dir_all(&bundle_tmp)
        .map_err(|e| Error::Build(format!("Cannot create bundle temp dir: {e}")))?;

    let assets_dir = opts.site_dir.join("assets");
    fs::create_dir_all(&assets_dir)
        .map_err(|e| Error::Io(format!("Cannot create assets dir: {e}")))?;

    let mut entry_files: Vec<(String, std::path::PathBuf)> = Vec::new();
    let mut component_imports = HashSet::new();

    if let Some(root) = opts.root_bundle {
        let entry_path = bundle_tmp.join("index.ts");
        let mut entry = String::with_capacity(32 + root.component_scripts.len() * 80);
        entry.push_str("// Root template script\n");
        let mut imports = HashSet::with_capacity(
            root.component_scripts.len() + usize::from(root.script_path.is_some()),
        );
        if let Some(path) = root.script_path.as_ref() {
            let specifier = path_for_js(path);
            push_import_once(&mut entry, &mut imports, &specifier);
        }
        for path in &root.component_scripts {
            component_imports.insert(path.clone());
            let specifier = path_for_js(path);
            push_import_once(&mut entry, &mut imports, &specifier);
        }
        fs::write(&entry_path, &entry)
            .map_err(|e| Error::Build(format!("Cannot write root script entry: {e}")))?;
        entry_files.push(("index".to_string(), entry_path));
    }

    // Write one virtual entry per page. Inline scripts are written as sibling
    // modules and imported, preserving module scope when a page has multiple
    // bundled script tags.
    for bundle in opts.page_bundles {
        let entry_name = format!("assets/page-{}", bundle.id);
        let entry_path = bundle_tmp.join(format!("{entry_name}.ts"));
        if let Some(parent) = entry_path.parent() {
            fs::create_dir_all(parent)
                .map_err(|e| Error::Build(format!("Cannot create script entry dir: {e}")))?;
        }

        let mut entry = String::with_capacity(
            32 + bundle.page_path.len()
                + (bundle.component_scripts.len() + bundle.explicit_scripts.len()) * 80,
        );
        entry.push_str("// Page: ");
        entry.push_str(&bundle.page_path);
        entry.push('\n');
        let mut imports =
            HashSet::with_capacity(bundle.component_scripts.len() + bundle.explicit_scripts.len());

        for path in &bundle.component_scripts {
            component_imports.insert(path.clone());
            let specifier = path_for_js(path);
            push_import_once(&mut entry, &mut imports, &specifier);
        }

        for (idx, source) in bundle.explicit_scripts.iter().enumerate() {
            match source {
                ScriptSource::Inline(code) => {
                    let inline_path =
                        bundle_tmp.join(format!("inline/page-{}-{idx}.ts", bundle.id));
                    if let Some(parent) = inline_path.parent() {
                        fs::create_dir_all(parent).map_err(|e| {
                            Error::Build(format!("Cannot create inline script dir: {e}"))
                        })?;
                    }
                    fs::write(&inline_path, code)
                        .map_err(|e| Error::Build(format!("Cannot write inline script: {e}")))?;
                    let specifier = path_for_js(&inline_path);
                    push_import_once(&mut entry, &mut imports, &specifier);
                }
                ScriptSource::File(path) => {
                    // Resolve src path relative to config_dir and canonicalize
                    // to an absolute path so the entry file (in a temp dir)
                    // can resolve the import.
                    let resolved = opts.config_dir.join(path);
                    let abs_path = resolved.canonicalize().unwrap_or(resolved);
                    let specifier = path_for_js(&abs_path);
                    push_import_once(&mut entry, &mut imports, &specifier);
                }
            }
        }

        fs::write(&entry_path, &entry)
            .map_err(|e| Error::Build(format!("Cannot write script entry: {e}")))?;
        entry_files.push((entry_name, entry_path));
    }

    let args = esbuild_args(opts, &entry_files, &bundle_tmp);
    let esbuild_bin = esbuild_command(opts.node_modules);

    let output = std::process::Command::new(&esbuild_bin)
        .args(&args)
        .env("NODE_PATH", opts.node_modules)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .output()
        .map_err(|e| Error::Build(format!("esbuild failed to start: {e}")))?;

    let stderr = String::from_utf8_lossy(&output.stderr);
    if !output.status.success() {
        return Err(Error::Build(format!("esbuild error: {stderr}")));
    }

    let mut root_imports = HashSet::new();
    let root_script = if opts.root_bundle.is_some() {
        let output_file = "index.js";
        let full_path = opts.site_dir.join(output_file);
        if full_path.exists() {
            let contents = fs::read_to_string(&full_path)
                .map_err(|e| Error::Io(format!("Cannot read {}: {e}", full_path.display())))?;
            collect_leading_relative_imports(&contents, output_file, &mut root_imports);
            Some(versioned_asset_path(output_file, &full_path)?)
        } else {
            return Err(Error::Build(format!(
                "Bundled root script output missing: {}",
                full_path.display()
            )));
        }
    } else {
        None
    };

    // Build script_map: find output files for page-script entries.
    let mut script_map = HashMap::with_capacity(opts.page_bundles.len());
    for bundle in opts.page_bundles {
        let entry_name = format!("page-{}", bundle.id);
        // esbuild outputs entry chunks as `{entry_name}.js` in the output dir.
        let output_file = format!("assets/{entry_name}.js");
        let full_path = opts.site_dir.join(&output_file);
        if full_path.exists() {
            script_map.insert(
                bundle.id,
                page_script_paths(opts.site_dir, &output_file, &full_path, &root_imports)?,
            );
        } else {
            return Err(Error::Build(format!(
                "Bundled script output missing: {}",
                full_path.display()
            )));
        }
    }

    // Clean up temp dir.
    fs::remove_dir_all(&bundle_tmp).ok();

    Ok(BundleResult {
        root_script,
        component_count: component_imports.len(),
        page_entry_count: opts.page_bundles.len(),
        script_map,
    })
}

/// Resolve the esbuild binary path from node_modules.
fn esbuild_command(node_modules: &Path) -> std::path::PathBuf {
    let binary = if cfg!(windows) {
        "esbuild.cmd"
    } else {
        "esbuild"
    };
    if let Some(project_dir) = node_modules.parent() {
        for dir in project_dir.ancestors() {
            let candidate = dir.join("node_modules").join(".bin").join(binary);
            if candidate.exists() {
                return candidate;
            }
        }
    }
    std::path::PathBuf::from(binary)
}

const PRE_BLOCK_MARKER_PREFIX: &str = "<span data-webui-press-pre-block=\"";
const PRE_BLOCK_MARKER_SUFFIX: &str = "\"></span>";

/// Extract `<script type="module" bundle>` and `<script type="module" bundle src="...">` tags
/// from page content HTML. Returns the modified content (with those tags removed)
/// and the extracted script sources.
///
/// The scanner is iterative and avoids regex (per project rules). It looks for
/// `<script` tags containing the `bundle` attribute.
fn extract_bundle_scripts(content: &str) -> (String, Vec<ScriptSource>) {
    let mut scripts = Vec::new();
    let mut out = String::with_capacity(content.len());
    let mut cursor = 0;
    let bytes = content.as_bytes();

    while cursor < bytes.len() {
        // Find next <script (case-sensitive — HTML from our pipeline is always lowercase).
        let Some(rel) = content[cursor..].find("<script") else {
            break;
        };
        let tag_start = cursor + rel;
        let after_tag = tag_start + 7; // len("<script")

        // Verify boundary: next char must be ' ', '\t', '\n', '\r', or '>'.
        match bytes.get(after_tag) {
            Some(b' ' | b'\t' | b'\n' | b'\r' | b'>') => {}
            _ => {
                out.push_str(&content[cursor..after_tag]);
                cursor = after_tag;
                continue;
            }
        }

        // Find the end of the opening tag '>'
        let Some(gt_rel) = content[after_tag..].find('>') else {
            break;
        };
        let gt_pos = after_tag + gt_rel;
        let attrs_region = &content[after_tag..gt_pos];

        // Check for `bundle` attribute (space-separated, could be anywhere).
        if !has_bundle_attr(attrs_region) {
            out.push_str(&content[cursor..gt_pos + 1]);
            cursor = gt_pos + 1;
            continue;
        }

        // Find closing </script> tag.
        let Some(close_rel) = content[gt_pos + 1..].find("</script>") else {
            // Malformed — pass through the rest.
            break;
        };
        let close_start = gt_pos + 1 + close_rel;
        let close_end = close_start + "</script>".len();

        // Extract source info.
        let src_attr = extract_src_attr(attrs_region);
        let inline_body = &content[gt_pos + 1..close_start];

        let source = if let Some(src) = src_attr {
            ScriptSource::File(src.to_string())
        } else if !inline_body.trim().is_empty() {
            ScriptSource::Inline(inline_body.to_string())
        } else {
            // Empty script with no src — skip it.
            out.push_str(&content[cursor..close_end]);
            cursor = close_end;
            continue;
        };

        scripts.push(source);

        // Drop the original bundled script tag. The page's generated virtual
        // entry is injected once near the end of the rendered document.
        out.push_str(&content[cursor..tag_start]);
        cursor = close_end;
    }

    out.push_str(&content[cursor..]);
    (out, scripts)
}

/// Check if the attributes region contains a `bundle` attribute name.
fn has_bundle_attr(attrs: &str) -> bool {
    let bytes = attrs.as_bytes();
    let target = b"bundle";
    let mut i = 0;

    while i < bytes.len() {
        while i < bytes.len() && matches!(bytes[i], b' ' | b'\t' | b'\n' | b'\r' | b'/') {
            i += 1;
        }
        if i >= bytes.len() {
            break;
        }

        let name_start = i;
        while i < bytes.len()
            && !matches!(bytes[i], b' ' | b'\t' | b'\n' | b'\r' | b'=' | b'/' | b'>')
        {
            i += 1;
        }
        if name_start == i {
            i += 1;
            continue;
        }

        if i - name_start == target.len() && &bytes[name_start..i] == target {
            return true;
        }

        while i < bytes.len() && matches!(bytes[i], b' ' | b'\t' | b'\n' | b'\r') {
            i += 1;
        }

        if i < bytes.len() && bytes[i] == b'=' {
            i += 1;
            while i < bytes.len() && matches!(bytes[i], b' ' | b'\t' | b'\n' | b'\r') {
                i += 1;
            }
            if i < bytes.len() && matches!(bytes[i], b'"' | b'\'') {
                let quote = bytes[i];
                i += 1;
                while i < bytes.len() && bytes[i] != quote {
                    i += 1;
                }
                if i < bytes.len() {
                    i += 1;
                }
            } else {
                while i < bytes.len() && !matches!(bytes[i], b' ' | b'\t' | b'\n' | b'\r') {
                    i += 1;
                }
            }
        }
    }

    false
}

/// Extract the value of a `src="..."` or `src='...'` attribute from an attrs region.
fn extract_src_attr(attrs: &str) -> Option<&str> {
    let bytes = attrs.as_bytes();
    let mut i = 0;
    while i + 4 <= bytes.len() {
        if let Some(rel) = attrs[i..].find("src") {
            let pos = i + rel;
            // Verify boundary before: space or start.
            let before_ok = pos == 0 || matches!(bytes[pos - 1], b' ' | b'\t' | b'\n' | b'\r');
            let after_src = pos + 3;
            if !before_ok || after_src >= bytes.len() {
                i = pos + 1;
                continue;
            }

            // Skip whitespace and '='.
            let mut eq = after_src;
            while eq < bytes.len() && matches!(bytes[eq], b' ' | b'\t') {
                eq += 1;
            }
            if eq >= bytes.len() || bytes[eq] != b'=' {
                i = pos + 1;
                continue;
            }
            eq += 1;
            while eq < bytes.len() && matches!(bytes[eq], b' ' | b'\t') {
                eq += 1;
            }

            // Read quoted value.
            if eq >= bytes.len() {
                i = pos + 1;
                continue;
            }
            let quote = bytes[eq];
            if quote != b'"' && quote != b'\'' {
                i = pos + 1;
                continue;
            }
            let val_start = eq + 1;
            if let Some(val_end_rel) = attrs[val_start..].find(quote as char) {
                return Some(&attrs[val_start..val_start + val_end_rel]);
            }
            i = pos + 1;
        } else {
            break;
        }
    }
    None
}

fn module_script_tag(base_path: &str, rel_path: &str) -> String {
    let mut tag = String::with_capacity(base_path.len() + rel_path.len() + 40);
    tag.push_str("\n<script type=\"module\" src=\"");
    if base_path.is_empty() || base_path == "/" {
        tag.push('/');
        tag.push_str(rel_path.trim_start_matches('/'));
    } else {
        tag.push_str(base_path.trim_end_matches('/'));
        tag.push('/');
        tag.push_str(rel_path.trim_start_matches('/'));
    }
    tag.push_str("\"></script>");
    tag
}

fn inject_module_script_tags(html: &mut String, base_path: &str, rel_paths: &[String]) -> usize {
    let mut count = 0;
    for rel_path in rel_paths {
        let tag = module_script_tag(base_path, rel_path);
        inject_script_tag(html, &tag);
        count += 1;
    }
    count
}

fn inject_script_tag(html: &mut String, tag: &str) {
    if let Some(pos) = html.rfind("</body>") {
        html.insert_str(pos, tag);
    } else {
        html.push_str(tag);
    }
}

/// Replace `<pre …>…</pre>` blocks with placeholder elements so the WebUI
/// HTML parser does not normalize whitespace inside them. Returns the
/// modified string and the original blocks (in order) for restoration
/// after rendering.
fn protect_pre_blocks(content: &str) -> (String, Vec<String>) {
    use std::fmt::Write as _;
    let mut blocks: Vec<String> = Vec::new();
    let mut out = String::with_capacity(content.len());
    let mut cursor = 0;
    while let Some(rel_start) = find_pre_open(&content[cursor..]) {
        let start = cursor + rel_start;
        if let Some(rel_end) = content[start..].find("</pre>") {
            let end = start + rel_end + "</pre>".len();
            out.push_str(&content[cursor..start]);
            out.push_str(PRE_BLOCK_MARKER_PREFIX);
            // write! into existing buffer — avoids `format!` allocation per block.
            let _ = write!(&mut out, "{}", blocks.len());
            out.push_str(PRE_BLOCK_MARKER_SUFFIX);
            blocks.push(content[start..end].to_string());
            cursor = end;
        } else {
            break;
        }
    }
    out.push_str(&content[cursor..]);
    (out, blocks)
}

/// Find the next opening `<pre` tag where the next byte is one of `>`, ` `,
/// `\t`, `\n`, or `\r`. Avoids matching `<presentation`, `<pretend`, etc.
fn find_pre_open(s: &str) -> Option<usize> {
    let bytes = s.as_bytes();
    let mut i = 0;
    while let Some(rel) = s[i..].find("<pre") {
        let pos = i + rel;
        let after = pos + 4;
        match bytes.get(after) {
            Some(b'>' | b' ' | b'\t' | b'\n' | b'\r') => return Some(pos),
            _ => i = after,
        }
    }
    None
}

/// Single-pass restoration of pre-block placeholder elements to their
/// original content. Faster than calling `String::replace` once per block.
fn restore_pre_blocks(html: &str, blocks: &[String]) -> String {
    if blocks.is_empty() {
        return html.to_string();
    }
    let extra: usize = blocks.iter().map(|b| b.len()).sum();
    let mut out = String::with_capacity(html.len() + extra);
    let mut cursor = 0;
    while let Some(rel) = html[cursor..].find(PRE_BLOCK_MARKER_PREFIX) {
        let p = cursor + rel;
        out.push_str(&html[cursor..p]);
        let after = p + PRE_BLOCK_MARKER_PREFIX.len();
        if let Some(end_rel) = html[after..].find(PRE_BLOCK_MARKER_SUFFIX) {
            let num_str = &html[after..after + end_rel];
            if let Ok(idx) = num_str.parse::<usize>() {
                if let Some(block) = blocks.get(idx) {
                    out.push_str(block);
                    cursor = after + end_rel + PRE_BLOCK_MARKER_SUFFIX.len();
                    continue;
                }
            }
            // Unknown placeholder — keep verbatim.
            out.push_str(&html[p..after + end_rel + PRE_BLOCK_MARKER_SUFFIX.len()]);
            cursor = after + end_rel + PRE_BLOCK_MARKER_SUFFIX.len();
        } else {
            out.push_str(&html[p..]);
            return out;
        }
    }
    out.push_str(&html[cursor..]);
    out
}

/// Cheap deterministic hash for building unique temp directory names.
fn fxhash(s: &str) -> u64 {
    fxhash_bytes(s.as_bytes())
}

fn fxhash_bytes(bytes: &[u8]) -> u64 {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for &b in bytes {
        hash ^= u64::from(b);
        hash = hash.wrapping_mul(0x100_0000_01b3);
    }
    hash
}

#[cfg(test)]
mod tests {
    use super::*;

    type TestResult = std::result::Result<(), Box<dyn std::error::Error>>;

    #[test]
    fn esbuild_command_resolves_from_node_modules() {
        let tmp = std::env::temp_dir().join("webui-press-esbuild-test");
        let bin_dir = tmp.join("node_modules/.bin");
        fs::create_dir_all(&bin_dir).unwrap();
        let bin_path = if cfg!(windows) {
            bin_dir.join("esbuild.cmd")
        } else {
            bin_dir.join("esbuild")
        };
        fs::write(&bin_path, "").unwrap();
        let resolved = esbuild_command(&tmp.join("node_modules"));
        assert_eq!(resolved, bin_path);
        fs::remove_dir_all(&tmp).ok();
    }

    #[test]
    fn push_external_args_filters_aliased_packages() {
        let external = vec![
            "@microsoft/webui-framework".to_string(),
            "cdn-only-package".to_string(),
        ];
        let mut aliases = BTreeMap::new();
        aliases.insert(
            "@microsoft/webui-framework".to_string(),
            "/repo/packages/webui-framework/dist/index.js".to_string(),
        );
        let mut args = Vec::new();

        push_external_args(&mut args, &external, &aliases);

        assert!(!args
            .iter()
            .any(|arg| arg.contains("@microsoft/webui-framework")));
        assert!(args.contains(&"--external:cdn-only-package".to_string()));
    }

    #[test]
    fn config_component_source_preserves_npm_packages() {
        let cwd = Path::new("project");

        assert_eq!(resolve_config_component_source("@mai-ui", cwd), "@mai-ui");
        assert_eq!(
            resolve_config_component_source("@mai-ui/button", cwd),
            "@mai-ui/button"
        );
        assert_eq!(
            resolve_config_component_source("plain-widget", cwd),
            "plain-widget"
        );
    }

    #[test]
    fn config_component_source_resolves_local_paths() {
        let cwd = Path::new("project");

        assert_eq!(
            std::path::PathBuf::from(resolve_config_component_source("./components", cwd)),
            cwd.join("./components")
        );
    }

    // --- truncate_utf8 ---------------------------------------------------

    #[test]
    fn truncate_utf8_short_string_unchanged() {
        assert_eq!(truncate_utf8("hello", 100), "hello");
    }

    #[test]
    fn truncate_utf8_ascii_at_exact_boundary() {
        assert_eq!(truncate_utf8("hello world", 5), "hello");
    }

    #[test]
    fn truncate_utf8_steps_back_off_multibyte_boundary() {
        // "é" is two bytes (0xC3 0xA9). Cutting at 1 byte must step back to 0.
        let s = "é-suffix";
        let out = truncate_utf8(s, 1);
        assert!(s.is_char_boundary(out.len()), "out.len()={}", out.len());
        assert_eq!(out, "");
    }

    #[test]
    fn truncate_utf8_keeps_multibyte_when_room_allows() {
        let s = "café";
        let out = truncate_utf8(s, 5); // "café" is 5 bytes total
        assert_eq!(out, "café");
    }

    // --- protect_pre_blocks / find_pre_open ------------------------------

    #[test]
    fn protect_pre_blocks_with_attrs() {
        let input = r#"<p>before</p><pre class="hljs">code</pre><p>after</p>"#;
        let (out, blocks) = protect_pre_blocks(input);
        assert_eq!(blocks.len(), 1);
        assert!(out.contains(r#"<span data-webui-press-pre-block="0"></span>"#));
        assert!(!out.contains("<pre"));
        assert_eq!(blocks[0], r#"<pre class="hljs">code</pre>"#);
    }

    #[test]
    fn protect_pre_blocks_bare_open_tag() {
        // Was previously missed because we only matched "<pre " (with space).
        let input = "<pre>code</pre>";
        let (out, blocks) = protect_pre_blocks(input);
        assert_eq!(blocks.len(), 1, "got out={out:?} blocks={blocks:?}");
        assert_eq!(blocks[0], "<pre>code</pre>");
    }

    #[test]
    fn protect_pre_blocks_does_not_match_presentation_tag() {
        // <presentation> must NOT be treated as a pre block.
        let input = "<presentation>x</presentation>";
        let (_out, blocks) = protect_pre_blocks(input);
        assert_eq!(blocks.len(), 0);
    }

    #[test]
    fn protect_then_restore_round_trip() {
        let input = "before <pre>a</pre> mid <pre class=\"x\">b</pre> end";
        let (protected, blocks) = protect_pre_blocks(input);
        let restored = restore_pre_blocks(&protected, &blocks);
        assert_eq!(restored, input);
    }

    #[test]
    fn restore_pre_blocks_no_blocks_returns_input() {
        assert_eq!(restore_pre_blocks("plain html", &[]), "plain html");
    }

    #[test]
    fn pre_block_placeholder_survives_webui_component_slot_render() -> TestResult {
        let input = "<code-block><pre><code>let x = 1;\n</code></pre></code-block>";
        let (protected, blocks) = protect_pre_blocks(input);
        let page_html = format!("<!DOCTYPE html><html><body>{protected}</body></html>");

        let tmp = std::env::temp_dir().join(format!(
            "webui-press-slot-test-{}-{:x}",
            std::process::id(),
            fxhash(input)
        ));
        if tmp.exists() {
            fs::remove_dir_all(&tmp)?;
        }
        fs::create_dir_all(&tmp)?;
        fs::write(tmp.join("index.html"), page_html)?;

        let components = vec![Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("components")
            .to_string_lossy()
            .into_owned()];
        let build_result = webui::build(BuildOptions {
            app_dir: tmp.clone(),
            entry: "index.html".to_string(),
            plugin: Some(webui::Plugin::WebUI),
            components,
            ..BuildOptions::default()
        })?;

        let mut writer = StringWriter::with_capacity(4096);
        let handler = WebUIHandler::with_plugin(|| {
            Box::new(webui_handler::plugin::webui::WebUIHydrationPlugin::new())
        });
        handler.render(
            &build_result.protocol,
            &Value::Object(Map::new()),
            &RenderOptions::new("index.html", "/"),
            &mut writer,
        )?;

        fs::remove_dir_all(&tmp)?;

        let html = restore_pre_blocks(&writer.buf, &blocks);
        assert!(
            html.contains("<pre><code>let x = 1;\n</code></pre>"),
            "slotted pre block should be restored after WebUI render: {html}"
        );
        assert!(
            !html.contains(PRE_BLOCK_MARKER_PREFIX),
            "placeholder marker must not survive output: {html}"
        );

        Ok(())
    }

    #[test]
    fn discover_component_scripts_pairs_html_and_ts() -> TestResult {
        let root = std::env::temp_dir().join(format!(
            "webui-press-component-script-test-{}-{:x}",
            std::process::id(),
            fxhash("component-script")
        ));
        if root.exists() {
            fs::remove_dir_all(&root)?;
        }
        fs::create_dir_all(root.join("my-widget"))?;
        fs::create_dir_all(root.join("html-only"))?;
        fs::write(root.join("my-widget/my-widget.ts"), "")?;
        fs::write(root.join("my-widget/my-widget.html"), "<p>widget</p>")?;
        fs::write(root.join("html-only/html-only.html"), "<p>no script</p>")?;

        let index = discover_component_scripts(&[root.to_string_lossy().into_owned()])?;
        let expected = root.join("my-widget/my-widget.ts").canonicalize()?;

        fs::remove_dir_all(&root)?;

        assert_eq!(index.len(), 1);
        let Some(script) = index.get("my-widget") else {
            panic!("my-widget script should be discovered");
        };
        assert_eq!(script.script_path, expected);
        Ok(())
    }

    #[test]
    fn collect_component_scripts_for_html_follows_nested_local_components() -> TestResult {
        let root = std::env::temp_dir().join(format!(
            "webui-press-component-nesting-test-{}-{:x}",
            std::process::id(),
            fxhash("component-nesting")
        ));
        if root.exists() {
            fs::remove_dir_all(&root)?;
        }
        fs::create_dir_all(root.join("live-preview"))?;
        fs::create_dir_all(root.join("inner-card"))?;
        fs::write(
            root.join("live-preview/live-preview.html"),
            "<section><inner-card></inner-card></section>",
        )?;
        fs::write(root.join("live-preview/live-preview.ts"), "")?;
        fs::write(root.join("inner-card/inner-card.html"), "<slot></slot>")?;
        fs::write(root.join("inner-card/inner-card.ts"), "")?;

        let index = discover_component_scripts(&[root.to_string_lossy().into_owned()])?;
        let scripts =
            collect_component_scripts_for_html(&["<live-preview></live-preview>"], &index)?;
        let expected_live = root.join("live-preview/live-preview.ts").canonicalize()?;
        let expected_inner = root.join("inner-card/inner-card.ts").canonicalize()?;

        fs::remove_dir_all(&root)?;

        assert_eq!(scripts, vec![expected_live, expected_inner]);
        Ok(())
    }

    #[test]
    fn page_bundle_signature_matches_identical_import_sets() {
        let config_dir = Path::new("/repo/.webui-press");
        let components = vec![PathBuf::from(
            "/repo/components/live-preview/live-preview.ts",
        )];
        let scripts = vec![
            ScriptSource::File("./scripts/fluent.ts".to_string()),
            ScriptSource::Inline("import \"@mai-ui/button/define.js\";".to_string()),
        ];

        assert_eq!(
            page_bundle_signature(&components, &scripts, config_dir),
            page_bundle_signature(&components, &scripts, config_dir)
        );

        let changed = vec![ScriptSource::Inline(
            "import \"@mai-ui/card/define.js\";".to_string(),
        )];
        assert_ne!(
            page_bundle_signature(&components, &scripts, config_dir),
            page_bundle_signature(&components, &changed, config_dir)
        );
    }

    #[test]
    fn import_only_relative_imports_detects_wrapper_entries() {
        let imports = import_only_relative_imports(
            r#"import"./chunk-a.js";import "./chunk-b.js";"#,
            "assets/page-0.js",
        );
        assert_eq!(
            imports,
            Some(vec![
                "assets/chunk-a.js".to_string(),
                "assets/chunk-b.js".to_string()
            ])
        );

        assert_eq!(
            import_only_relative_imports(
                r#"import{a as b}from"./chunk-a.js";console.log(b);"#,
                "assets/page-0.js"
            ),
            None
        );
    }

    #[test]
    fn page_script_paths_flattens_import_only_wrapper() -> TestResult {
        let root = std::env::temp_dir().join(format!(
            "webui-press-wrapper-flatten-test-{}-{:x}",
            std::process::id(),
            fxhash("wrapper-flatten")
        ));
        if root.exists() {
            fs::remove_dir_all(&root)?;
        }
        let assets = root.join("assets");
        fs::create_dir_all(&assets)?;
        fs::write(
            assets.join("page-0.js"),
            r#"import"./chunk-a.js";import"./chunk-b.js";import"./chunk-shared.js";"#,
        )?;
        fs::write(
            assets.join("chunk-a.js"),
            r#"import"./chunk-shared.js";console.log("a");"#,
        )?;
        fs::write(assets.join("chunk-b.js"), r#"console.log("b");"#)?;
        fs::write(assets.join("chunk-shared.js"), r#"console.log("shared");"#)?;

        let mut root_imports = HashSet::new();
        root_imports.insert("assets/chunk-shared.js".to_string());
        let paths = page_script_paths(
            &root,
            "assets/page-0.js",
            &assets.join("page-0.js"),
            &root_imports,
        )?;

        assert!(!assets.join("page-0.js").exists());
        assert_eq!(paths.len(), 2);
        assert!(paths[0].starts_with("assets/chunk-a.js?v="));
        assert!(paths[1].starts_with("assets/chunk-b.js?v="));
        assert!(!paths
            .iter()
            .any(|path| path.starts_with("assets/chunk-shared.js")));

        fs::remove_dir_all(&root)?;
        Ok(())
    }

    #[test]
    fn build_search_entry_decodes_code_text_and_captures_headings() {
        let entry = build_search_entry(
            "`<for>` Loop Directive",
            "/guide/concepts/directives/for/",
            r##"<h1 id="for"><code>&lt;for&gt;</code> Loop Directive <a class="header-anchor" href="#for">#</a></h1><p>Use <code>&lt;for&gt;</code> loops.</p><h2 id="syntax">Syntax <a class="header-anchor" href="#syntax">#</a></h2><h3 id="nested">Nested Search <a class="header-anchor" href="#nested">#</a></h3>"##,
        );

        assert_eq!(entry["title"].as_str(), Some("`<for>` Loop Directive"));
        assert_eq!(
            entry["content"].as_str(),
            Some("<for> Loop Directive Use <for> loops. Syntax Nested Search")
        );
        let Some(headings) = entry["headings"].as_array() else {
            panic!("headings should be an array: {entry:?}");
        };
        assert_eq!(headings.len(), 3);
        assert_eq!(headings[0]["text"].as_str(), Some("<for> Loop Directive"));
        assert_eq!(headings[0]["anchor"].as_str(), Some("for"));
        assert_eq!(headings[0]["level"].as_u64(), Some(1));
        assert_eq!(headings[1]["text"].as_str(), Some("Syntax"));
        assert_eq!(headings[1]["anchor"].as_str(), Some("syntax"));
        assert_eq!(headings[1]["level"].as_u64(), Some(2));
        assert_eq!(headings[2]["text"].as_str(), Some("Nested Search"));
        assert_eq!(headings[2]["anchor"].as_str(), Some("nested"));
        assert_eq!(headings[2]["level"].as_u64(), Some(3));
    }

    // --- fxhash ----------------------------------------------------------

    #[test]
    fn fxhash_deterministic() {
        assert_eq!(fxhash("abc"), fxhash("abc"));
        assert_ne!(fxhash("abc"), fxhash("abd"));
    }

    // --- extract_bundle_scripts -------------------------------------------

    #[test]
    fn extract_bundle_scripts_inline() {
        let html = r#"<p>Hello</p><script type="module" bundle>import "@fluentui/web-components";</script><p>World</p>"#;
        let (out, scripts) = extract_bundle_scripts(html);
        assert_eq!(scripts.len(), 1);
        assert!(matches!(&scripts[0], ScriptSource::Inline(s) if s.contains("@fluentui")));
        assert!(!out.contains("<script"));
        assert!(out.contains("<p>Hello</p>"));
        assert!(out.contains("<p>World</p>"));
    }

    #[test]
    fn extract_bundle_scripts_src() {
        let html = r#"<script type="module" bundle src="./scripts/playground.ts"></script>"#;
        let (out, scripts) = extract_bundle_scripts(html);
        assert_eq!(scripts.len(), 1);
        assert!(matches!(&scripts[0], ScriptSource::File(s) if s == "./scripts/playground.ts"));
        assert_eq!(out, "");
    }

    #[test]
    fn extract_bundle_scripts_ignores_non_bundle() {
        let html = r#"<script type="module">console.log("hi");</script>"#;
        let (out, scripts) = extract_bundle_scripts(html);
        assert_eq!(scripts.len(), 0);
        assert_eq!(out, html);
    }

    #[test]
    fn extract_bundle_scripts_multiple() {
        let html = concat!(
            r#"<script type="module" bundle>import "a";</script>"#,
            r#"<p>middle</p>"#,
            r#"<script type="module" bundle src="./b.ts"></script>"#,
        );
        let (out, scripts) = extract_bundle_scripts(html);
        assert_eq!(scripts.len(), 2);
        assert!(matches!(&scripts[0], ScriptSource::Inline(s) if s.contains("import \"a\"")));
        assert!(matches!(&scripts[1], ScriptSource::File(s) if s == "./b.ts"));
        assert!(out.contains("<p>middle</p>"));
        assert!(!out.contains("<script"));
    }

    #[test]
    fn extract_bundle_scripts_empty_body_no_src_skipped() {
        let html = r#"<script type="module" bundle></script>"#;
        let (out, scripts) = extract_bundle_scripts(html);
        assert_eq!(scripts.len(), 0);
        assert_eq!(out, html); // passes through unchanged
    }

    // --- has_bundle_attr --------------------------------------------------

    #[test]
    fn has_bundle_attr_standalone() {
        assert!(has_bundle_attr(r#" type="module" bundle"#));
        assert!(has_bundle_attr(r#" bundle type="module""#));
        assert!(has_bundle_attr(" bundle"));
        assert!(has_bundle_attr(r#" bundle="" type="module""#));
    }

    #[test]
    fn has_bundle_attr_not_substring() {
        assert!(!has_bundle_attr(r#" type="module" data-bundle="true""#));
        assert!(!has_bundle_attr(r#" type="module" data-mode="bundle""#));
        assert!(!has_bundle_attr(r#" unbundle"#));
    }

    // --- extract_src_attr -------------------------------------------------

    #[test]
    fn extract_src_attr_double_quotes() {
        assert_eq!(extract_src_attr(r#" src="./foo.ts""#), Some("./foo.ts"));
    }

    #[test]
    fn extract_src_attr_single_quotes() {
        assert_eq!(extract_src_attr(" src='bar.js'"), Some("bar.js"));
    }

    #[test]
    fn extract_src_attr_none_when_missing() {
        assert_eq!(extract_src_attr(r#" type="module" bundle"#), None);
    }

    // --- script tag injection --------------------------------------------

    #[test]
    fn module_script_tag_prefixes_base_path() {
        let tag = module_script_tag("/webui/", "assets/page-0.js?v=abc");
        assert_eq!(
            tag,
            "\n<script type=\"module\" src=\"/webui/assets/page-0.js?v=abc\"></script>"
        );
    }

    #[test]
    fn module_script_tag_handles_root_base_path() {
        assert_eq!(
            module_script_tag("/", "assets/page-0.js"),
            "\n<script type=\"module\" src=\"/assets/page-0.js\"></script>"
        );
    }

    #[test]
    fn inject_script_tag_inserts_before_body_close() {
        let mut html = "<html><body><p>content</p></body></html>".to_string();
        inject_script_tag(
            &mut html,
            "\n<script type=\"module\" src=\"/assets/page-0.js\"></script>",
        );
        assert!(html.contains(
            "<p>content</p>\n<script type=\"module\" src=\"/assets/page-0.js\"></script></body>"
        ));
    }
}

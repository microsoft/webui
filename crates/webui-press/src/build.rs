// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

//! Build orchestrator: content pipeline → protocol build → parallel render → output.

use std::collections::{BTreeMap, HashMap};
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

/// A script entry extracted from a page's HTML for bundling.
#[derive(Debug, Clone)]
struct PageScript {
    /// Unique numeric ID used in the placeholder comment.
    id: usize,
    /// The page path this script belongs to (retained for diagnostics).
    #[allow(dead_code)]
    page_path: String,
    /// Script content: either inline source or a `src` path reference.
    source: ScriptSource,
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

    // Kick off TypeScript component bundling on a background thread.
    // Rolldown is independent of the render pipeline, so we overlap it
    // with the per-page protocol build + render.
    //
    // Step 2b: Extract <script bundle> tags from page content BEFORE
    // rendering. This gives us the page scripts to bundle alongside
    // components. The extraction mutates page content by replacing scripts
    // with placeholder comments.
    //
    // Also collect `scriptFile` entries from customPages config — these
    // produce per-page scripts without polluting the HTML string.
    let mut all_page_scripts: Vec<PageScript> = Vec::new();
    let mut page_contents_with_scripts: HashMap<String, String> = HashMap::new();

    for page in &pages {
        let content = page.state["page"]["content"].as_str().unwrap_or("");
        if content.contains("<script") && content.contains("bundle") {
            let (modified, scripts) =
                extract_bundle_scripts(content, &page.path, all_page_scripts.len());
            if !scripts.is_empty() {
                page_contents_with_scripts.insert(page.path.clone(), modified);
                all_page_scripts.extend(scripts);
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
            let id = all_page_scripts.len();
            all_page_scripts.push(PageScript {
                id,
                page_path,
                source: ScriptSource::File(script_path.to_string()),
            });
        }
    }

    // Build a map of page_path → [script_id] so we can inject <script> tags
    // into the rendered output in step 8b. This is separate from
    // `page_contents_with_scripts` (which uses inline placeholders) because
    // the WebUI renderer strips HTML comments, so scriptFile entries can't
    // rely on placeholder comments surviving.
    let mut page_script_ids: HashMap<String, Vec<usize>> =
        HashMap::with_capacity(all_page_scripts.len());
    for script in &all_page_scripts {
        page_script_ids
            .entry(script.page_path.clone())
            .or_default()
            .push(script.id);
    }

    let template_dir_owned = template_dir.to_path_buf();
    let component_sources_clone = component_sources.clone();
    let site_dir_clone = site_dir.clone();
    let page_scripts_clone = all_page_scripts.clone();
    let bundler_config = config.bundler.clone();
    let dev_mode = cache.dev_mode;
    let config_dir_owned = config_dir.to_path_buf();
    let bundle_handle = std::thread::spawn(move || -> Result<BundleResult> {
        let node_modules = std::env::current_dir()
            .unwrap_or_default()
            .join("node_modules");
        bundle_assets(&BundleOptions {
            template_dir: &template_dir_owned,
            component_sources: &component_sources_clone,
            site_dir: &site_dir_clone,
            node_modules: &node_modules,
            page_scripts: &page_scripts_clone,
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

        // Use modified content (with script placeholders) if scripts were extracted.
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

    let not_found_content = format!(
        "<h1>404 — Page Not Found</h1>\
         <p>The page you're looking for doesn't exist or has been moved.</p>\
         <p><a href=\"{base_path}\">← Back to Home</a></p>"
    );
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
    let bundle_result = bundle_handle
        .join()
        .map_err(|_| Error::Build("Bundle thread panicked".to_string()))??;
    print_success(
        cache,
        &format!("Bundled {} components", bundle_result.component_count),
    );

    // Step 8b: Inject page script <script> tags into rendered pages.
    // For inline <script bundle> tags, placeholder comments in the content
    // were replaced by the renderer. For scriptFile entries (and as a
    // fallback), we inject <script> tags before </body> using page_script_ids.
    if !bundle_result.script_map.is_empty() {
        let mut linked_count = 0usize;
        for page in &pages {
            let has_placeholders = page_contents_with_scripts.contains_key(&page.path);
            let script_ids = page_script_ids.get(&page.path);
            if !has_placeholders && script_ids.is_none() {
                continue;
            }
            let page_dir = site_dir.join(page.path.strip_prefix(base_path).unwrap_or(&page.path));
            let target = page_dir.join("index.html");
            let Ok(mut html) = fs::read_to_string(&target) else {
                continue;
            };

            // Replace inline placeholder comments (from <script bundle> extraction).
            if has_placeholders && html.contains(SCRIPT_PLACEHOLDER_PREFIX) {
                html = replace_script_placeholders(&html, base_path, &bundle_result.script_map);
            }

            // Inject <script> tags for script IDs that don't have placeholders
            // (i.e. scriptFile entries). Insert before </body>.
            if let Some(ids) = script_ids {
                let mut tags = String::new();
                for &id in ids {
                    if let Some(rel_path) = bundle_result.script_map.get(&id) {
                        let src = if base_path.is_empty() || base_path == "/" {
                            rel_path.clone()
                        } else {
                            format!(
                                "{}/{}",
                                base_path.trim_end_matches('/'),
                                rel_path.trim_start_matches('/')
                            )
                        };
                        tags.push_str(&format!(
                            "\n<script type=\"module\" src=\"{src}\"></script>"
                        ));
                        linked_count += 1;
                    }
                }
                if !tags.is_empty() {
                    if let Some(pos) = html.rfind("</body>") {
                        html.insert_str(pos, &tags);
                    } else {
                        html.push_str(&tags);
                    }
                }
            }

            fs::write(&target, &html)
                .map_err(|e| Error::Io(format!("Cannot rewrite {}: {e}", page.path)))?;
        }
        if linked_count > 0 {
            print_success(
                cache,
                &format!(
                    "Linked {} page script{}",
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

fn is_component_ts_file(path: &Path) -> bool {
    let Some(file_name) = path.file_name().and_then(|name| name.to_str()) else {
        return false;
    };
    path.extension().is_some_and(|extension| extension == "ts")
        && !file_name.ends_with(".spec.ts")
        && !file_name.ends_with(".test.ts")
        && !file_name.ends_with(".d.ts")
}

/// Collect component `.ts` files from a directory tree (iterative).
fn collect_ts_files(dir: &Path) -> Vec<std::path::PathBuf> {
    let mut files = Vec::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        let Ok(entries) = fs::read_dir(&d) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
            } else if is_component_ts_file(&path) {
                files.push(path);
            }
        }
    }
    files.sort();
    files
}

/// Bundle result returned by [`bundle_assets`].
struct BundleResult {
    /// Number of component entry points bundled.
    component_count: usize,
    /// Map from page-script ID to relative output path (e.g. `"assets/playground-abc123.js"`).
    script_map: HashMap<usize, String>,
}

/// Configuration for the [`bundle_assets`] function.
struct BundleOptions<'a> {
    template_dir: &'a Path,
    component_sources: &'a [String],
    site_dir: &'a Path,
    node_modules: &'a Path,
    page_scripts: &'a [PageScript],
    bundler_config: Option<&'a BundlerConfig>,
    dev_mode: bool,
    config_dir: &'a Path,
}

fn path_for_js(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

fn json_string(value: &str) -> Result<String> {
    serde_json::to_string(value)
        .map_err(|e| Error::Build(format!("Cannot serialize Rolldown config string: {e}")))
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

fn write_rolldown_config(
    opts: &BundleOptions<'_>,
    entry_files: &[(String, PathBuf)],
    output_dir: &Path,
    bundle_tmp: &Path,
) -> Result<PathBuf> {
    let mut aliases: BTreeMap<String, String> = BTreeMap::new();
    if let Some(path) = default_framework_alias(opts.node_modules) {
        aliases.insert("@microsoft/webui-framework".to_string(), path_for_js(&path));
    }

    if let Some(cfg) = opts.bundler_config {
        for (from, to) in &cfg.alias {
            aliases.insert(from.clone(), normalized_alias_target(opts.config_dir, to));
        }
    }

    let mut config = String::with_capacity(2048);
    config.push_str("export default {\n  input: {\n");
    for (name, path) in entry_files {
        config.push_str("    ");
        config.push_str(&json_string(name)?);
        config.push_str(": ");
        config.push_str(&json_string(&path_for_js(path))?);
        config.push_str(",\n");
    }
    config.push_str("  },\n  output: {\n    dir: ");
    config.push_str(&json_string(&path_for_js(output_dir))?);
    config.push_str(",\n    format: \"esm\",\n    entryFileNames: \"[name].js\",\n    chunkFileNames: \"assets/[name]-[hash].js\",\n  },\n  advancedChunks: { minSize: 0 },\n  moduleTypes: { \".html\": \"text\", \".css\": \"text\" },\n");

    if !opts.dev_mode {
        config.push_str("  minify: true,\n");
    }

    if let Some(cfg) = opts.bundler_config {
        if !cfg.external.is_empty() {
            config.push_str("  external: ");
            config.push_str(
                &serde_json::to_string(&cfg.external).map_err(|e| {
                    Error::Build(format!("Cannot serialize Rolldown externals: {e}"))
                })?,
            );
            config.push_str(",\n");
        }

        if cfg.target.is_some() || !cfg.define.is_empty() {
            config.push_str("  transform: {\n");
            if let Some(target) = &cfg.target {
                config.push_str("    target: ");
                config.push_str(&json_string(target)?);
                config.push_str(",\n");
            }
            if !cfg.define.is_empty() {
                config.push_str("    define: ");
                config.push_str(&serde_json::to_string(&cfg.define).map_err(|e| {
                    Error::Build(format!("Cannot serialize Rolldown defines: {e}"))
                })?);
                config.push_str(",\n");
            }
            config.push_str("  },\n");
        }
    }

    if !aliases.is_empty() {
        config.push_str("  resolve: { alias: ");
        config.push_str(
            &serde_json::to_string(&aliases)
                .map_err(|e| Error::Build(format!("Cannot serialize Rolldown aliases: {e}")))?,
        );
        config.push_str(" },\n");
    }

    config.push_str("};\n");

    let config_path = bundle_tmp.join("rolldown.config.mjs");
    fs::write(&config_path, config)
        .map_err(|e| Error::Build(format!("Cannot write Rolldown config: {e}")))?;
    Ok(config_path)
}

/// Bundle component TypeScript files and page scripts via Rolldown.
///
/// Uses a single Rolldown invocation with multiple entry points for optimal
/// code splitting. Component .ts files produce `components.js` (the global
/// hydration bundle); page scripts produce separate entry chunks under `assets/`.
///
/// Returns a [`BundleResult`] with the component count and a mapping from
/// page-script IDs to their output file paths.
fn bundle_assets(opts: &BundleOptions<'_>) -> Result<BundleResult> {
    let mut ts_files = Vec::new();

    // Collect from user component directories
    for dir in opts.component_sources {
        let p = Path::new(dir);
        if p.exists() {
            ts_files.extend(collect_ts_files(p));
        }
    }

    // Collect from template subdirectories (docs-search, docs-theme-toggle, etc.)
    if let Ok(entries) = fs::read_dir(opts.template_dir) {
        for entry in entries.flatten() {
            if entry.path().is_dir() {
                ts_files.extend(collect_ts_files(&entry.path()));
            }
        }
    }

    // Exclude component .ts files that are explicitly referenced as page script sources.
    // This avoids bundling the same file in both the global components.js and a page script.
    if !opts.page_scripts.is_empty() {
        let page_script_paths: Vec<std::path::PathBuf> = opts
            .page_scripts
            .iter()
            .filter_map(|s| match &s.source {
                ScriptSource::File(path) => {
                    let resolved = opts.config_dir.join(path);
                    resolved.canonicalize().ok()
                }
                ScriptSource::Inline(_) => None,
            })
            .collect();

        if !page_script_paths.is_empty() {
            ts_files.retain(|f| {
                let canon = f.canonicalize().unwrap_or_else(|_| f.clone());
                !page_script_paths.contains(&canon)
            });
        }
    }

    let has_components = !ts_files.is_empty();
    let has_scripts = !opts.page_scripts.is_empty();

    if !has_components && !has_scripts {
        return Ok(BundleResult {
            component_count: 0,
            script_map: HashMap::new(),
        });
    }

    // Create a temp directory for the bundler entry files.
    let bundle_tmp =
        std::env::temp_dir().join(format!("webui-press-bundle-{}", std::process::id(),));
    if bundle_tmp.exists() {
        fs::remove_dir_all(&bundle_tmp).ok();
    }
    fs::create_dir_all(&bundle_tmp)
        .map_err(|e| Error::Build(format!("Cannot create bundle temp dir: {e}")))?;

    let assets_dir = opts.site_dir.join("assets");
    fs::create_dir_all(&assets_dir)
        .map_err(|e| Error::Io(format!("Cannot create assets dir: {e}")))?;

    // Build the component entry file (imports all component .ts files).
    let mut entry_files: Vec<(String, std::path::PathBuf)> = Vec::new();

    if has_components {
        let imports: String = ts_files
            .iter()
            .map(|f| format!("import \"{}\";", f.to_string_lossy().replace('\\', "/")))
            .collect::<Vec<_>>()
            .join("\n");
        let components_entry = bundle_tmp.join("_components.ts");
        fs::write(&components_entry, &imports)
            .map_err(|e| Error::Build(format!("Cannot write component entry: {e}")))?;
        entry_files.push(("components".to_string(), components_entry));
    }

    // Write page script entry files.
    for script in opts.page_scripts {
        let entry_name = format!("assets/page-{}", script.id);
        let entry_path = bundle_tmp.join(format!("{entry_name}.ts"));
        let content = match &script.source {
            ScriptSource::Inline(code) => code.clone(),
            ScriptSource::File(path) => {
                // Resolve src path relative to config_dir and canonicalize
                // to an absolute path so the entry file (in a temp dir)
                // can resolve the import.
                let resolved = opts.config_dir.join(path);
                let abs_path = resolved
                    .canonicalize()
                    .unwrap_or(resolved)
                    .to_string_lossy()
                    .replace('\\', "/");
                format!("import \"{abs_path}\";")
            }
        };
        if let Some(parent) = entry_path.parent() {
            fs::create_dir_all(parent)
                .map_err(|e| Error::Build(format!("Cannot create script entry dir: {e}")))?;
        }
        fs::write(&entry_path, &content)
            .map_err(|e| Error::Build(format!("Cannot write script entry: {e}")))?;
        entry_files.push((entry_name, entry_path));
    }

    let rolldown_config = write_rolldown_config(opts, &entry_files, opts.site_dir, &bundle_tmp)?;

    // Resolve the rolldown binary from node_modules.
    let rolldown_bin = rolldown_command(opts.node_modules);

    let output = std::process::Command::new(&rolldown_bin)
        .arg("--config")
        .arg(&rolldown_config)
        .arg("--logLevel")
        .arg("warn")
        .env("NODE_PATH", opts.node_modules)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .output()
        .map_err(|e| Error::Build(format!("rolldown failed to start: {e}")))?;

    let stderr = String::from_utf8_lossy(&output.stderr);
    if !output.status.success() {
        return Err(Error::Build(format!("rolldown error: {stderr}")));
    }
    if stderr.contains("[UNRESOLVED_IMPORT]") {
        return Err(Error::Build(format!(
            "rolldown unresolved import: {stderr}"
        )));
    }

    // Build script_map: find output files for page-script entries.
    let mut script_map = HashMap::with_capacity(opts.page_scripts.len());
    for script in opts.page_scripts {
        let entry_name = format!("page-{}", script.id);
        // Rolldown outputs entry chunks as `{entry_name}.js` in the output dir.
        let output_file = format!("assets/{entry_name}.js");
        let full_path = opts.site_dir.join(&output_file);
        if full_path.exists() {
            script_map.insert(script.id, output_file);
        }
    }

    // Clean up temp dir.
    fs::remove_dir_all(&bundle_tmp).ok();

    Ok(BundleResult {
        component_count: ts_files.len(),
        script_map,
    })
}

/// Resolve the rolldown binary path from node_modules.
fn rolldown_command(node_modules: &Path) -> std::path::PathBuf {
    let bin_dir = node_modules.join(".bin");
    let rolldown = if cfg!(windows) {
        bin_dir.join("rolldown.cmd")
    } else {
        bin_dir.join("rolldown")
    };
    if rolldown.exists() {
        rolldown
    } else {
        // Fallback to PATH-based lookup.
        std::path::PathBuf::from(if cfg!(windows) {
            "rolldown.cmd"
        } else {
            "rolldown"
        })
    }
}

const PRE_BLOCK_MARKER_PREFIX: &str = "<span data-webui-press-pre-block=\"";
const PRE_BLOCK_MARKER_SUFFIX: &str = "\"></span>";

const SCRIPT_PLACEHOLDER_PREFIX: &str = "<!--ws:script:";
const SCRIPT_PLACEHOLDER_SUFFIX: &str = "-->";

/// Extract `<script type="module" bundle>` and `<script type="module" bundle src="...">` tags
/// from page content HTML. Returns the modified content (with placeholder comments)
/// and the extracted script entries.
///
/// The scanner is iterative and avoids regex (per project rules). It looks for
/// `<script` tags containing the `bundle` attribute and replaces each with a
/// `<!--ws:script:ID-->` comment placeholder.
fn extract_bundle_scripts(
    content: &str,
    page_path: &str,
    id_offset: usize,
) -> (String, Vec<PageScript>) {
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

        let id = id_offset + scripts.len();
        scripts.push(PageScript {
            id,
            page_path: page_path.to_string(),
            source,
        });

        // Emit the placeholder comment.
        out.push_str(&content[cursor..tag_start]);
        out.push_str(SCRIPT_PLACEHOLDER_PREFIX);
        out.push_str(&id.to_string());
        out.push_str(SCRIPT_PLACEHOLDER_SUFFIX);
        cursor = close_end;
    }

    out.push_str(&content[cursor..]);
    (out, scripts)
}

/// Check if the attributes region contains a standalone `bundle` word.
fn has_bundle_attr(attrs: &str) -> bool {
    let bytes = attrs.as_bytes();
    let target = b"bundle";
    let mut i = 0;
    while i + target.len() <= bytes.len() {
        if let Some(rel) = attrs[i..].find("bundle") {
            let pos = i + rel;
            let before_ok =
                pos == 0 || matches!(bytes[pos - 1], b' ' | b'\t' | b'\n' | b'\r' | b'"' | b'\'');
            let after_pos = pos + target.len();
            let after_ok = after_pos >= bytes.len()
                || matches!(
                    bytes[after_pos],
                    b' ' | b'\t' | b'\n' | b'\r' | b'>' | b'=' | b'"' | b'\''
                );
            if before_ok && after_ok {
                return true;
            }
            i = pos + 1;
        } else {
            break;
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

/// Replace `<!--ws:script:ID-->` placeholders in rendered HTML with actual
/// `<script type="module" src="...">` tags pointing to bundled output files.
///
/// `script_map` maps script ID → relative output path (e.g. `"assets/playground-abc123.js"`).
fn replace_script_placeholders(
    html: &str,
    base_path: &str,
    script_map: &HashMap<usize, String>,
) -> String {
    if script_map.is_empty() {
        return html.to_string();
    }
    let mut out = String::with_capacity(html.len());
    let mut cursor = 0;
    while let Some(rel) = html[cursor..].find(SCRIPT_PLACEHOLDER_PREFIX) {
        let p = cursor + rel;
        out.push_str(&html[cursor..p]);
        let after = p + SCRIPT_PLACEHOLDER_PREFIX.len();
        if let Some(end_rel) = html[after..].find(SCRIPT_PLACEHOLDER_SUFFIX) {
            let num_str = &html[after..after + end_rel];
            if let Ok(id) = num_str.parse::<usize>() {
                if let Some(output_path) = script_map.get(&id) {
                    out.push_str("<script type=\"module\" src=\"");
                    out.push_str(base_path);
                    out.push_str(output_path);
                    out.push_str("\"></script>");
                    cursor = after + end_rel + SCRIPT_PLACEHOLDER_SUFFIX.len();
                    continue;
                }
            }
            // Unknown placeholder — keep verbatim.
            out.push_str(&html[p..after + end_rel + SCRIPT_PLACEHOLDER_SUFFIX.len()]);
            cursor = after + end_rel + SCRIPT_PLACEHOLDER_SUFFIX.len();
        } else {
            out.push_str(&html[p..]);
            return out;
        }
    }
    out.push_str(&html[cursor..]);
    out
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
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for b in s.bytes() {
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
    fn rolldown_command_resolves_from_node_modules() {
        let tmp = std::env::temp_dir().join("webui-press-rolldown-test");
        let bin_dir = tmp.join("node_modules/.bin");
        fs::create_dir_all(&bin_dir).unwrap();
        let bin_path = if cfg!(windows) {
            bin_dir.join("rolldown.cmd")
        } else {
            bin_dir.join("rolldown")
        };
        fs::write(&bin_path, "").unwrap();
        let resolved = rolldown_command(&tmp.join("node_modules"));
        assert_eq!(resolved, bin_path);
        fs::remove_dir_all(&tmp).ok();
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
    fn collect_ts_files_skips_tests_and_declarations() -> TestResult {
        let root = std::env::temp_dir().join(format!(
            "webui-press-ts-collect-test-{}-{:x}",
            std::process::id(),
            fxhash("ts-collect")
        ));
        if root.exists() {
            fs::remove_dir_all(&root)?;
        }
        fs::create_dir_all(root.join("my-widget"))?;
        fs::write(root.join("my-widget/my-widget.ts"), "")?;
        fs::write(root.join("my-widget/my-widget.spec.ts"), "")?;
        fs::write(root.join("my-widget/my-widget.test.ts"), "")?;
        fs::write(root.join("my-widget/my-widget.d.ts"), "")?;

        let files = collect_ts_files(&root);

        fs::remove_dir_all(&root)?;

        assert_eq!(
            files,
            vec![root.join("my-widget/my-widget.ts")],
            "only hydration entry files should be bundled"
        );
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
        let (out, scripts) = extract_bundle_scripts(html, "/test/", 0);
        assert_eq!(scripts.len(), 1);
        assert_eq!(scripts[0].id, 0);
        assert!(matches!(&scripts[0].source, ScriptSource::Inline(s) if s.contains("@fluentui")));
        assert!(out.contains("<!--ws:script:0-->"));
        assert!(!out.contains("<script"));
        assert!(out.contains("<p>Hello</p>"));
        assert!(out.contains("<p>World</p>"));
    }

    #[test]
    fn extract_bundle_scripts_src() {
        let html = r#"<script type="module" bundle src="./scripts/playground.ts"></script>"#;
        let (out, scripts) = extract_bundle_scripts(html, "/playground/", 0);
        assert_eq!(scripts.len(), 1);
        assert!(
            matches!(&scripts[0].source, ScriptSource::File(s) if s == "./scripts/playground.ts")
        );
        assert!(out.contains("<!--ws:script:0-->"));
    }

    #[test]
    fn extract_bundle_scripts_ignores_non_bundle() {
        let html = r#"<script type="module">console.log("hi");</script>"#;
        let (out, scripts) = extract_bundle_scripts(html, "/page/", 0);
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
        let (out, scripts) = extract_bundle_scripts(html, "/page/", 5);
        assert_eq!(scripts.len(), 2);
        assert_eq!(scripts[0].id, 5);
        assert_eq!(scripts[1].id, 6);
        assert!(out.contains("<!--ws:script:5-->"));
        assert!(out.contains("<!--ws:script:6-->"));
        assert!(out.contains("<p>middle</p>"));
    }

    #[test]
    fn extract_bundle_scripts_empty_body_no_src_skipped() {
        let html = r#"<script type="module" bundle></script>"#;
        let (out, scripts) = extract_bundle_scripts(html, "/page/", 0);
        assert_eq!(scripts.len(), 0);
        assert_eq!(out, html); // passes through unchanged
    }

    // --- has_bundle_attr --------------------------------------------------

    #[test]
    fn has_bundle_attr_standalone() {
        assert!(has_bundle_attr(r#" type="module" bundle"#));
        assert!(has_bundle_attr(r#" bundle type="module""#));
        assert!(has_bundle_attr(" bundle"));
    }

    #[test]
    fn has_bundle_attr_not_substring() {
        assert!(!has_bundle_attr(r#" type="module" data-bundle="true""#));
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

    // --- replace_script_placeholders -------------------------------------

    #[test]
    fn replace_script_placeholders_basic() {
        let html = "<html><!--ws:script:0--><p>content</p><!--ws:script:1--></html>";
        let mut map = HashMap::new();
        map.insert(0, "assets/page-0.js".to_string());
        map.insert(1, "assets/page-1.js".to_string());
        let result = replace_script_placeholders(html, "/webui/", &map);
        assert!(result.contains(r#"<script type="module" src="/webui/assets/page-0.js"></script>"#));
        assert!(result.contains(r#"<script type="module" src="/webui/assets/page-1.js"></script>"#));
        assert!(!result.contains("<!--ws:script"));
    }

    #[test]
    fn replace_script_placeholders_empty_map_unchanged() {
        let html = "no scripts here";
        let map = HashMap::new();
        assert_eq!(replace_script_placeholders(html, "/", &map), html);
    }
}

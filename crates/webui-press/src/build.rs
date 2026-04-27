// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

//! Build orchestrator: content pipeline → protocol build → parallel render → output.

use std::fs;
use std::path::Path;
use std::time::Instant;

use console::style;
use rayon::prelude::*;
use serde_json::{Map, Value};
use webui::BuildOptions;
use webui_handler::{RenderOptions, ResponseWriter, WebUIHandler};

use crate::content::process_content;
use crate::error::{Error, Result};
use crate::markdown::Highlighter;
use crate::types::{BuildStats, DocsConfig};

// ── Output helpers ──────────────────────────────────────────────
//
// Mirrors the styling vocabulary in `crates/webui-cli/src/utils/output.rs`
// so webui-press feels at home in a workspace where users may also see
// `webui build` / `webui serve` output.

fn print_header(title: &str) {
    eprintln!(
        "\n  {} {}",
        style("⚡").cyan().bold(),
        style(title).cyan().bold()
    );
}

fn print_success(message: &str) {
    eprintln!("  {} {message}", style("✔").green());
}

fn print_finish(message: &str) {
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
pub fn build_docs(
    config: &DocsConfig,
    config_dir: &Path,
    template_dir: &Path,
) -> Result<BuildStats> {
    let start = Instant::now();
    let base_path = &config.base_path;
    let out_dir = Path::new(&config.out_dir);

    // When basePath is not "/", nest output under out_dir/<basePath> so a
    // local static server works without path rewriting.
    let site_dir = if base_path != "/" {
        let trimmed = base_path.trim_matches('/');
        out_dir.join(trimmed)
    } else {
        out_dir.to_path_buf()
    };

    // Read custom CSS
    let custom_css = config
        .css
        .as_ref()
        .and_then(|p| fs::read_to_string(p).ok())
        .unwrap_or_default();

    print_header(&config.site.title);
    eprintln!();

    // Step 1: Process content
    let highlighter = Highlighter::new();

    let mut pages = process_content(config, config_dir, &highlighter)?;

    // Step 2: Resolve component sources for the per-page builds
    let mut component_sources: Vec<String> = Vec::new();
    // Built-in component library (e.g. crates/webui-press/components/)
    let builtin_components = template_dir.parent().map(|p| p.join("components"));
    if let Some(ref bc) = builtin_components {
        if bc.exists() {
            component_sources.push(bc.to_string_lossy().to_string());
        }
    }
    // User component dirs from config
    if let Some(ref user_dirs) = config.components {
        for d in user_dirs {
            let abs = std::env::current_dir().unwrap_or_default().join(d);
            component_sources.push(abs.to_string_lossy().to_string());
        }
    }
    // Template-local components (e.g. docs-search, docs-theme-toggle living
    // beside the template's index.html).
    component_sources.push(template_dir.to_string_lossy().to_string());

    let template_html = fs::read_to_string(template_dir.join("index.html"))
        .map_err(|e| Error::Build(format!("Failed to read template: {e}")))?;

    // Step 3: Pre-render pages in parallel
    if out_dir.exists() {
        fs::remove_dir_all(out_dir)
            .map_err(|e| Error::Io(format!("Cannot clean output dir: {e}")))?;
    }
    // The site root must exist before we copy CSS into it; per-page subdirs
    // are created later, just before parallel rendering.
    fs::create_dir_all(&site_dir).map_err(|e| Error::Io(format!("Cannot create site dir: {e}")))?;

    // Copy base stylesheet to output and build head injection
    let base_css_src = template_dir.join("docs.css");
    let base_css_link = if base_css_src.exists() {
        let css_filename = "docs.css";
        fs::copy(&base_css_src, site_dir.join(css_filename))
            .map_err(|e| Error::Io(format!("Cannot copy {css_filename}: {e}")))?;
        format!("<link rel=\"stylesheet\" href=\"{base_path}{css_filename}\">")
    } else {
        String::new()
    };

    // Write custom theme CSS to an external file
    let theme_css_link = if custom_css.is_empty() {
        String::new()
    } else {
        let css_filename = "theme.css";
        fs::write(site_dir.join(css_filename), &custom_css)
            .map_err(|e| Error::Io(format!("Cannot write {css_filename}: {e}")))?;
        format!("<link rel=\"stylesheet\" href=\"{base_path}{css_filename}\">")
    };

    let head_injection = {
        let mut parts = Vec::new();
        if !base_css_link.is_empty() {
            parts.push(base_css_link);
        }
        if !theme_css_link.is_empty() {
            parts.push(theme_css_link);
        }
        for tag in &config.head {
            parts.push(tag.to_html());
        }
        parts.join("\n  ")
    };

    // Inject headTags and component defaults into each page's state
    for page in &mut pages {
        page.state["headTags"] = Value::String(head_injection.clone());
        page.state["label"] = Value::String("Copy".to_string());
        page.state["icon"] = Value::String("🌙".to_string());
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

    // Kick off TypeScript component bundling on a background thread — esbuild
    // (npx process startup) takes 100s of ms and is independent of the render
    // pipeline, so we overlap it with the per-page protocol build + render.
    let template_dir_owned = template_dir.to_path_buf();
    let component_sources_clone = component_sources.clone();
    let site_dir_clone = site_dir.clone();
    let bundle_handle = std::thread::spawn(move || -> Result<usize> {
        let node_modules = std::env::current_dir()
            .unwrap_or_default()
            .join("node_modules");
        bundle_components(
            &template_dir_owned,
            &component_sources_clone,
            &site_dir_clone,
            &node_modules,
        )
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
    let component_css: std::sync::Mutex<std::collections::HashMap<String, String>> =
        std::sync::Mutex::new(std::collections::HashMap::new());
    pages.par_iter().try_for_each(|page| -> Result<()> {
        let content = page.state["page"]["content"].as_str().unwrap_or("");

        // Protect <pre> blocks from HTML parser whitespace normalization.
        let (protected, pre_blocks) = protect_pre_blocks(content);

        // Substitute the raw signal in the template with the literal HTML.
        let page_html = template_html.replace("{{{page.content}}}", &protected);

        // Per-page temp dir holding only this page's index.html — components
        // come exclusively from `component_sources`, which already includes
        // the template dir (for docs-search/docs-theme-toggle) plus any
        // configured component libraries.
        let page_tmp = std::env::temp_dir().join(format!(
            "webui-press-page-{}-{:x}",
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
            css: webui::CssStrategy::Link,
            dom: webui::DomStrategy::Shadow,
            plugin: Some(webui::Plugin::WebUI),
            components: component_sources.clone(),
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
                .map_err(|e| Error::Build(format!("css mutex poisoned: {e}")))?;
            for (name, content) in build_result.css_files {
                css_map.entry(name).or_insert(content);
            }
        }

        let mut writer = StringWriter::with_capacity(8192);
        handler
            .render(
                &build_result.protocol,
                &page.state,
                &RenderOptions::new("index.html", &page.path),
                &mut writer,
            )
            .map_err(|e| Error::Render(format!("{}: {e}", page.path)))?;

        // Restore the protected <pre> blocks via a single-pass scan.
        let html = restore_pre_blocks(&writer.buf, &pre_blocks);

        // Write directly inside the parallel closure.
        let page_dir = site_dir.join(page.path.strip_prefix(base_path).unwrap_or(&page.path));
        fs::write(page_dir.join("index.html"), &html)
            .map_err(|e| Error::Io(format!("Cannot write {}: {e}", page.path)))?;

        fs::remove_dir_all(&page_tmp).ok();
        Ok(())
    })?;

    print_success(&format!("Rendered {} pages", pages.len()));

    // Step 4: Search index (parallel)
    let search_index: Vec<serde_json::Value> = pages
        .par_iter()
        .filter(|p| !p.is_home)
        .map(|p| {
            let html = p.state["page"]["content"].as_str().unwrap_or("");
            let mut clean = String::with_capacity(html.len() / 2);
            let mut in_tag = false;
            for ch in html.chars() {
                match ch {
                    '<' => in_tag = true,
                    '>' => {
                        in_tag = false;
                        clean.push(' ');
                    }
                    _ if !in_tag => clean.push(ch),
                    _ => {}
                }
            }
            let content: String = clean.split_whitespace().collect::<Vec<_>>().join(" ");
            json_obj([
                ("title", Value::String(p.title.clone())),
                ("path", Value::String(p.path.clone())),
                (
                    "content",
                    Value::String(truncate_utf8(&content, 500).to_string()),
                ),
            ])
        })
        .collect();

    fs::write(
        site_dir.join("search-index.json"),
        serde_json::to_string(&search_index)
            .map_err(|e| Error::Build(format!("JSON error: {e}")))?,
    )
    .map_err(|e| Error::Io(e.to_string()))?;
    print_success(&format!("Indexed {} pages for search", search_index.len()));

    // Step 5: Copy static assets
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
    let nf_tmp = std::env::temp_dir().join(format!("webui-press-404-{}", std::process::id()));
    if nf_tmp.exists() {
        fs::remove_dir_all(&nf_tmp).ok();
    }
    fs::create_dir_all(&nf_tmp).map_err(|e| Error::Io(e.to_string()))?;
    fs::write(nf_tmp.join("index.html"), &not_found_html).map_err(|e| Error::Io(e.to_string()))?;

    let nf_build = webui::build(BuildOptions {
        app_dir: nf_tmp.clone(),
        entry: "index.html".to_string(),
        css: webui::CssStrategy::Link,
        dom: webui::DomStrategy::Shadow,
        plugin: Some(webui::Plugin::WebUI),
        components: component_sources.clone(),
    })
    .map_err(|e| Error::Build(format!("404 build failed: {e}")))?;

    // Fold the 404 page's component CSS into the shared map, then write
    // all per-component stylesheets at site root in one pass. The handler
    // emits relative hrefs like `<link rel="stylesheet" href="code-block.css">`;
    // the template's <base href="{site.base}"> resolves them against site root.
    {
        let mut css_map = component_css
            .lock()
            .map_err(|e| Error::Build(format!("css mutex poisoned: {e}")))?;
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

    // Write 404.html into site_dir so GitHub Pages (and any host that maps
    // its publish root to `<basePath>`) serves it for unmatched URLs.
    fs::write(site_dir.join("404.html"), writer_404.buf).map_err(|e| Error::Io(e.to_string()))?;
    fs::remove_dir_all(&nf_tmp).ok();
    print_success("Generated 404 page");

    // Write deduped per-component stylesheets gathered during page + 404 builds.
    let css_map = component_css
        .into_inner()
        .map_err(|e| Error::Build(format!("css mutex poisoned: {e}")))?;
    let css_count = css_map.len();
    for (name, content) in css_map {
        fs::write(site_dir.join(&name), content)
            .map_err(|e| Error::Io(format!("Cannot write {name}: {e}")))?;
    }
    if css_count > 0 {
        print_success(&format!("Wrote {css_count} component stylesheets"));
    }

    // Step 7: Wait for the background bundling thread.
    let component_count = bundle_handle
        .join()
        .map_err(|_| Error::Build("Bundle thread panicked".to_string()))??;
    print_success(&format!("Bundled {component_count} components"));

    let elapsed = start.elapsed();
    let total_bytes = total_bytes.load(std::sync::atomic::Ordering::Relaxed);
    print_finish(&format!("Built in {:.1}s", elapsed.as_secs_f64()));

    Ok(BuildStats {
        pages: pages.len(),
        protocol_bytes: total_bytes,
    })
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
fn truncate_utf8(s: &str, max: usize) -> &str {
    if s.len() <= max {
        return s;
    }
    let mut end = max;
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    &s[..end]
}

/// Collect all `.ts` files from a directory tree (iterative).
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
            } else if path.extension().is_some_and(|e| e == "ts") {
                files.push(path);
            }
        }
    }
    files.sort();
    files
}

/// Bundle component TypeScript files into a single `components.js` via esbuild.
/// Returns the number of components bundled.
fn bundle_components(
    template_dir: &Path,
    component_sources: &[String],
    site_dir: &Path,
    node_modules: &Path,
) -> Result<usize> {
    let mut ts_files = Vec::new();

    // Collect from user component directories
    for dir in component_sources {
        let p = Path::new(dir);
        if p.exists() {
            ts_files.extend(collect_ts_files(p));
        }
    }

    // Collect from template subdirectories (docs-search, docs-theme-toggle, etc.)
    if let Ok(entries) = fs::read_dir(template_dir) {
        for entry in entries.flatten() {
            if entry.path().is_dir() {
                ts_files.extend(collect_ts_files(&entry.path()));
            }
        }
    }

    if ts_files.is_empty() {
        return Ok(0);
    }

    // Generate the entry file content
    let imports: String = ts_files
        .iter()
        .map(|f| format!("import \"{}\";", f.to_string_lossy().replace('\\', "/")))
        .collect::<Vec<_>>()
        .join("\n");

    let out_file = site_dir.join("components.js");

    let status = std::process::Command::new("npx")
        .arg("esbuild")
        .arg("--bundle")
        .arg("--format=esm")
        .arg("--minify")
        .arg("--loader:.html=text")
        .arg("--loader:.css=text")
        .arg(format!("--outfile={}", out_file.to_string_lossy()))
        .env("NODE_PATH", node_modules)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .and_then(|mut child| {
            use std::io::Write;
            if let Some(ref mut stdin) = child.stdin {
                let _ = stdin.write_all(imports.as_bytes());
            }
            child.wait_with_output()
        })
        .map_err(|e| Error::Build(format!("esbuild failed: {e}")))?;

    if !status.status.success() {
        let stderr = String::from_utf8_lossy(&status.stderr);
        return Err(Error::Build(format!("esbuild error: {stderr}")));
    }

    Ok(ts_files.len())
}

/// Replace `<pre …>…</pre>` blocks with placeholder comments so the WebUI
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
            out.push_str("<!--PRE_BLOCK_");
            // write! into existing buffer — avoids `format!` allocation per block.
            let _ = write!(&mut out, "{}", blocks.len());
            out.push_str("-->");
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

/// Single-pass restoration of `<!--PRE_BLOCK_N-->` placeholders to their
/// original content. Faster than calling `String::replace` once per block.
fn restore_pre_blocks(html: &str, blocks: &[String]) -> String {
    if blocks.is_empty() {
        return html.to_string();
    }
    const PREFIX: &str = "<!--PRE_BLOCK_";
    let extra: usize = blocks.iter().map(|b| b.len()).sum();
    let mut out = String::with_capacity(html.len() + extra);
    let mut cursor = 0;
    while let Some(rel) = html[cursor..].find(PREFIX) {
        let p = cursor + rel;
        out.push_str(&html[cursor..p]);
        let after = p + PREFIX.len();
        if let Some(end_rel) = html[after..].find("-->") {
            let num_str = &html[after..after + end_rel];
            if let Ok(idx) = num_str.parse::<usize>() {
                if let Some(block) = blocks.get(idx) {
                    out.push_str(block);
                    cursor = after + end_rel + 3;
                    continue;
                }
            }
            // Unknown placeholder — keep verbatim.
            out.push_str(&html[p..after + end_rel + 3]);
            cursor = after + end_rel + 3;
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
        assert!(out.contains("<!--PRE_BLOCK_0-->"));
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

    // --- fxhash ----------------------------------------------------------

    #[test]
    fn fxhash_deterministic() {
        assert_eq!(fxhash("abc"), fxhash("abc"));
        assert_ne!(fxhash("abc"), fxhash("abd"));
    }
}

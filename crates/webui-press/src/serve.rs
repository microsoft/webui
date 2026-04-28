// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

//! Development server for `webui-press`.
//!
//! Composes [`webui_dev_server`] primitives:
//!
//!  - [`LiveReload`] for SSE-based browser auto-reload.
//!  - [`spawn_watcher`](webui_dev_server::spawn_watcher) for debounced
//!    filesystem notifications.
//!  - [`spawn_rebuild_worker`](webui_dev_server::spawn_rebuild_worker)
//!    for the rebuild loop (tick coalescing, success/error reporting,
//!    livereload broadcast).
//!  - [`serve_static_file`](webui_dev_server::serve_static_file) for
//!    `basePath`-aware static file routing with HTML livereload
//!    injection.
//!
//! Press-specific responsibilities that stay in this file:
//!  - re-reading and parsing `config.json` on every rebuild so live edits
//!    take effect,
//!  - calling [`build_docs`](crate::build::build_docs) and reusing the
//!    syntect highlighter across rebuilds via [`BuildCache`],
//!  - serving the flat `out_dir` (the dev-server crate handles the
//!    actual HTTP machinery — this module just wires it up).

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use actix_web::{web, App, HttpRequest, HttpResponse, HttpServer};
use anyhow::{anyhow, Context, Result};
use console::style;
use webui_dev_server::path::normalize_base_path;
use webui_dev_server::{
    default_ignore_paths, serve_static_file, spawn_rebuild_worker, spawn_watcher, sse_handler,
    LiveReload, NotFoundStrategy, StaticServeConfig, WatchConfig, WatcherHandle,
};

use crate::build::{build_docs_with_cache, BuildCache};
use crate::types::DocsConfig;

/// Filesystem-event debounce window. Editors often save in multiple bursts;
/// a single rebuild per burst feels right.
const DEBOUNCE_DURATION: Duration = Duration::from_millis(50);

/// SSE endpoint path. Absolute (root-relative) so it's not affected by
/// `<base href>` set in built pages.
const RELOAD_PATH: &str = "/__webui_press/livereload";

/// Inputs needed to start the dev server.
pub struct ServeConfig {
    pub config: DocsConfig,
    pub config_dir: PathBuf,
    pub template_dir: PathBuf,
    pub config_path: PathBuf,
    pub host: String,
    pub port: u16,
}

/// Run the dev server until interrupted.
pub async fn run_serve(opts: ServeConfig) -> Result<()> {
    let ServeConfig {
        config,
        config_dir,
        template_dir,
        config_path,
        host,
        port,
    } = opts;
    let base_path = normalize_base_path(&config.base_path);
    // Match `build_docs` semantics: `out_dir`, `content_dir`, and
    // `public_dir` are interpreted relative to the process working
    // directory, not relative to config_dir.
    let out_dir = PathBuf::from(&config.out_dir);
    let content_dir_str = config.content_dir.clone();
    let public_dir_str = config.public_dir.clone();

    println!(
        "{} {}",
        style("⚡").cyan().bold(),
        style("WebUI Press dev server").bold()
    );
    println!();

    // Initial build (synchronous so the server starts with a populated
    // dist). The cache is carried into the rebuild worker so subsequent
    // ticks reuse the syntect highlighter (the only state worth
    // amortizing across rebuilds — every other build step runs from
    // scratch).
    let initial_cache: BuildCache = {
        let cfg = clone_config_via_reparse(&config_path)?;
        let cd = config_dir.clone();
        let td = template_dir.clone();
        tokio::task::spawn_blocking(move || -> Result<BuildCache> {
            let mut cache = BuildCache::new();
            build_docs_with_cache(&cfg, &cd, &td, &mut cache)
                .map_err(|e| anyhow!("initial build failed: {e}"))?;
            // Subsequent rebuilds suppress the per-step banner output
            // (the rolling rebuild line replaces it) and skip cleaning
            // the output dir — overwriting in place is faster and
            // avoids macOS-ENOTEMPTY races. Stale files from deleted
            // source pages survive until the server is restarted.
            cache.set_dev_rebuild();
            Ok(cache)
        })
        .await
        .map_err(|e| anyhow!("build task panicked: {e}"))??
    };
    // Borrow the original config only for setup (basePath, paths) — every
    // rebuild re-reads from disk so live edits to config.json take effect.
    let _ = config;

    let livereload = LiveReload::new(RELOAD_PATH);

    // Cache lives across rebuilds. Wrapped in Arc<Mutex<_>> so the
    // worker thread can mutate it; the only contender is the worker
    // itself, so the mutex is uncontended in practice.
    let cache = Arc::new(Mutex::new(initial_cache));

    let tick_tx = {
        let config_path = config_path.clone();
        let config_dir = config_dir.clone();
        let template_dir = template_dir.clone();
        let cache = cache.clone();
        spawn_rebuild_worker(livereload.clone(), move || {
            let cfg = clone_config_via_reparse(&config_path)
                .map_err(|e| format!("config reload failed: {e}"))?;
            let mut guard = cache
                .lock()
                .map_err(|_| "cache mutex poisoned".to_string())?;
            build_docs_with_cache(&cfg, &config_dir, &template_dir, &mut guard)
                .map_err(|e| format!("{e}"))?;
            Ok(())
        })
    };

    let _watcher: WatcherHandle = {
        let tx = tick_tx.clone();
        let watched = watch_paths(
            &config_dir,
            &config_path,
            Path::new(&content_dir_str),
            Path::new(&public_dir_str),
        );
        let mut ignore = default_ignore_paths();
        ignore.push(out_dir.clone());
        ignore.push(PathBuf::from(".webui-press/cache"));
        spawn_watcher(
            WatchConfig {
                paths: watched,
                ignore,
                debounce: DEBOUNCE_DURATION,
            },
            move |_paths: Vec<PathBuf>| {
                // Coalesce on full: the worker drains all queued ticks
                // before starting a build, so a dropped tick is
                // equivalent to a coalesced one.
                let _ = tx.try_send(());
            },
        )?
    };

    let static_cfg = StaticServeConfig {
        root: out_dir.clone(),
        base_path: base_path.clone(),
        livereload: livereload.clone(),
        not_found: NotFoundStrategy::File(PathBuf::from("404.html")),
    };
    let static_data = web::Data::new(static_cfg);
    let lr_data = web::Data::new(livereload.clone());

    let bind = format!("{host}:{port}");
    let url = format!("http://{host}:{port}{base_path}");
    println!(
        "  {} {}",
        style("➜").green().bold(),
        style(&url).cyan().underlined()
    );
    println!();

    HttpServer::new(move || {
        App::new()
            .app_data(static_data.clone())
            .app_data(lr_data.clone())
            .route(RELOAD_PATH, web::get().to(sse_handler))
            .default_service(web::get().to(static_handler))
    })
    .bind(&bind)
    .with_context(|| format!("Cannot bind {bind}"))?
    .run()
    .await
    .context("Dev server failed")?;

    // Hold the watcher until the server returns so it isn't dropped early.
    drop(_watcher);
    Ok(())
}

/// Default actix handler — delegates to the shared static-file
/// responder. A trivial wrapper because actix needs a function pointer
/// and the shared crate ships an `&HttpRequest`-taking helper.
async fn static_handler(req: HttpRequest, cfg: web::Data<StaticServeConfig>) -> HttpResponse {
    serve_static_file(&req, cfg.get_ref()).await
}

// ─────────────────────────────────────────────────────────────────────────────
// Watch-path selection
// ─────────────────────────────────────────────────────────────────────────────

/// Compute the deduped set of filesystem paths the dev server should
/// watch. The watcher tracks every path that contributes to the rendered
/// output:
///
///  - `config_dir` — components, theme, and other assets co-located with
///    `config.json`,
///  - the parent of `config_path` — only added if it isn't already
///    covered (it usually equals `config_dir`),
///  - `content_dir` — markdown source files, typically a sibling of
///    `config_dir` (e.g. `docs/` while config lives at
///    `docs/.webui-press/config.json`),
///  - `public_dir` — verbatim assets copied into the build, often
///    nested inside `config_dir`.
///
/// We deliberately do NOT watch the framework's `template_dir`: that
/// directory ships with the `webui-press` crate and only changes when
/// the binary itself is rebuilt, which already restarts the dev server.
///
/// A path is dropped if it is already covered (via `starts_with`) by
/// something we are already watching. When a new path is a parent of an
/// already-watched one, it replaces the descendant — this keeps `notify`
/// from receiving overlapping subscriptions.
fn watch_paths(
    config_dir: &Path,
    config_path: &Path,
    content_dir: &Path,
    public_dir: &Path,
) -> Vec<PathBuf> {
    let mut paths: Vec<PathBuf> = Vec::with_capacity(4);
    let mut push_dedup = |candidate: PathBuf| {
        if candidate.as_os_str().is_empty() {
            return;
        }
        // Canonicalize so that subset/superset relations compare on
        // identical, symlink-resolved roots — e.g. `./` vs an absolute
        // `/Users/.../docs` would otherwise look disjoint and watch
        // both. Falls back to the raw candidate when the path doesn't
        // exist yet (e.g. publicDir a user hasn't created).
        let candidate = candidate.canonicalize().unwrap_or(candidate);
        if paths.iter().any(|p| candidate.starts_with(p)) {
            return;
        }
        paths.retain(|p| !p.starts_with(&candidate));
        paths.push(candidate);
    };
    push_dedup(config_dir.to_path_buf());
    if let Some(parent) = config_path.parent() {
        push_dedup(parent.to_path_buf());
    }
    push_dedup(content_dir.to_path_buf());
    push_dedup(public_dir.to_path_buf());
    paths
}

/// Re-read and parse `config.json`. Used both to seed the initial build
/// and inside the rebuild worker so live edits to the config take effect.
fn clone_config_via_reparse(config_path: &Path) -> Result<DocsConfig> {
    let s = std::fs::read_to_string(config_path)
        .with_context(|| format!("Cannot read {}", config_path.display()))?;
    serde_json::from_str(&s).with_context(|| format!("Invalid JSON in {}", config_path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn watch_paths_dedupes_nested_paths() {
        let cfg = Path::new("/proj/docs/.webui-press");
        let config_file = Path::new("/proj/docs/.webui-press/config.json");
        let content = Path::new("/proj/docs/.webui-press");
        let public = Path::new("/proj/docs/.webui-press/public");
        let paths = watch_paths(cfg, config_file, content, public);
        // config_path parent, content_dir, public_dir are all inside
        // config_dir → only config_dir watched.
        assert_eq!(paths, vec![PathBuf::from("/proj/docs/.webui-press")]);
    }

    #[test]
    fn watch_paths_includes_sibling_content_dir() {
        // Real-world layout: config lives in docs/.webui-press/config.json,
        // markdown lives in docs/ (a parent / sibling of .webui-press).
        let cfg = Path::new("docs/.webui-press");
        let config_file = Path::new("docs/.webui-press/config.json");
        let content = Path::new("docs");
        let public = Path::new("docs/.webui-press/public");
        let paths = watch_paths(cfg, config_file, content, public);
        // content_dir (docs) is a parent of everything else → it
        // promotes to the only watched root.
        assert_eq!(paths, vec![PathBuf::from("docs")]);
    }

    #[test]
    fn watch_paths_includes_external_content_dir() {
        // content_dir lives outside config_dir.
        let cfg = Path::new("/proj/.webui-press");
        let config_file = Path::new("/proj/.webui-press/config.json");
        let content = Path::new("/proj/markdown");
        let public = Path::new("/proj/.webui-press/public");
        let paths = watch_paths(cfg, config_file, content, public);
        assert_eq!(
            paths,
            vec![
                PathBuf::from("/proj/.webui-press"),
                PathBuf::from("/proj/markdown"),
            ]
        );
    }
}

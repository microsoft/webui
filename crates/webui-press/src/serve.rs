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
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use actix_web::{web, App, HttpRequest, HttpResponse, HttpServer};
use anyhow::{anyhow, Context, Result};
use console::style;
use webui_dev_server::path::normalize_base_path;
use webui_dev_server::shutdown::Control;
use webui_dev_server::{
    default_ignore_paths, serve_static_file, spawn_rebuild_worker, spawn_watcher, sse_handler,
    LiveReload, NotFoundStrategy, StaticServeConfig, TickSender, WatchConfig, WatchTargets,
    WatcherHandle,
};

use crate::build::{build_docs_with_cache, BuildCache};
use crate::state::StateDependencies;
use crate::types::{DocsConfig, ShowMode};

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
    /// Explicit CLI display-mode override, reapplied after each config reload.
    pub show_override: Option<ShowMode>,
}

/// Run the dev server until interrupted.
pub async fn run_serve(opts: ServeConfig, control: Option<Control>) -> Result<()> {
    let ServeConfig {
        config,
        config_dir,
        template_dir,
        config_path,
        host,
        port,
        show_override,
    } = opts;
    let base_path = normalize_base_path(&config.base_path);
    // Match `build_docs` semantics: `out_dir`, `content_dir`, and
    // `public_dir` are interpreted relative to the process working
    // directory, not relative to config_dir.
    let out_dir = PathBuf::from(&config.out_dir);

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
        let cfg = clone_config_via_reparse(&config_path, show_override)?;
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
    let livereload = LiveReload::new(RELOAD_PATH);

    // Cache lives across rebuilds. Wrapped in Arc<Mutex<_>> so the
    // worker thread can mutate it; the only contender is the worker
    // itself, so the mutex is uncontended in practice.
    let cache = Arc::new(Mutex::new(initial_cache));
    let watcher = Arc::new(Mutex::new(None::<WatcherHandle>));
    let sender = Arc::new(OnceLock::<TickSender>::new());

    let worker = {
        let config_path = config_path.clone();
        let config_dir = config_dir.clone();
        let template_dir = template_dir.clone();
        let cache = cache.clone();
        let watcher = watcher.clone();
        let sender = sender.clone();
        spawn_rebuild_worker(livereload.clone(), move || {
            let cfg = clone_config_via_reparse(&config_path, show_override)
                .map_err(|e| format!("config reload failed: {e}"))?;
            let mut guard = cache
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            let result = build_docs_with_cache(&cfg, &config_dir, &template_dir, &mut guard);
            let tx = sender
                .get()
                .ok_or_else(|| "Press watcher sender is unavailable".to_string())?;
            {
                let mut slot = watcher
                    .lock()
                    .map_err(|error| format!("Press watcher lock failed: {error}"))?;
                let handle = slot
                    .as_mut()
                    .ok_or_else(|| "Press watcher is unavailable".to_string())?;
                refresh_source_watcher(
                    handle,
                    source_targets(&cfg, &config_dir, &config_path, &guard.dependencies),
                    &guard.dependencies,
                    tx,
                )
                .map_err(|error| format!("Cannot refresh Press input watches: {error}"))?;
            }
            result.map_err(|e| format!("{e}"))?;
            // webui-press has no per-rebuild advisories to surface.
            Ok(Vec::new())
        })
    };

    let tx = worker.sender();
    sender
        .set(tx.clone())
        .map_err(|_| anyhow!("Press watcher sender was already initialized"))?;
    {
        let guard = cache
            .lock()
            .map_err(|error| anyhow!("Press build cache lock failed: {error}"))?;
        let initial_watcher =
            source_watcher(&config, &config_dir, &config_path, &guard.dependencies, tx)?;
        *watcher
            .lock()
            .map_err(|error| anyhow!("Press watcher lock failed: {error}"))? =
            Some(initial_watcher);
    }

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

    let mut server = HttpServer::new(move || {
        App::new()
            .app_data(static_data.clone())
            .app_data(lr_data.clone())
            .route(RELOAD_PATH, web::get().to(sse_handler))
            .default_service(web::get().to(static_handler))
    });
    if control.is_some() {
        server = server.disable_signals();
    }
    let server = server
        .bind(&bind)
        .with_context(|| format!("Cannot bind {bind}"))?
        .run();
    let server_result: Result<()> = match control {
        Some(control) => control.serve(server).await.map_err(Into::into),
        None => server.await.map_err(Into::into),
    };
    let server_result = server_result.context("Dev server failed");

    worker.shutdown()?;
    *watcher
        .lock()
        .map_err(|error| anyhow!("Press watcher lock failed: {error}"))? = None;
    server_result?;
    Ok(())
}

/// Start the input watcher for the build that produced `dependencies`.
fn source_watcher(
    config: &DocsConfig,
    config_dir: &Path,
    config_path: &Path,
    dependencies: &StateDependencies,
    tx: TickSender,
) -> Result<WatcherHandle> {
    let events = tx.clone();
    let mut watcher = spawn_watcher(
        WatchConfig {
            paths: Vec::new(),
            explicit_files: Vec::new(),
            ignore: Vec::new(),
            debounce: DEBOUNCE_DURATION,
            retry_unchanged_when: None,
        },
        move |paths| {
            let _ = events.try_send(paths);
        },
    )?;
    refresh_source_watcher(
        &mut watcher,
        source_targets(config, config_dir, config_path, dependencies),
        dependencies,
        &tx,
    )?;
    Ok(watcher)
}

/// Point the running watcher at the inputs of the build that just finished.
///
/// The watcher is retargeted rather than replaced: dropping it would discard
/// source edits it observed during the build but had not debounced yet.
fn refresh_source_watcher(
    watcher: &mut WatcherHandle,
    targets: WatchTargets,
    dependencies: &StateDependencies,
    tx: &TickSender,
) -> Result<()> {
    watcher.retarget(targets)?;
    // The previous targets may exclude a newly discovered include. Compare with
    // the bytes used for rendering only after the new subscriptions are active.
    let changed = dependencies.changed_paths();
    if !changed.is_empty() {
        match tx.try_send(changed) {
            Ok(()) | Err(std::sync::mpsc::TrySendError::Full(_)) => {}
            Err(std::sync::mpsc::TrySendError::Disconnected(_)) => {
                return Err(anyhow!("Press rebuild worker disconnected"));
            }
        }
    }
    Ok(())
}

/// Inputs that contribute to a build: source roots, generated includes, and
/// projection manifests.
fn source_targets(
    config: &DocsConfig,
    config_dir: &Path,
    config_path: &Path,
    dependencies: &StateDependencies,
) -> WatchTargets {
    let mut explicit_files = projection_manifest_paths(
        config_dir,
        config
            .bundler
            .as_ref()
            .map_or(&[][..], |bundler| bundler.projection_manifests.as_slice()),
    );
    explicit_files.extend_from_slice(&dependencies.paths);
    let mut ignore = default_ignore_paths();
    ignore.push(PathBuf::from(&config.out_dir));
    ignore.push(PathBuf::from(".webui-press/cache"));
    ignore.push(config_dir.join(crate::scratch::PROJECT_SCRATCH_DIR));
    WatchTargets {
        paths: watch_paths(
            config_dir,
            config_path,
            Path::new(&config.content_dir),
            Path::new(&config.public_dir),
        ),
        explicit_files,
        ignore,
    }
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

fn projection_manifest_paths(config_dir: &Path, manifests: &[String]) -> Vec<PathBuf> {
    manifests
        .iter()
        .map(|manifest| {
            let path = config_dir.join(manifest);
            path.canonicalize().unwrap_or(path)
        })
        .collect()
}

/// Re-read and parse `config.json`. Used both to seed the initial build
/// and inside the rebuild worker so live edits to the config take effect.
fn clone_config_via_reparse(
    config_path: &Path,
    show_override: Option<ShowMode>,
) -> Result<DocsConfig> {
    let s = std::fs::read_to_string(config_path)
        .with_context(|| format!("Cannot read {}", config_path.display()))?;
    let mut config: DocsConfig = serde_json::from_str(&s)
        .with_context(|| format!("Invalid JSON in {}", config_path.display()))?;
    if let Some(show) = show_override {
        config.show = show;
    }
    Ok(config)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn watcher_tracks_external_appearance_sources() -> Result<()> {
        let project = tempfile::tempdir()?;
        let external = tempfile::tempdir()?;
        let file = external.path().join("node_modules/theme/tokens.json");
        std::fs::create_dir_all(file.parent().context("theme parent")?)?;
        std::fs::write(
            &file,
            r#"{"themes":{"light":{"size":"1px"},"dark":{"size":"2px"}}}"#,
        )?;
        let mut config: DocsConfig = serde_json::from_str(
            r#"{"site":{"title":"Test"},"basePath":"/","contentDir":"missing","nav":[],"sidebar":[],"appearance":{"defaultTheme":"official","themes":[{"id":"official","label":"Official"}]}}"#,
        )?;
        config
            .appearance
            .as_mut()
            .context("appearance config")?
            .themes[0]
            .source = Some(file.to_string_lossy().into_owned());
        let mut dependencies = StateDependencies::default();
        crate::appearance::PreparedAppearance::prepare(&config, project.path(), &mut dependencies)?;
        let (tx, rx) = std::sync::mpsc::sync_channel(8);
        let _watcher = source_watcher(
            &config,
            project.path(),
            &project.path().join("config.json"),
            &dependencies,
            tx,
        )?;
        std::fs::write(
            &file,
            r#"{"themes":{"light":{"size":"3px"},"dark":{"size":"4px"}}}"#,
        )?;
        let changed = rx.recv_timeout(Duration::from_secs(3))?;
        assert!(changed.contains(&file.canonicalize()?));
        Ok(())
    }

    #[test]
    fn watcher_ignores_project_scratch_but_rebuilds_authored_sources() -> Result<()> {
        let root = tempfile::tempdir()?;
        let config_dir = root.path().join(".webui-press");
        let scratch_dir = config_dir.join(crate::scratch::PROJECT_SCRATCH_DIR);
        std::fs::create_dir_all(&scratch_dir)?;
        let generated = scratch_dir.join("index.html");
        let authored = root.path().join("index.md");
        std::fs::write(&generated, "generated")?;
        std::fs::write(&authored, "# Home")?;
        let mut config: DocsConfig = serde_json::from_str(
            r#"{"site":{"title":"Test"},"basePath":"/","contentDir":".","nav":[],"sidebar":[]}"#,
        )?;
        config.content_dir = root.path().to_string_lossy().into_owned();
        config.public_dir = config.content_dir.clone();
        let (tx, rx) = std::sync::mpsc::sync_channel(8);
        let _watcher = source_watcher(
            &config,
            &config_dir,
            &config_dir.join("config.json"),
            &crate::state::StateLoader::new().into_dependencies(),
            tx,
        )?;

        std::fs::write(&generated, "rebuilt")?;
        assert!(matches!(
            rx.recv_timeout(Duration::from_millis(500)),
            Err(std::sync::mpsc::RecvTimeoutError::Timeout)
        ));
        std::fs::remove_file(&generated)?;
        assert!(matches!(
            rx.recv_timeout(Duration::from_millis(500)),
            Err(std::sync::mpsc::RecvTimeoutError::Timeout)
        ));

        std::fs::write(&authored, "# Updated home")?;
        let changed = rx.recv_timeout(Duration::from_secs(3))?;
        assert_eq!(changed.len(), 1);
        assert_eq!(changed[0].canonicalize()?, authored.canonicalize()?);
        Ok(())
    }

    #[test]
    fn watcher_requeues_state_changed_after_the_render_read() -> Result<()> {
        let root = tempfile::tempdir()?;
        let file = root.path().join("target/state.json");
        std::fs::create_dir(file.parent().context("state parent")?)?;
        std::fs::write(&file, r#"{"count":1}"#)?;
        let mut loader = crate::state::StateLoader::new();
        loader.load_markdown_state(
            Some("target/state.json"),
            None,
            &root.path().join("page.md"),
        )?;
        let dependencies = loader.into_dependencies();
        std::fs::write(&file, r#"{"count":2}"#)?;
        let config: DocsConfig = serde_json::from_str(
            r#"{"site":{"title":"Test"},"basePath":"/","contentDir":"missing","nav":[],"sidebar":[]}"#,
        )?;
        let (tx, rx) = std::sync::mpsc::sync_channel(8);
        let _watcher = source_watcher(
            &config,
            root.path(),
            &root.path().join("config.json"),
            &dependencies,
            tx,
        )?;
        assert_eq!(
            rx.recv_timeout(Duration::from_millis(200))?,
            dependencies.paths
        );
        Ok(())
    }

    #[test]
    fn source_edits_during_a_watch_refresh_still_rebuild() -> Result<()> {
        let root = tempfile::tempdir()?;
        let content = root.path().join("content");
        std::fs::create_dir(&content)?;
        let page = content.join("page.md");
        std::fs::write(&page, "# One")?;
        let config_dir = root.path().join(".webui-press");
        std::fs::create_dir(&config_dir)?;
        let mut config: DocsConfig = serde_json::from_str(
            r#"{"site":{"title":"Test"},"basePath":"/","contentDir":".","nav":[],"sidebar":[]}"#,
        )?;
        config.content_dir = content.to_string_lossy().into_owned();
        config.public_dir = config.content_dir.clone();
        let dependencies = crate::state::StateLoader::new().into_dependencies();
        let config_path = config_dir.join("config.json");
        let (tx, rx) = std::sync::mpsc::sync_channel(8);
        let mut watcher = source_watcher(
            &config,
            &config_dir,
            &config_path,
            &dependencies,
            tx.clone(),
        )?;

        // A rebuild that read the old page finishes and refreshes its watches
        // before the debounce window reports this edit.
        std::fs::write(&page, "# Two")?;
        refresh_source_watcher(
            &mut watcher,
            source_targets(&config, &config_dir, &config_path, &dependencies),
            &dependencies,
            &tx,
        )?;
        let changed = rx.recv_timeout(Duration::from_secs(5))?;
        assert_eq!(changed.len(), 1);
        assert_eq!(changed[0].canonicalize()?, page.canonicalize()?);
        Ok(())
    }

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

    #[test]
    fn projection_manifest_paths_resolve_from_config_directory() {
        let paths = projection_manifest_paths(
            Path::new("/proj/docs/.webui-press"),
            &["../../client/dist/webui-projection.json".to_string()],
        );

        assert_eq!(
            paths,
            [PathBuf::from(
                "/proj/docs/.webui-press/../../client/dist/webui-projection.json"
            )]
        );
    }
}

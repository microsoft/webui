// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

//! Compiling `src/index.html` into a protocol, and the sample data it renders.
//!
//! None of this is streaming-specific — every WebUI example builds a protocol
//! and hands it render-time state the same way. It lives here so `main.rs`
//! stays about the streaming response.

use std::path::Path;
use std::sync::Arc;

use anyhow::{Context, Result};
use serde_json::{Map, Value};
use webui::{
    build, load_token_file, resolve_theme_path, BuildOptions, CssStrategy, Plugin,
    ProjectionManifestSource, Protocol,
};
use webui_tokens::{inject_into_state, resolve_tokens};

use crate::ENTRY;

const THEME: &str = "@microsoft/webui-examples-theme";

/// Everything [`load`] produces: the compiled protocol, its render-time
/// state, and the component stylesheets the build emitted.
///
/// `css_files` never reaches `dist/` — the client build only emits
/// JavaScript — so the caller must serve it for the `link` and `module` CSS
/// strategies.
pub(crate) struct LoadedApp {
    pub(crate) protocol: Arc<Protocol>,
    pub(crate) state: Value,
    pub(crate) css_files: Vec<(String, String)>,
}

/// Build the WebUI protocol and its render-time state from `app_root`.
///
/// The theme is loaded once and passed into `BuildOptions::theme` so the
/// build validates every CSS custom property this app's templates and
/// components reference (see `webui_tokens::validate_required_tokens`),
/// then re-resolved against the build's actual token usage to produce the
/// per-theme CSS injected into `state.tokens.{light,dark}` for the
/// `{{{tokens.light}}}` / `{{{tokens.dark}}}` bindings in `index.html`.
///
/// The client build's `dist/webui-projection.json` is passed through so the
/// protocol uses `InitialStateStrategy::Components`. This is load-bearing for
/// streaming: without it every boundary checkpoint falls back to serializing
/// the *entire* state object, so each of the five envelopes would re-ship all
/// three feed batches plus both resolved token sheets. With it, a checkpoint
/// carries only the hydration keys its own components declare, which is the
/// "boundary payload locality" contract in DESIGN.md ("Progressive Streaming
/// Hydration").
pub(crate) fn load(app_root: &Path, css: CssStrategy, base_path: &str) -> Result<LoadedApp> {
    let theme_path = resolve_theme_path(THEME, app_root)
        .with_context(|| format!("Failed to resolve theme {THEME}"))?;
    let token_file = load_token_file(&theme_path)
        .with_context(|| format!("Failed to load theme tokens from {}", theme_path.display()))?;

    // `main` already requires `dist/index.js`, so a served build always has
    // the manifest. Tolerate its absence so `cargo check`/unit runs that
    // never execute the client build still succeed.
    let manifest_path = app_root.join("dist").join("webui-projection.json");
    let projection_manifests = if manifest_path.is_file() {
        vec![ProjectionManifestSource::Path(manifest_path)]
    } else {
        Vec::new()
    };

    let build_result = build(BuildOptions {
        app_dir: app_root.join("src"),
        entry: ENTRY.to_string(),
        css,
        plugin: Some(Plugin::WebUI),
        theme: Some(token_file.clone()),
        projection_manifests,
        ..BuildOptions::default()
    })
    .context("Failed to build the streaming example WebUI protocol")?;

    let resolved_tokens = resolve_tokens(&build_result.protocol.tokens, &token_file)
        .context("Failed to resolve design tokens for the loaded theme")?;
    let css_files = build_result.css_files;
    let protocol = Protocol::new(build_result.protocol);

    let mut state_map = Map::with_capacity(4);
    state_map.insert("basePath".to_owned(), Value::String(base_path.to_owned()));
    for (index, batch) in feed_batches().into_iter().enumerate() {
        state_map.insert(format!("feedBatch{}", index + 1), Value::Array(batch));
    }
    let mut state = Value::Object(state_map);
    inject_into_state(&mut state, &resolved_tokens);

    Ok(LoadedApp {
        protocol: Arc::new(protocol),
        state,
        css_files,
    })
}

/// Parse the `--css` flag.
///
/// # Errors
///
/// Returns an error naming the accepted values if `value` is not one of them.
pub(crate) fn css_strategy(value: &str) -> Result<CssStrategy> {
    match value {
        "link" => Ok(CssStrategy::Link),
        "style" => Ok(CssStrategy::Style),
        "module" => Ok(CssStrategy::Module),
        other => {
            anyhow::bail!("Unknown CSS strategy: {other}. Use \"link\", \"style\", or \"module\".")
        }
    }
}

fn feed_post(post_id: &str, author: &str, text: &str, like_count: &str) -> Value {
    let mut post = Map::with_capacity(4);
    post.insert("postId".to_owned(), Value::String(post_id.to_owned()));
    post.insert("author".to_owned(), Value::String(author.to_owned()));
    post.insert("text".to_owned(), Value::String(text.to_owned()));
    post.insert("likeCount".to_owned(), Value::String(like_count.to_owned()));
    Value::Object(post)
}

/// Explicit, complete feed batches, one per `<boundary>` in `src/index.html`.
///
/// Each is committed with its own checkpoint and flush. DESIGN.md's
/// "Outside the current design" names an open-ended `<webui-stream>` append
/// directive; until that exists, a fixed batch per boundary is how a feed
/// streams.
fn feed_batches() -> Vec<Vec<Value>> {
    vec![
        vec![
            feed_post(
                "1",
                "Ada",
                "Streaming boundaries keep the composer interactive immediately.",
                "4",
            ),
            feed_post(
                "2",
                "Grace",
                "This is feed batch one — it just committed.",
                "1",
            ),
        ],
        vec![feed_post(
            "3",
            "Alan",
            "Feed batch two arrived after its own checkpoint.",
            "9",
        )],
        vec![feed_post(
            "4",
            "Barbara",
            "Feed batch three is the lowest-priority chunk.",
            "0",
        )],
    ]
}

#[cfg(test)]
mod tests {
    use super::{css_strategy, feed_batches};
    use crate::pacing::FEED_BATCH_COUNT;
    use webui::CssStrategy;

    #[test]
    fn css_strategy_accepts_known_values() {
        assert_eq!(
            css_strategy("style").unwrap_or_else(|e| panic!("style: {e}")),
            CssStrategy::Style
        );
        assert_eq!(
            css_strategy("link").unwrap_or_else(|e| panic!("link: {e}")),
            CssStrategy::Link
        );
        assert_eq!(
            css_strategy("module").unwrap_or_else(|e| panic!("module: {e}")),
            CssStrategy::Module
        );
    }

    #[test]
    fn css_strategy_rejects_unknown_values() {
        assert!(css_strategy("bogus").is_err());
    }

    /// The pacing schedule hard-codes how many gaps precede feed batches, so
    /// adding a batch here without a matching `<boundary>` and schedule
    /// update would silently stop pacing the last one.
    #[test]
    fn the_batch_count_matches_the_pacing_schedule() {
        assert_eq!(feed_batches().len(), FEED_BATCH_COUNT);
    }

    #[test]
    fn every_post_carries_the_fields_feed_item_binds() {
        for batch in feed_batches() {
            for post in batch {
                for field in ["postId", "author", "text", "likeCount"] {
                    let value = post.get(field).and_then(serde_json::Value::as_str);
                    assert!(
                        value.is_some_and(|text| !text.is_empty()),
                        "a feed post is missing {field}"
                    );
                }
            }
        }
    }
}

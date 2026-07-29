// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

//! `<link rel="modulepreload">` hints for the critical entry's shared chunks.
//!
//! Splitting the weather island into its own entry point (see
//! `../../build-client.mjs`) moves the framework runtime into a chunk that
//! `index.js` statically imports. The browser's preload scanner sees the
//! `<script src="./index.js">` tag in `<head>`, but it cannot see the chunk
//! behind it — that import is only discoverable after `index.js` has been
//! downloaded and parsed, which costs a full round trip on the critical path.
//!
//! Measured on this example over a throttled link (100 ms RTT, 1.6 Mbps,
//! deterministic feed pacing, 12 cold contexts, median composer
//! time-to-interactive):
//!
//! | Variant                          | Composer interactive |
//! | -------------------------------- | -------------------- |
//! | Island bundled into `index.js`   | 1074 ms              |
//! | Island split, no preload hint    | 1061 ms              |
//! | Island split, preload hint       | 955 ms               |
//!
//! The hint is what makes the split pay: it lets the 9.8 KiB entry and the
//! 35.8 KiB shared chunk download concurrently instead of serially.
//!
//! Chunk filenames are content-hashed, so they cannot be written into
//! `index.html` by hand. The client build records them in
//! `dist/critical-modules.json` and this module renders them into the
//! `{{{modulePreloads}}}` binding once at startup.

use anyhow::{bail, Context, Result};
use std::path::Path;

/// Bound on the manifest so a corrupt or hostile file cannot be turned into
/// unbounded `<head>` markup.
const MAX_CRITICAL_CHUNKS: usize = 32;

/// Read `dist/critical-modules.json` and render one `<link rel="modulepreload">`
/// per chunk the critical entry statically imports.
///
/// Returns an empty string when the manifest is absent, so a `dist/` produced
/// by an older build still serves — just without the hint.
pub(crate) fn render_module_preloads(dist_dir: &Path) -> Result<String> {
    let manifest = dist_dir.join("critical-modules.json");
    let Ok(contents) = std::fs::read_to_string(&manifest) else {
        return Ok(String::new());
    };

    let chunks: Vec<String> = serde_json::from_str(&contents)
        .with_context(|| format!("Failed to parse {}", manifest.display()))?;
    render_preload_links(&chunks, &manifest.display().to_string())
}

/// Render preload links for already-parsed chunk paths.
fn render_preload_links(chunks: &[String], source: &str) -> Result<String> {
    if chunks.len() > MAX_CRITICAL_CHUNKS {
        bail!(
            "{source} lists {} critical chunks, more than the {MAX_CRITICAL_CHUNKS} this example \
             supports",
            chunks.len()
        );
    }

    // One reusable buffer sized for the whole run rather than a `String` per
    // chunk; this is startup code, but the pattern is the one the framework
    // holds itself to.
    const OPEN: &str = "<link rel=\"modulepreload\" href=\"";
    const CLOSE: &str = "\">";
    let capacity = chunks
        .iter()
        .map(|chunk| chunk.len() + OPEN.len() + CLOSE.len())
        .sum();
    let mut markup = String::with_capacity(capacity);

    for chunk in chunks {
        // The manifest is build output, not user input, but it is still read
        // off disk — reject anything that could break out of the attribute
        // rather than escaping it, since a legitimate chunk path never
        // contains these bytes.
        if chunk.is_empty()
            || chunk
                .bytes()
                .any(|b| matches!(b, b'"' | b'\'' | b'<' | b'>' | b'&') || b.is_ascii_control())
        {
            bail!("{source} contains a chunk path that is not safe to emit: {chunk:?}");
        }
        markup.push_str(OPEN);
        markup.push_str(chunk);
        markup.push_str(CLOSE);
    }

    Ok(markup)
}

#[cfg(test)]
mod tests {
    use super::{render_module_preloads, render_preload_links, MAX_CRITICAL_CHUNKS};
    use std::fs;
    use std::path::PathBuf;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temp_dir(label: &str) -> PathBuf {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |d| d.as_nanos());
        std::env::temp_dir().join(format!(
            "webui-streaming-preload-{label}-{}-{unique}",
            std::process::id()
        ))
    }

    #[test]
    fn renders_one_link_per_chunk_in_manifest_order() {
        let markup = render_preload_links(
            &[
                "./chunk-AAAAAAAA.js".to_owned(),
                "./chunk-BBBB.js".to_owned(),
            ],
            "test",
        )
        .unwrap_or_else(|e| panic!("{e}"));

        assert_eq!(
            markup,
            "<link rel=\"modulepreload\" href=\"./chunk-AAAAAAAA.js\">\
             <link rel=\"modulepreload\" href=\"./chunk-BBBB.js\">"
        );
    }

    #[test]
    fn no_chunks_renders_nothing() {
        let markup = render_preload_links(&[], "test").unwrap_or_else(|e| panic!("{e}"));
        assert!(markup.is_empty());
    }

    /// A missing manifest must not stop the server: the page still works, it
    /// just pays the waterfall this module exists to remove.
    #[test]
    fn missing_manifest_is_not_an_error() {
        let dir = temp_dir("missing");
        let markup = render_module_preloads(&dir).unwrap_or_else(|e| panic!("{e}"));
        assert!(markup.is_empty());
    }

    #[test]
    fn manifest_on_disk_is_read_and_rendered() {
        let dir = temp_dir("present");
        fs::create_dir_all(&dir).unwrap_or_else(|e| panic!("{e}"));
        fs::write(
            dir.join("critical-modules.json"),
            "[\"./chunk-NKNSLYVV.js\"]",
        )
        .unwrap_or_else(|e| panic!("{e}"));

        let markup = render_module_preloads(&dir).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(
            markup,
            "<link rel=\"modulepreload\" href=\"./chunk-NKNSLYVV.js\">"
        );

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn malformed_manifest_is_reported_not_ignored() {
        let dir = temp_dir("malformed");
        fs::create_dir_all(&dir).unwrap_or_else(|e| panic!("{e}"));
        fs::write(dir.join("critical-modules.json"), "{ not an array }")
            .unwrap_or_else(|e| panic!("{e}"));

        assert!(render_module_preloads(&dir).is_err());

        let _ = fs::remove_dir_all(&dir);
    }

    /// The manifest is emitted into raw `<head>` markup through a
    /// triple-brace binding, so a path that could close the attribute must be
    /// rejected rather than escaped.
    #[test]
    fn attribute_breaking_paths_are_rejected() {
        for hostile in [
            "./a\">.js",
            "./a<script>.js",
            "./a'.js",
            "./a&b.js",
            "./a\nb.js",
            "",
        ] {
            assert!(
                render_preload_links(&[hostile.to_owned()], "test").is_err(),
                "expected {hostile:?} to be rejected"
            );
        }
    }

    #[test]
    fn absurd_chunk_counts_are_rejected() {
        let chunks = vec!["./chunk.js".to_owned(); MAX_CRITICAL_CHUNKS + 1];
        assert!(render_preload_links(&chunks, "test").is_err());
    }
}

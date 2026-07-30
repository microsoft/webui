// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

//! Resolve `<link rel="modulepreload">` targets for a page's critical module
//! entries.
//!
//! A bundler splits shared code into chunks that are named only *inside* the
//! entry's own bytes, so the browser's preload scanner cannot see them: it
//! must download and parse the entry first, then discover the chunk, then
//! fetch it. That serialization is worth roughly 100 ms on a mid-latency
//! connection, and preload hints remove it.
//!
//! The two inputs meet here. The parser reports each authored
//! `<script type="module" src>` outside a `<boundary>`; the projection
//! manifest reports each bundler entry's transitive static import closure,
//! already ordered largest-first. This module joins them and produces the
//! ordered, servable href list the handler writes into `<head>`.
//!
//! **Order is the whole point.** Preloads are issued in document order over a
//! shared connection, so a small chunk listed ahead of a large one delays the
//! long pole. Measured on the streaming example: largest-first 956 ms,
//! smallest-first 1076 ms, no hints at all 1061 ms. Getting the order wrong is
//! worse than emitting nothing, which is exactly why this is the framework's
//! job and not the application author's.

use std::collections::BTreeMap;

use webui_parser::codes;
use webui_parser::Diagnostic;

/// Upper bound on emitted hints.
///
/// Preloads compete for the same connection, so a runaway list would starve
/// the entry itself. Real closures are 1-5 chunks; anything approaching this
/// is a build mistake, not a page that needs more hints.
const MAX_HINTS: usize = 32;

/// Ordered `modulepreload` hrefs plus advisories for entries that could not be
/// resolved.
#[derive(Debug, Default)]
pub(crate) struct ModulePreloads {
    /// Servable hrefs in the exact order they must be emitted.
    pub(crate) hrefs: Vec<String>,
    /// Build warnings for entries that carry hints in principle but could not
    /// be matched to exactly one manifest output.
    pub(crate) warnings: Vec<Diagnostic>,
}

/// Join parser-observed module entries with manifest-recorded import closures.
///
/// `entry_srcs` is in document order; `entry_closures` is keyed by
/// build-root-relative output path. Returns hrefs in emission order,
/// deduplicated across entries by first occurrence, so a chunk shared by two
/// entries is preloaded once at its earliest (largest) position.
pub(crate) fn resolve(
    entry_closures: &BTreeMap<String, Vec<String>>,
    entry_srcs: &[String],
) -> ModulePreloads {
    let mut result = ModulePreloads::default();
    if entry_closures.is_empty() || entry_srcs.is_empty() {
        return result;
    }

    for src in entry_srcs {
        let Some((src_dir, src_file)) = split_href(src) else {
            continue;
        };

        let mut matched: Option<&str> = None;
        for key in entry_closures.keys() {
            if file_name(key) != src_file {
                continue;
            }
            if matched.is_some() {
                // Two outputs share this basename, so nothing here proves
                // which one the URL serves. Guessing risks preloading chunks
                // from the wrong app, which costs a round trip and a 404.
                result.warnings.push(ambiguous_entry_warning(src));
                matched = None;
                break;
            }
            matched = Some(key);
        }
        let Some(entry_key) = matched else {
            continue;
        };

        let key_dir = &entry_key[..entry_key.len() - src_file.len()];
        for member in &entry_closures[entry_key] {
            // Rewriting the manifest's directory prefix to the URL's is the
            // same resolution the browser performs for the entry's real
            // `import` statements. A member outside that directory cannot be
            // expressed this way, and a wrong href costs more than a missing
            // one, so it is skipped.
            let Some(relative) = member.strip_prefix(key_dir) else {
                continue;
            };
            if result.hrefs.len() == MAX_HINTS {
                result.warnings.push(too_many_hints_warning(src));
                return result;
            }
            let href = concat_href(src_dir, relative);
            if !is_attribute_safe(&href) {
                // The handler writes these straight into a quoted attribute
                // with no per-request escaping, so anything that could close
                // the attribute is rejected here rather than encoded there.
                // Manifest keys are canonical relative paths, which still
                // permits characters a real bundler never emits.
                result.warnings.push(unsafe_href_warning(&href));
                continue;
            }
            if !result.hrefs.contains(&href) {
                result.hrefs.push(href);
            }
        }
    }
    result
}

/// Split a script `src` into its directory prefix (with trailing separator)
/// and file name.
///
/// Returns `None` for anything this module cannot reason about: an absolute or
/// protocol-relative URL points at an origin whose layout the manifest does not
/// describe, and a query or fragment means the served path is not the path.
fn split_href(src: &str) -> Option<(&str, &str)> {
    if src.contains("//") || src.contains(':') || src.contains('?') || src.contains('#') {
        return None;
    }
    let file = file_name(src);
    if file.is_empty() {
        return None;
    }
    Some((&src[..src.len() - file.len()], file))
}

#[inline]
fn file_name(path: &str) -> &str {
    match path.rfind('/') {
        Some(index) => &path[index + 1..],
        None => path,
    }
}

fn concat_href(dir: &str, relative: &str) -> String {
    let mut href = String::with_capacity(dir.len() + relative.len());
    href.push_str(dir);
    href.push_str(relative);
    href
}

/// Whether an href can be written verbatim into a double-quoted HTML
/// attribute.
///
/// Deliberately an allow-list of what real bundler output contains rather than
/// a deny-list of what breaks, so a character nobody anticipated fails closed.
fn is_attribute_safe(href: &str) -> bool {
    href.bytes().all(|byte| {
        byte.is_ascii_alphanumeric() || matches!(byte, b'/' | b'.' | b'-' | b'_' | b'~' | b'+')
    })
}

/// Cold: at most one per rejected href, and only for a file name no bundler
/// produces.
#[cold]
#[inline(never)]
fn unsafe_href_warning(href: &str) -> Diagnostic {
    Diagnostic::warning(format!(
        "skipped the modulepreload hint for '{href}' because its path contains \
         characters that are unsafe to write into an HTML attribute"
    ))
    .code(codes::UNSAFE_MODULE_PRELOAD)
    .help("restrict build output file names to letters, digits, '.', '-', '_', and '/'")
}

/// Cold: at most one per unresolvable entry, and only on a misconfigured build.
#[cold]
#[inline(never)]
fn ambiguous_entry_warning(src: &str) -> Diagnostic {
    Diagnostic::warning(format!(
        "cannot preload the shared chunks of '{src}' because more than one \
         build output has that file name"
    ))
    .code(codes::AMBIGUOUS_MODULE_ENTRY)
    .help(
        "give each application's bundler entry a distinct output file name, or \
         merge their projection manifests so only one build owns this page",
    )
}

/// Cold: at most one per build, and only for an implausibly large closure.
#[cold]
#[inline(never)]
fn too_many_hints_warning(src: &str) -> Diagnostic {
    Diagnostic::warning(format!(
        "stopped after {MAX_HINTS} modulepreload hints while resolving '{src}'"
    ))
    .code(codes::EXCESSIVE_MODULE_PRELOADS)
    .help(
        "preloads share one connection, so past a few dozen they delay the \
         entry itself; reduce code splitting or load later chunks from inside \
         the boundary that needs them",
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn closures(pairs: &[(&str, &[&str])]) -> BTreeMap<String, Vec<String>> {
        pairs
            .iter()
            .map(|(entry, members)| {
                (
                    (*entry).to_string(),
                    members.iter().map(|m| (*m).to_string()).collect(),
                )
            })
            .collect()
    }

    fn srcs(values: &[&str]) -> Vec<String> {
        values.iter().map(|v| (*v).to_string()).collect()
    }

    #[test]
    fn rewrites_manifest_keys_into_served_urls_preserving_order() {
        let map = closures(&[(
            "examples/app/streaming/dist/index.js",
            &[
                "examples/app/streaming/dist/chunk-big.js",
                "examples/app/streaming/dist/chunk-small.js",
            ],
        )]);
        let result = resolve(&map, &srcs(&["/index.js"]));

        assert_eq!(result.hrefs, ["/chunk-big.js", "/chunk-small.js"]);
        assert!(result.warnings.is_empty());
    }

    #[test]
    fn preserves_a_nested_url_prefix() {
        let map = closures(&[("build/out/app.js", &["build/out/vendor.js"])]);
        let result = resolve(&map, &srcs(&["/static/js/app.js"]));

        assert_eq!(result.hrefs, ["/static/js/vendor.js"]);
    }

    #[test]
    fn a_shared_chunk_is_preloaded_once_at_its_earliest_position() {
        let map = closures(&[
            ("d/a.js", &["d/shared.js", "d/only-a.js"]),
            ("d/b.js", &["d/shared.js", "d/only-b.js"]),
        ]);
        let result = resolve(&map, &srcs(&["/a.js", "/b.js"]));

        assert_eq!(
            result.hrefs,
            ["/shared.js", "/only-a.js", "/only-b.js"],
            "the duplicate must not push the connection budget twice"
        );
    }

    #[test]
    fn an_ambiguous_basename_warns_instead_of_guessing() {
        let map = closures(&[
            ("apps/one/dist/index.js", &["apps/one/dist/a.js"]),
            ("apps/two/dist/index.js", &["apps/two/dist/b.js"]),
        ]);
        let result = resolve(&map, &srcs(&["/index.js"]));

        assert!(
            result.hrefs.is_empty(),
            "a wrong preload is worse than none"
        );
        assert_eq!(result.warnings.len(), 1);
        assert!(result.warnings[0].body().contains("/index.js"));
    }

    #[test]
    fn an_unknown_entry_is_silently_skipped() {
        let map = closures(&[("d/a.js", &["d/shared.js"])]);
        let result = resolve(&map, &srcs(&["/vendor-analytics.js"]));

        assert!(result.hrefs.is_empty());
        assert!(
            result.warnings.is_empty(),
            "a third-party script is not a build mistake"
        );
    }

    #[test]
    fn cross_origin_and_parameterized_srcs_are_skipped() {
        let map = closures(&[("d/a.js", &["d/shared.js"])]);
        for src in [
            "https://cdn.example.com/a.js",
            "//cdn.example.com/a.js",
            "/a.js?v=2",
            "/a.js#x",
        ] {
            let result = resolve(&map, &srcs(&[src]));
            assert!(result.hrefs.is_empty(), "{src} must not resolve");
        }
    }

    #[test]
    fn members_outside_the_entry_directory_are_skipped() {
        let map = closures(&[("d/a.js", &["d/inside.js", "elsewhere/outside.js"])]);
        let result = resolve(&map, &srcs(&["/a.js"]));

        assert_eq!(
            result.hrefs,
            ["/inside.js"],
            "an unmappable member must not become a guessed href"
        );
    }

    #[test]
    fn hints_are_capped_and_the_cap_is_reported() {
        let members: Vec<String> = (0..MAX_HINTS + 5).map(|i| format!("d/c{i}.js")).collect();
        let mut map = BTreeMap::new();
        map.insert("d/a.js".to_string(), members);
        let result = resolve(&map, &srcs(&["/a.js"]));

        assert_eq!(result.hrefs.len(), MAX_HINTS);
        assert_eq!(result.warnings.len(), 1);
    }

    #[test]
    fn an_attribute_unsafe_href_is_dropped_not_escaped() {
        let map = closures(&[("d/a.js", &[r#"d/we"ird.js"#, "d/fine.js"])]);
        let result = resolve(&map, &srcs(&["/a.js"]));

        assert_eq!(
            result.hrefs,
            ["/fine.js"],
            "the handler writes hrefs raw, so unsafe ones must never reach it"
        );
        assert_eq!(result.warnings.len(), 1);
    }

    #[test]
    fn no_manifest_or_no_entries_produces_nothing() {
        assert!(resolve(&BTreeMap::new(), &srcs(&["/a.js"]))
            .hrefs
            .is_empty());
        assert!(resolve(&closures(&[("d/a.js", &["d/s.js"])]), &[])
            .hrefs
            .is_empty());
    }
}

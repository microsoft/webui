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
//! Compiler-generated entries use the same resolver with exact physical output
//! identities and host-provided cache-busted URLs.
//!
//! **Order is the whole point.** Preloads are issued in document order over a
//! shared connection, so a small chunk listed ahead of a large one delays the
//! long pole. Measured on the streaming example: largest-first 956 ms,
//! smallest-first 1076 ms, no hints at all 1061 ms. Getting the order wrong is
//! worse than emitting nothing, which is exactly why this is the framework's
//! job and not the application author's.

use std::collections::{BTreeMap, HashSet};
use std::path::PathBuf;

use crate::projection::EntryClosure;
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
    entry_closures: &BTreeMap<PathBuf, EntryClosure>,
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

        let mut matched: Option<&EntryClosure> = None;
        for closure in entry_closures.values() {
            if file_name(&closure.entry_key) != src_file {
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
            matched = Some(closure);
        }
        let Some(closure) = matched else {
            continue;
        };

        for member in &closure.members {
            // Resolve from the entry's manifest-relative directory exactly as
            // the browser resolves the entry's own import specifiers. Sibling
            // output directories become safe `../` href segments rather than
            // being silently discarded.
            let relative = relative_member_path(&closure.entry_key, &member.key);
            if result.hrefs.len() == MAX_HINTS {
                result.warnings.push(too_many_hints_warning(src));
                return result;
            }
            let href = concat_href(src_dir, &relative);
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

/// Resolve compiler-generated entries through exact output identities.
///
/// Unlike authored script URLs, generated URLs may carry cache-busting query
/// parameters. The host supplies the complete identity-to-URL map, so this
/// path never strips parameters or guesses from a basename.
pub(crate) fn resolve_exact(
    entry_closures: &BTreeMap<PathBuf, EntryClosure>,
    existing_hrefs: &[String],
    entry_outputs: &[&std::path::Path],
    output_urls: &BTreeMap<PathBuf, String>,
) -> ModulePreloads {
    let mut result = ModulePreloads {
        hrefs: existing_hrefs.iter().take(MAX_HINTS).cloned().collect(),
        warnings: Vec::new(),
    };
    for entry_output in entry_outputs {
        let Some(closure) = entry_closures.get(*entry_output) else {
            continue;
        };
        for member in &closure.members {
            let Some(href) = output_urls.get(&member.identity) else {
                // An incomplete map cannot prove a servable URL. Missing a
                // hint is safer than synthesizing one from a filesystem path.
                continue;
            };
            if result.hrefs.contains(href) {
                continue;
            }
            if result.hrefs.len() == MAX_HINTS {
                result
                    .warnings
                    .push(too_many_hints_warning(&closure.entry_key));
                return result;
            }
            if !is_generated_attribute_safe(href) {
                result.warnings.push(unsafe_href_warning(href));
                continue;
            }
            result.hrefs.push(href.clone());
        }
    }
    result
}

pub(crate) fn exact_output_identities(
    entry_closures: &BTreeMap<PathBuf, EntryClosure>,
    entry_outputs: &[&std::path::Path],
) -> Vec<PathBuf> {
    let mut seen = HashSet::new();
    let mut outputs = Vec::new();
    for entry_output in entry_outputs {
        let Some(closure) = entry_closures.get(*entry_output) else {
            continue;
        };
        for member in &closure.members {
            if seen.insert(member.identity.as_path()) {
                outputs.push(member.identity.clone());
            }
        }
    }
    outputs
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

fn relative_member_path(entry: &str, member: &str) -> String {
    let entry_dir = entry.rsplit_once('/').map_or("", |(dir, _)| dir);
    let common_segments = entry_dir
        .split('/')
        .zip(member.split('/'))
        .take_while(|(left, right)| left == right)
        .count();
    let entry_segments = if entry_dir.is_empty() {
        0
    } else {
        entry_dir.split('/').count()
    };
    let parent_segments = entry_segments.saturating_sub(common_segments);
    let member_bytes = member
        .split('/')
        .skip(common_segments)
        .map(str::len)
        .sum::<usize>();
    let remaining_segments = member.split('/').count().saturating_sub(common_segments);
    let mut relative = String::with_capacity(
        parent_segments * 3 + member_bytes + remaining_segments.saturating_sub(1),
    );
    for _ in 0..parent_segments {
        relative.push_str("../");
    }
    for (index, segment) in member.split('/').skip(common_segments).enumerate() {
        if index > 0 {
            relative.push('/');
        }
        relative.push_str(segment);
    }
    relative
}

/// Whether an href can be written verbatim into a double-quoted HTML
/// attribute.
///
/// Deliberately an allow-list of what real bundler output contains rather than
/// a deny-list of what breaks, so a character nobody anticipated fails closed.
fn is_attribute_safe(href: &str) -> bool {
    href.bytes().all(is_attribute_safe_byte)
}

fn is_generated_attribute_safe(href: &str) -> bool {
    href.bytes()
        .all(|byte| is_attribute_safe_byte(byte) || matches!(byte, b'?' | b'='))
}

#[inline]
fn is_attribute_safe_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || matches!(byte, b'/' | b'.' | b'-' | b'_' | b'~' | b'+')
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
    use crate::projection::{EntryClosure, EntryClosureMember};
    use std::path::Path;

    fn closures(pairs: &[(&str, &[&str])]) -> BTreeMap<PathBuf, EntryClosure> {
        pairs
            .iter()
            .map(|(entry, members)| {
                (
                    PathBuf::from(entry),
                    EntryClosure {
                        entry_key: (*entry).to_string(),
                        members: members
                            .iter()
                            .map(|member| EntryClosureMember {
                                key: (*member).to_string(),
                                identity: Path::new(member).to_path_buf(),
                            })
                            .collect(),
                    },
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
    fn an_empty_owner_still_makes_a_basename_ambiguous() {
        let map = closures(&[
            ("apps/one/dist/index.js", &[]),
            ("apps/two/dist/index.js", &["apps/two/dist/b.js"]),
        ]);
        let result = resolve(&map, &srcs(&["/index.js"]));

        assert!(
            result.hrefs.is_empty(),
            "an entry with no imports still owns its basename"
        );
        assert_eq!(result.warnings.len(), 1);
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
    fn members_in_sibling_output_directories_use_relative_hrefs() {
        let map = closures(&[(
            "app/dist/a.js",
            &["app/dist/inside.js", "app/chunks/outside.js"],
        )]);
        let result = resolve(&map, &srcs(&["/static/dist/a.js"]));

        assert_eq!(
            result.hrefs,
            [
                "/static/dist/inside.js",
                "/static/dist/../chunks/outside.js"
            ],
            "the href must preserve the manifest's directory relationship"
        );
    }

    #[test]
    fn hints_are_capped_and_the_cap_is_reported() {
        let members: Vec<String> = (0..MAX_HINTS + 5).map(|i| format!("d/c{i}.js")).collect();
        let mut map = BTreeMap::new();
        map.insert(
            PathBuf::from("d/a.js"),
            EntryClosure {
                entry_key: "d/a.js".to_string(),
                members: members
                    .into_iter()
                    .map(|key| EntryClosureMember {
                        identity: PathBuf::from(&key),
                        key,
                    })
                    .collect(),
            },
        );
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

    #[test]
    fn compiler_generated_root_and_page_entries_use_exact_versioned_urls() {
        let map = closures(&[
            ("out/index.js", &["out/shared.js"]),
            (
                "out/assets/page-0.js",
                &["out/assets/page.js", "out/shared.js"],
            ),
        ]);
        let output_urls = BTreeMap::from([
            (
                PathBuf::from("out/shared.js"),
                "/docs/shared.js?v=shared".to_string(),
            ),
            (
                PathBuf::from("out/assets/page.js"),
                "/docs/assets/page.js?v=page".to_string(),
            ),
            (
                // A matching basename at the wrong identity must not be used.
                PathBuf::from("other/shared.js"),
                "/wrong/shared.js?v=wrong".to_string(),
            ),
        ]);
        let entries = [Path::new("out/index.js"), Path::new("out/assets/page-0.js")];

        assert_eq!(
            exact_output_identities(&map, &entries),
            [
                PathBuf::from("out/shared.js"),
                PathBuf::from("out/assets/page.js")
            ]
        );
        let result = resolve_exact(&map, &[], &entries, &output_urls);

        assert_eq!(
            result.hrefs,
            ["/docs/shared.js?v=shared", "/docs/assets/page.js?v=page"]
        );
        assert!(result.warnings.is_empty());
    }

    #[test]
    fn compiler_generated_entries_do_not_guess_missing_output_urls() {
        let map = closures(&[("out/index.js", &["out/shared.js"])]);
        let output_urls = BTreeMap::from([(
            PathBuf::from("other/shared.js"),
            "/wrong/shared.js?v=wrong".to_string(),
        )]);

        let result = resolve_exact(&map, &[], &[Path::new("out/index.js")], &output_urls);

        assert!(result.hrefs.is_empty());
        assert!(result.warnings.is_empty());
    }
}

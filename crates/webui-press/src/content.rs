// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

//! Content pipeline: reads markdown files, parses frontmatter, builds page state.

use std::collections::HashMap;
use std::fs;
use std::path::Path;

use serde_json::{Map, Value};

use crate::error::{Error, Result};
use crate::markdown::{render_markdown, Highlighter};
use crate::types::{DocsConfig, PageDescriptor, SidebarItem, SidebarSection};

/// Normalize a config link (e.g. `/guide/intro/` or `/guide/intro`) to a
/// canonical URL path that includes the site's `base_path` prefix and
/// **never** has a trailing slash (except for root, which is just the base).
/// GitHub Pages and most static hosts auto-redirect `/foo` → `/foo/index.html`,
/// so trailing slashes are unnecessary and pollute internal hrefs.
fn normalize_link(base: &str, link: &str) -> String {
    if link.is_empty() {
        return String::new();
    }
    // Tolerate config links written without a leading slash; also avoids a
    // panic when `link` starts with a multi-byte UTF-8 char (a bare `&link[1..]`
    // would slice mid-char and abort the build).
    let stripped = link.strip_prefix('/').unwrap_or(link);
    if link.contains('#') {
        // Fragment link — preserve original shape, don't touch the slash.
        return format!("{base}{stripped}");
    }
    let cleaned = stripped.trim_end_matches('/');
    if cleaned.is_empty() {
        // Root: `base` already ends with '/'.
        base.to_string()
    } else {
        format!("{base}{cleaned}")
    }
}

/// Build a JSON object from key-value pairs without using `json!` (which calls `unwrap`).
fn json_obj<const N: usize>(entries: [(&str, Value); N]) -> Value {
    let mut map = Map::with_capacity(N);
    for (k, v) in entries {
        map.insert(k.to_string(), v);
    }
    Value::Object(map)
}

/// Top-level state keys reserved by the docs renderer. Custom-page `stateFile`
/// objects whose top-level fields are flattened onto the page state must not
/// shadow these names — collisions are silently skipped so the canonical docs
/// state always wins.
const RESERVED_STATE_KEYS: &[&str] = &[
    "site",
    "navigation",
    "sidebar",
    "page",
    "hero",
    "footer",
    "prev",
    "next",
    "pageData",
];

/// Parsed frontmatter from a markdown file.
#[derive(Debug)]
struct Frontmatter {
    title: Option<String>,
    description: Option<String>,
    layout: Option<String>,
}

fn parse_frontmatter(content: &str) -> Result<(Frontmatter, &str)> {
    if !content.starts_with("---\n") && !content.starts_with("---\r\n") {
        return Ok((
            Frontmatter {
                title: None,
                description: None,
                layout: None,
            },
            content,
        ));
    }

    // Skip leading "---" + line ending.
    let body_start_marker = if content.starts_with("---\r\n") { 5 } else { 4 };
    let after_open = &content[body_start_marker..];

    // Find the closing "---" on its own line (preceded by `\n` or `\r\n`).
    let (yaml_end_offset, close_skip) = if let Some(i) = after_open.find("\r\n---") {
        (i, "\r\n---".len())
    } else if let Some(i) = after_open.find("\n---") {
        (i, "\n---".len())
    } else {
        (after_open.len(), 0)
    };

    let yaml_str = &after_open[..yaml_end_offset];
    let body_offset = body_start_marker + yaml_end_offset + close_skip;
    let body = if body_offset < content.len() {
        // Strip the line-ending immediately following the closing "---".
        let rest = &content[body_offset..];
        rest.strip_prefix("\r\n")
            .or_else(|| rest.strip_prefix('\n'))
            .unwrap_or(rest)
    } else {
        ""
    };

    let yaml: HashMap<String, serde_yaml::Value> = serde_yaml::from_str(yaml_str)
        .map_err(|e| crate::error::Error::Markdown(format!("Invalid frontmatter YAML: {e}")))?;

    Ok((
        Frontmatter {
            title: yaml.get("title").and_then(|v| v.as_str()).map(String::from),
            description: yaml
                .get("description")
                .and_then(|v| v.as_str())
                .map(String::from),
            layout: yaml
                .get("layout")
                .and_then(|v| v.as_str())
                .map(String::from),
        },
        body,
    ))
}

fn extract_h1(content: &str) -> Option<String> {
    for line in content.lines() {
        if let Some(heading) = line.strip_prefix("# ") {
            return Some(heading.trim().to_string());
        }
    }
    None
}

/// Collect all links from a sidebar item and its children (iterative).
fn collect_sidebar_links(items: &[SidebarItem]) -> Vec<&str> {
    let mut links = Vec::new();
    let mut stack: Vec<&SidebarItem> = items.iter().rev().collect();
    while let Some(item) = stack.pop() {
        if !item.link.is_empty() {
            links.push(item.link.as_str());
        }
        for child in item.items.iter().rev() {
            stack.push(child);
        }
    }
    links
}

/// Build the page registry by walking content_dir for every `.md` file.
///
/// Discovery is filesystem-driven: any markdown file under content_dir becomes
/// a page. The sidebar/nav config controls navigation, not discovery.
/// Strip redundant filename if it matches the parent folder name.
/// E.g., "my-button/my-button" → "my-button"
///      "my-button/usage" → "my-button/usage" (unchanged)
fn normalize_path_as_index(path: &str) -> &str {
    // Find the last slash to split parent/filename
    let Some(slash_pos) = path.rfind('/') else {
        return path;
    };
    
    let parent = &path[..slash_pos];
    let filename = &path[slash_pos + 1..];
    
    // Get the parent folder name (last segment before filename)
    let folder_name = parent.rsplit('/').next().unwrap_or("");
    
    // If folder name matches filename, treat as index and strip filename
    if folder_name == filename {
        parent
    } else {
        path
    }
}

fn build_page_registry(content_dir: &Path, base_path: &str) -> Vec<(String, std::path::PathBuf)> {
    let mut pages = Vec::new();
    let mut stack: Vec<std::path::PathBuf> = vec![content_dir.to_path_buf()];

    while let Some(dir) = stack.pop() {
        let Ok(entries) = fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            // Skip dotfiles/dotdirs and the build output.
            let name = entry.file_name();
            let name_str = name.to_string_lossy();
            if name_str.starts_with('.') || name_str == "dist" || name_str == "node_modules" {
                continue;
            }
            if path.is_dir() {
                stack.push(path);
                continue;
            }
            if path.extension().and_then(|s| s.to_str()) != Some("md") {
                continue;
            }

            // Compute URL: relative path from content_dir, drop trailing /index.md,
            // drop .md, prefix with base_path.
            let rel = match path.strip_prefix(content_dir) {
                Ok(r) => r,
                Err(_) => continue,
            };
            let rel_str = rel.to_string_lossy().replace('\\', "/");
            let trimmed = rel_str.trim_end_matches(".md");
            
            // Strip /index suffix
            let trimmed = trimmed.strip_suffix("/index").unwrap_or(trimmed);
            
            // Strip redundant filename if it matches parent folder
            // e.g., "webui-button/webui-button" → "webui-button"
            let trimmed = normalize_path_as_index(trimmed);
            
            // Root index.md => "/"
            let url_path = if trimmed.is_empty() || trimmed == "index" {
                if base_path.is_empty() {
                    "/".to_string()
                } else {
                    base_path.trim_end_matches('/').to_string() + "/"
                }
            } else {
                let prefix = base_path.trim_end_matches('/');
                format!("{prefix}/{trimmed}")
            };
            pages.push((url_path, path));
        }
    }

    pages.sort_by(|a, b| a.0.cmp(&b.0));
    pages
}

fn build_sidebar_state(url_path: &str, config: &DocsConfig) -> serde_json::Value {
    let base = &config.base_path;

    // Determine active sidebar
    let active_sidebar = config
        .sidebar_groups
        .iter()
        .find(|(prefix, _)| {
            let full_prefix = format!("{}{}", base, prefix.strip_prefix('/').unwrap_or(prefix));
            let trimmed = full_prefix.trim_end_matches('/');
            url_path == trimmed || url_path.starts_with(&format!("{trimmed}/"))
        })
        .map(|(_, s)| s.as_slice())
        .unwrap_or(&config.sidebar);

    // Convert a SidebarItem to a JSON value (iterative via stack for children)
    let item_to_value = |item: &SidebarItem| -> Value {
        let item_path = normalize_link(base, &item.link);
        let active = !item_path.is_empty() && item_path == url_path;
        let children: Vec<Value> = item
            .items
            .iter()
            .map(|child| {
                let child_path = normalize_link(base, &child.link);
                let child_active = !child_path.is_empty() && child_path == url_path;
                json_obj([
                    ("text", Value::String(child.text.clone())),
                    ("link", Value::String(child_path)),
                    ("active", Value::Bool(child_active)),
                    ("hasChildren", Value::Bool(false)),
                    ("children", Value::Array(vec![])),
                ])
            })
            .collect();
        json_obj([
            ("text", Value::String(item.text.clone())),
            ("link", Value::String(item_path)),
            ("active", Value::Bool(active)),
            ("hasChildren", Value::Bool(!children.is_empty())),
            ("children", Value::Array(children)),
        ])
    };

    let sections: Vec<Value> = active_sidebar
        .iter()
        .map(|section| {
            let items: Vec<Value> = section.items.iter().map(item_to_value).collect();
            json_obj([
                ("title", Value::String(section.title.clone())),
                ("items", Value::Array(items)),
            ])
        })
        .collect();

    json_obj([("sections", Value::Array(sections))])
}

fn find_sidebar_text(url_path: &str, config: &DocsConfig) -> Option<String> {
    let base = &config.base_path;
    let all: Vec<&[SidebarSection]> = std::iter::once(config.sidebar.as_slice())
        .chain(config.sidebar_groups.values().map(|v| v.as_slice()))
        .collect();

    for sidebar in all {
        for section in sidebar {
            // Use iterative stack to search nested items
            let mut stack: Vec<&SidebarItem> = section.items.iter().collect();
            while let Some(item) = stack.pop() {
                if !item.link.is_empty() {
                    let item_path = normalize_link(base, &item.link);
                    if item_path == url_path {
                        return Some(item.text.clone());
                    }
                }
                for child in &item.items {
                    stack.push(child);
                }
            }
        }
    }
    None
}

/// Resolve and load every custom page's `stateFile` once, returning a map keyed
/// by the custom-page link (e.g. `/playground/`) holding the parsed JSON value.
/// Inline `state` values are passed through untouched.
///
/// State files are cached by canonical filesystem path so that two custom pages
/// pointing at the same file only read and parse it once.
fn load_custom_page_states(
    config: &DocsConfig,
    config_dir: &Path,
) -> Result<HashMap<String, Value>> {
    let mut cache: HashMap<std::path::PathBuf, Value> = HashMap::new();
    let mut out: HashMap<String, Value> = HashMap::with_capacity(config.custom_pages.len());

    for (link, page) in &config.custom_pages {
        let inline = page.inline_state();
        let path = page.state_file();

        if inline.is_some() && path.is_some() {
            return Err(crate::error::Error::Build(format!(
                "Custom page {link}: 'state' and 'stateFile' are mutually exclusive — pick one."
            )));
        }

        if let Some(value) = inline {
            out.insert(link.clone(), value.clone());
            continue;
        }

        if let Some(rel) = path {
            let abs = config_dir.join(rel);
            let key = fs::canonicalize(&abs).unwrap_or_else(|_| abs.clone());
            let value = if let Some(cached) = cache.get(&key) {
                cached.clone()
            } else {
                let raw = fs::read_to_string(&abs).map_err(|e| {
                    crate::error::Error::Build(format!(
                        "Custom page {link}: cannot read stateFile {}: {e}",
                        abs.display()
                    ))
                })?;
                let parsed: Value = serde_json::from_str(&raw).map_err(|e| {
                    crate::error::Error::Build(format!(
                        "Custom page {link}: stateFile {} is not valid JSON: {e}",
                        abs.display()
                    ))
                })?;
                cache.insert(key, parsed.clone());
                parsed
            };
            out.insert(link.clone(), value);
        }
    }

    Ok(out)
}

/// Process all content files and return page descriptors.
///
/// `config_dir` is the directory containing `config.json`; relative paths
/// declared in the config (such as a custom page's `stateFile`) are resolved
/// against it.
///
/// `head_injection` is the fully-resolved `<head>` snippet (CSS link
/// tags + `config.head` entries). It MUST be computed before this call
/// so the descriptor is render-ready.
pub fn process_content(
    config: &DocsConfig,
    config_dir: &Path,
    highlighter: &Highlighter,
    head_injection: &str,
) -> Result<Vec<PageDescriptor>> {
    let content_dir = Path::new(&config.content_dir);
    let base_path = &config.base_path;

    // Load custom-page state once before the parallel pipeline.
    let custom_states = load_custom_page_states(config, config_dir)?;

    let nav_links: Vec<Value> = config
        .nav
        .iter()
        .map(|item| {
            let (link, section) = if item.link.starts_with("http") {
                // External links can never match a docs section. Use the URL
                // itself as a unique sentinel so the equality check below
                // never matches any page's section slug.
                (item.link.clone(), item.link.clone())
            } else {
                let full = format!("{}{}", base_path, &item.link[1..]);
                // Derive a section slug from the raw config link, e.g. "/guide/" -> "guide".
                let slug = item
                    .link
                    .trim_start_matches('/')
                    .split('/')
                    .next()
                    .unwrap_or("")
                    .to_string();
                (full, slug)
            };
            json_obj([
                ("text", Value::String(item.text.clone())),
                ("link", Value::String(link)),
                ("section", Value::String(section)),
            ])
        })
        .collect();

    let mut registry = build_page_registry(content_dir, base_path);

    // Register custom pages
    for page_link in config.custom_pages.keys() {
        let normalized = if page_link.ends_with('/') {
            page_link.clone()
        } else {
            format!("{}/", page_link)
        };
        let url_path = format!("{}{}", base_path, &normalized[1..]);
        let src_file = format!("{}index.md", &normalized[1..]);
        let full_path = content_dir.join(&src_file);
        if !registry.iter().any(|(p, _)| p == &url_path) {
            registry.push((url_path, full_path));
        }
    }

    use rayon::prelude::*;

    // Per-page processing: parse markdown / load custom HTML, expand
    // syntax highlighting, build the state JSON. Always runs for every
    // page on every call — there is no descriptor cache. The dev
    // server keeps a `BuildCache` only to amortize the syntect
    // highlighter load.
    let results: Vec<(usize, PageDescriptor)> = registry
        .par_iter()
        .enumerate()
        .map(
            |(idx, (url_path, full_path))| -> Result<(usize, PageDescriptor)> {
                let mut is_home = false;
                let mut html = String::new();
                let mut title = config.site.title.clone();
                let mut description = config.site.description.clone();
                let mut layout = "doc".to_string();

                // Check custom page override
                let logical_path = format!("/{}", &url_path[base_path.len()..]);
                let custom_entry = config
                    .custom_pages
                    .get(&logical_path)
                    .or_else(|| config.custom_pages.get(logical_path.trim_end_matches('/')));

                if let Some(entry) = custom_entry {
                    html = entry.html().to_string();
                    layout = entry.layout().to_string();
                } else if full_path.exists() {
                    let raw = fs::read_to_string(full_path)
                        .map_err(|e| crate::error::Error::Io(e.to_string()))?;
                    let (fm, body) = parse_frontmatter(&raw).map_err(|e| {
                        crate::error::Error::Markdown(format!("{}: {e}", full_path.display()))
                    })?;

                    is_home = fm.layout.as_deref() == Some("home");
                    layout = if is_home {
                        "home".to_string()
                    } else {
                        fm.layout.unwrap_or_else(|| "doc".to_string())
                    };

                    if !is_home {
                        html = render_markdown(body, highlighter, base_path)?;
                    }

                    if let Some(d) = fm.description {
                        description = d;
                    }

                    let h1 = extract_h1(body);
                    let sidebar_text = find_sidebar_text(url_path, config);
                    title = fm
                        .title
                        .or(h1)
                        .or(sidebar_text)
                        .unwrap_or_else(|| config.site.title.clone());
                }

                let hero_val = if is_home {
                    config
                        .hero
                        .as_ref()
                        .map(|h| {
                            json_obj([
                                (
                                    "text",
                                    h.text
                                        .as_ref()
                                        .map_or(Value::Null, |t| Value::String(t.clone())),
                                ),
                                ("tagline", Value::String(h.tagline.clone())),
                                (
                                    "manifesto",
                                    h.manifesto
                                        .as_ref()
                                        .map_or(Value::Null, |m| Value::String(m.clone())),
                                ),
                                (
                                    "actions",
                                    Value::Array(
                                        h.actions
                                            .iter()
                                            .map(|a| {
                                                json_obj([
                                                    ("text", Value::String(a.text.clone())),
                                                    ("link", Value::String(a.link.clone())),
                                                    ("brand", Value::Bool(a.brand)),
                                                ])
                                            })
                                            .collect(),
                                    ),
                                ),
                                (
                                    "features",
                                    Value::Array(
                                        h.features
                                            .iter()
                                            .map(|f| {
                                                json_obj([
                                                    ("icon", Value::String(f.icon.clone())),
                                                    ("title", Value::String(f.title.clone())),
                                                    (
                                                        "description",
                                                        Value::String(f.description.clone()),
                                                    ),
                                                ])
                                            })
                                            .collect(),
                                    ),
                                ),
                            ])
                        })
                        .unwrap_or(Value::Null)
                } else {
                    Value::Null
                };

                let footer_val = config
                    .footer
                    .as_ref()
                    .map(|f| json_obj([("html", Value::String(f.html.clone()))]))
                    .unwrap_or(Value::Null);

                // Top-level section slug for the current page, derived from the
                // first path segment after the base. Used by the nav template to
                // mark the active link via `?active="{{link.section == page.section}}"`.
                let page_section = logical_path
                    .trim_start_matches('/')
                    .split('/')
                    .next()
                    .unwrap_or("")
                    .to_string();

                let state = json_obj([
                    (
                        "site",
                        json_obj([
                            ("title", Value::String(config.site.title.clone())),
                            ("base", Value::String(base_path.to_string())),
                        ]),
                    ),
                    ("navigation", Value::Array(nav_links.clone())),
                    ("sidebar", build_sidebar_state(url_path, config)),
                    (
                        "page",
                        json_obj([
                            ("title", Value::String(title.clone())),
                            ("description", Value::String(description.clone())),
                            ("content", Value::String(html.clone())),
                            ("isHome", Value::Bool(is_home)),
                            ("layout", Value::String(layout)),
                            ("section", Value::String(page_section)),
                        ]),
                    ),
                    ("hero", hero_val),
                    ("footer", footer_val),
                    ("prev", Value::Null),
                    ("next", Value::Null),
                    (
                        "pageData",
                        custom_states
                            .get(&logical_path)
                            .or_else(|| custom_states.get(logical_path.trim_end_matches('/')))
                            .cloned()
                            .unwrap_or(Value::Null),
                    ),
                    ("headTags", Value::String(head_injection.to_string())),
                    ("label", Value::String("Copy".to_string())),
                    ("icon", Value::String("🌙".to_string())),
                ]);

                // Flatten the loaded custom-page state object onto the top-level
                // page state so component templates can bind directly to its
                // fields (e.g. `<for each="item in files">`). This is what enables
                // SSR for components driven by a `stateFile`. Reserved keys are
                // never overwritten.
                let state = if let Some(Value::Object(extra)) = custom_states
                    .get(&logical_path)
                    .or_else(|| custom_states.get(logical_path.trim_end_matches('/')))
                {
                    let mut map = match state {
                        Value::Object(m) => m,
                        other => {
                            let mut m = Map::new();
                            m.insert("_root".to_string(), other);
                            m
                        }
                    };
                    for (k, v) in extra {
                        if !RESERVED_STATE_KEYS.contains(&k.as_str()) && !map.contains_key(k) {
                            map.insert(k.clone(), v.clone());
                        }
                    }
                    Value::Object(map)
                } else {
                    state
                };

                Ok((
                    idx,
                    PageDescriptor {
                        path: url_path.clone(),
                        is_home,
                        state,
                    },
                ))
            },
        )
        .collect::<Result<Vec<_>>>()?;

    // Build prev/next links from the sidebar that matches each page's URL.
    // We resolve the active sidebar per-page by checking sidebar_groups
    // prefixes, then falling back to the default config.sidebar.
    let resolve_sidebar = |page_path: &str| -> Vec<SidebarItem> {
        let active = config
            .sidebar_groups
            .iter()
            .find(|(prefix, _)| {
                let full_prefix = format!(
                    "{}{}",
                    base_path,
                    prefix.strip_prefix('/').unwrap_or(prefix)
                );
                let trimmed = full_prefix.trim_end_matches('/');
                page_path == trimmed || page_path.starts_with(&format!("{trimmed}/"))
            })
            .map(|(_, s)| s.as_slice())
            .unwrap_or(&config.sidebar);
        active.iter().flat_map(|s| s.items.clone()).collect()
    };

    // Restore canonical (registry) order and apply prev/next to each
    // descriptor. Carrying the registry index explicitly through the
    // parallel pass avoids depending on `par_iter().collect()`'s
    // implicit order-preservation contract.
    let mut pages: Vec<Option<PageDescriptor>> = (0..results.len()).map(|_| None).collect();
    for (idx, mut desc) in results {
        let sidebar_items = resolve_sidebar(&desc.path);
        let flat_links = collect_sidebar_links(&sidebar_items);
        let all_links: Vec<(String, String)> = flat_links
            .iter()
            .map(|link| {
                let path = normalize_link(base_path, link);
                let text = find_sidebar_text(&path, config).unwrap_or_default();
                (path, text)
            })
            .collect();
        if let Some(pos) = all_links.iter().position(|(p, _)| p == &desc.path) {
            if pos > 0 {
                let (link, text) = &all_links[pos - 1];
                desc.state["prev"] = json_obj([
                    ("link", Value::String(link.clone())),
                    ("text", Value::String(text.clone())),
                ]);
            }
            if pos + 1 < all_links.len() {
                let (link, text) = &all_links[pos + 1];
                desc.state["next"] = json_obj([
                    ("link", Value::String(link.clone())),
                    ("text", Value::String(text.clone())),
                ]);
            }
        }
        pages[idx] = Some(desc);
    }
    let pages: Vec<PageDescriptor> = pages
        .into_iter()
        .enumerate()
        .map(|(i, slot)| {
            slot.ok_or_else(|| Error::Build(format!("missing page at registry index {i}")))
        })
        .collect::<Result<Vec<_>>>()?;

    Ok(pages)
}

#[cfg(test)]
mod tests {
    use super::*;

    // --- normalize_link --------------------------------------------------

    #[test]
    fn normalize_link_empty_returns_empty() {
        assert_eq!(normalize_link("/webui/", ""), "");
    }

    #[test]
    fn normalize_link_root_slash_returns_base() {
        assert_eq!(normalize_link("/webui/", "/"), "/webui/");
    }

    #[test]
    fn normalize_link_trims_trailing_slash() {
        assert_eq!(
            normalize_link("/webui/", "/guide/intro/"),
            "/webui/guide/intro"
        );
        assert_eq!(
            normalize_link("/webui/", "/guide/intro"),
            "/webui/guide/intro"
        );
    }

    #[test]
    fn normalize_link_preserves_fragment() {
        assert_eq!(
            normalize_link("/webui/", "/guide/intro#section"),
            "/webui/guide/intro#section"
        );
    }

    #[test]
    fn normalize_link_tolerates_missing_leading_slash() {
        // Was previously a slice-from-1 panic risk on multi-byte chars.
        assert_eq!(
            normalize_link("/webui/", "guide/intro"),
            "/webui/guide/intro"
        );
    }

    #[test]
    fn normalize_link_tolerates_non_ascii_first_char() {
        // `&link[1..]` would have panicked here on UTF-8 boundary.
        let out = normalize_link("/webui/", "é-page");
        assert!(out.ends_with("é-page"), "got {out}");
    }

    // --- normalize_path_as_index -----------------------------------------

    #[test]
    fn normalize_path_as_index_strips_duplicate_folder_name() {
        assert_eq!(
            normalize_path_as_index("webui-button/webui-button"),
            "webui-button"
        );
    }

    #[test]
    fn normalize_path_as_index_keeps_different_filename() {
        assert_eq!(
            normalize_path_as_index("webui-button/usage"),
            "webui-button/usage"
        );
    }

    #[test]
    fn normalize_path_as_index_nested_folders() {
        assert_eq!(
            normalize_path_as_index("components/webui-button/webui-button"),
            "components/webui-button"
        );
    }

    #[test]
    fn normalize_path_as_index_no_slash_returns_unchanged() {
        assert_eq!(
            normalize_path_as_index("webui-button"),
            "webui-button"
        );
    }

    #[test]
    fn normalize_path_as_index_deep_nesting_with_match() {
        assert_eq!(
            normalize_path_as_index("a/b/c/webui-card/webui-card"),
            "a/b/c/webui-card"
        );
    }

    #[test]
    fn normalize_path_as_index_deep_nesting_without_match() {
        assert_eq!(
            normalize_path_as_index("a/b/c/webui-card/examples"),
            "a/b/c/webui-card/examples"
        );
    }

    // --- parse_frontmatter -----------------------------------------------

    #[test]
    fn parse_frontmatter_no_frontmatter_returns_full_body() {
        let (fm, body) = parse_frontmatter("# Hello\nworld\n").expect("ok");
        assert!(fm.title.is_none());
        assert!(fm.layout.is_none());
        assert_eq!(body, "# Hello\nworld\n");
    }

    #[test]
    fn parse_frontmatter_valid_yaml_extracts_fields() {
        let raw = "---\ntitle: Hello\ndescription: World\nlayout: home\n---\n# Body\n";
        let (fm, body) = parse_frontmatter(raw).expect("ok");
        assert_eq!(fm.title.as_deref(), Some("Hello"));
        assert_eq!(fm.description.as_deref(), Some("World"));
        assert_eq!(fm.layout.as_deref(), Some("home"));
        assert_eq!(body, "# Body\n");
    }

    #[test]
    fn parse_frontmatter_handles_crlf_line_endings() {
        let raw = "---\r\ntitle: CRLF\r\nlayout: doc\r\n---\r\n# Body\r\n";
        let (fm, body) = parse_frontmatter(raw).expect("ok");
        assert_eq!(fm.title.as_deref(), Some("CRLF"));
        assert_eq!(fm.layout.as_deref(), Some("doc"));
        assert_eq!(body, "# Body\r\n");
    }

    #[test]
    fn parse_frontmatter_malformed_yaml_returns_error() {
        // `: : :` is not valid YAML.
        let raw = "---\ntitle: : : :\n---\nBody\n";
        let result = parse_frontmatter(raw);
        assert!(result.is_err(), "expected error, got {result:?}");
    }
}

//! Test utilities for WebUI framework.
//!
//! This crate provides testing helpers and should only be used in test code.

use std::fs;
use std::{collections::HashMap, path::PathBuf};
use tempfile::TempDir;
pub use webui_protocol;

/// A macro that wraps `serde_json::json!` but allows bypassing clippy::disallowed_methods.
///
/// This macro should only be used in test code.
#[macro_export]
macro_rules! test_json {
    ($($json:tt)+) => {{
        #[allow(clippy::disallowed_methods)]
        let value = serde_json::json!($($json)+);
        value
    }};
}

/// Assert that a fragment list matches the expected pattern.
///
/// Each matcher is one of:
/// - `raw("text")` — Raw fragment with exact value
/// - `signal("name")` — Escaped signal
/// - `signal("name", raw)` — Raw (unescaped) signal
/// - `attr("name", value: "v")` — Simple dynamic attribute
/// - `attr("name", template: "id")` — Template attribute
/// - `attr("name", complex: "v")` — Complex (:-prefixed) attribute
/// - `bool_attr("name", "signal")` — Boolean attribute with identifier condition
/// - `attr_raw("name", "v")` — Static attribute with rawValue
/// - `attr_raw("name", "v", attr_start)` — Static rawValue with attrStart
/// - `attr_skip("name", "v")` — Skipped attribute
/// - `component("id")` — Component fragment
/// - `for_loop("item", "collection", "template")` — For loop
/// - `if_cond("template")` — If condition
#[macro_export]
macro_rules! assert_fragments {
    ($fragments:expr, [ $($matcher:expr),* $(,)? ]) => {{
        let matchers: Vec<$crate::FragmentMatcher> = vec![$($matcher),*];
        $crate::assert_fragment_list(&$fragments, &matchers);
    }};
}

/// Assert that a named stream in the records matches the expected pattern.
#[macro_export]
macro_rules! assert_stream {
    ($records:expr, $stream_id:expr, [ $($matcher:expr),* $(,)? ]) => {{
        let stream = $records.get($stream_id)
            .unwrap_or_else(|| panic!("Missing stream: {}", $stream_id));
        let matchers: Vec<$crate::FragmentMatcher> = vec![$($matcher),*];
        $crate::assert_fragment_list(&stream.fragments, &matchers);
    }};
}

// ── Fragment matcher types ──────────────────────────────────────────

/// Describes an expected fragment for assertion matching.
#[derive(Debug)]
pub enum FragmentMatcher {
    Raw(String),
    Signal {
        value: String,
        raw: bool,
    },
    Attribute(AttrMatcher),
    Component(String),
    ForLoop {
        item: String,
        collection: String,
        template: String,
    },
    IfCond {
        template: String,
    },
}

/// Describes expected attribute properties.
#[derive(Debug, Default)]
pub struct AttrMatcher {
    pub name: String,
    pub value: Option<String>,
    pub template: Option<String>,
    pub complex: bool,
    pub attr_start: bool,
    pub attr_skip: bool,
    pub raw_value: bool,
    pub bool_signal: Option<String>,
}

// ── Matcher constructors ────────────────────────────────────────────

/// Match a raw fragment.
pub fn raw(value: &str) -> FragmentMatcher {
    FragmentMatcher::Raw(value.to_string())
}

/// Match an escaped signal fragment.
pub fn signal(value: &str) -> FragmentMatcher {
    FragmentMatcher::Signal {
        value: value.to_string(),
        raw: false,
    }
}

/// Match a raw (unescaped) signal fragment.
pub fn signal_raw(value: &str) -> FragmentMatcher {
    FragmentMatcher::Signal {
        value: value.to_string(),
        raw: true,
    }
}

/// Match a simple dynamic attribute.
pub fn attr(name: &str, value: &str) -> FragmentMatcher {
    FragmentMatcher::Attribute(AttrMatcher {
        name: name.to_string(),
        value: Some(value.to_string()),
        ..Default::default()
    })
}

/// Match a template (mixed) attribute.
pub fn attr_template(name: &str, template: &str) -> FragmentMatcher {
    FragmentMatcher::Attribute(AttrMatcher {
        name: name.to_string(),
        template: Some(template.to_string()),
        ..Default::default()
    })
}

/// Match a complex (:-prefixed) attribute.
pub fn attr_complex(name: &str, value: &str) -> FragmentMatcher {
    FragmentMatcher::Attribute(AttrMatcher {
        name: name.to_string(),
        value: Some(value.to_string()),
        complex: true,
        ..Default::default()
    })
}

/// Match a complex attribute with attrStart.
pub fn attr_complex_start(name: &str, value: &str) -> FragmentMatcher {
    FragmentMatcher::Attribute(AttrMatcher {
        name: name.to_string(),
        value: Some(value.to_string()),
        complex: true,
        attr_start: true,
        ..Default::default()
    })
}

/// Match a boolean attribute with an identifier condition.
pub fn bool_attr(name: &str, signal: &str) -> FragmentMatcher {
    FragmentMatcher::Attribute(AttrMatcher {
        name: name.to_string(),
        bool_signal: Some(signal.to_string()),
        ..Default::default()
    })
}

/// Match a boolean attribute with attrStart.
pub fn bool_attr_start(name: &str, signal: &str) -> FragmentMatcher {
    FragmentMatcher::Attribute(AttrMatcher {
        name: name.to_string(),
        bool_signal: Some(signal.to_string()),
        attr_start: true,
        ..Default::default()
    })
}

/// Match a static rawValue attribute.
pub fn attr_raw(name: &str, value: &str) -> FragmentMatcher {
    FragmentMatcher::Attribute(AttrMatcher {
        name: name.to_string(),
        value: Some(value.to_string()),
        raw_value: true,
        ..Default::default()
    })
}

/// Match a static rawValue attribute with attrStart.
pub fn attr_raw_start(name: &str, value: &str) -> FragmentMatcher {
    FragmentMatcher::Attribute(AttrMatcher {
        name: name.to_string(),
        value: Some(value.to_string()),
        raw_value: true,
        attr_start: true,
        ..Default::default()
    })
}

/// Match a skipped attribute.
pub fn attr_skip(name: &str, value: &str) -> FragmentMatcher {
    FragmentMatcher::Attribute(AttrMatcher {
        name: name.to_string(),
        value: Some(value.to_string()),
        attr_skip: true,
        ..Default::default()
    })
}

/// Match a simple dynamic attribute with attrStart.
pub fn attr_start(name: &str, value: &str) -> FragmentMatcher {
    FragmentMatcher::Attribute(AttrMatcher {
        name: name.to_string(),
        value: Some(value.to_string()),
        attr_start: true,
        ..Default::default()
    })
}

/// Match a component fragment.
pub fn component(id: &str) -> FragmentMatcher {
    FragmentMatcher::Component(id.to_string())
}

/// Match a for-loop fragment.
pub fn for_loop(item: &str, collection: &str, template: &str) -> FragmentMatcher {
    FragmentMatcher::ForLoop {
        item: item.to_string(),
        collection: collection.to_string(),
        template: template.to_string(),
    }
}

/// Match an if-condition fragment.
pub fn if_cond(template: &str) -> FragmentMatcher {
    FragmentMatcher::IfCond {
        template: template.to_string(),
    }
}

// ── Assertion implementation ────────────────────────────────────────

/// Assert that a fragment list matches the expected matchers.
pub fn assert_fragment_list(
    fragments: &[webui_protocol::WebUIFragment],
    matchers: &[FragmentMatcher],
) {
    use webui_protocol::web_ui_fragment::Fragment;

    assert_eq!(
        fragments.len(),
        matchers.len(),
        "Fragment count mismatch: got {} fragments, expected {}\nFragments: {:#?}",
        fragments.len(),
        matchers.len(),
        fragments.iter().map(format_fragment).collect::<Vec<_>>()
    );

    for (i, (frag, matcher)) in fragments.iter().zip(matchers.iter()).enumerate() {
        match (frag.fragment.as_ref(), matcher) {
            (Some(Fragment::Raw(r)), FragmentMatcher::Raw(expected)) => {
                assert_eq!(r.value, *expected, "Fragment[{}]: raw value mismatch", i);
            }
            (Some(Fragment::Signal(s)), FragmentMatcher::Signal { value, raw }) => {
                assert_eq!(s.value, *value, "Fragment[{}]: signal value mismatch", i);
                assert_eq!(s.raw, *raw, "Fragment[{}]: signal raw flag mismatch", i);
            }
            (Some(Fragment::Attribute(a)), FragmentMatcher::Attribute(m)) => {
                assert_eq!(a.name, m.name, "Fragment[{}]: attr name mismatch", i);
                if let Some(ref v) = m.value {
                    assert_eq!(a.value, *v, "Fragment[{}]: attr value mismatch", i);
                }
                if let Some(ref t) = m.template {
                    assert_eq!(a.template, *t, "Fragment[{}]: attr template mismatch", i);
                }
                assert_eq!(
                    a.complex, m.complex,
                    "Fragment[{}]: attr complex mismatch",
                    i
                );
                assert_eq!(
                    a.attr_start, m.attr_start,
                    "Fragment[{}]: attr_start mismatch",
                    i
                );
                assert_eq!(
                    a.attr_skip, m.attr_skip,
                    "Fragment[{}]: attr_skip mismatch",
                    i
                );
                assert_eq!(
                    a.raw_value, m.raw_value,
                    "Fragment[{}]: raw_value mismatch",
                    i
                );
                if let Some(ref sig) = m.bool_signal {
                    let cond = a
                        .condition_tree
                        .as_ref()
                        .unwrap_or_else(|| panic!("Fragment[{}]: expected condition_tree", i));
                    match cond.expr.as_ref() {
                        Some(webui_protocol::condition_expr::Expr::Identifier(id)) => {
                            assert_eq!(
                                id.value, *sig,
                                "Fragment[{}]: bool attr signal mismatch",
                                i
                            );
                        }
                        other => panic!(
                            "Fragment[{}]: expected identifier condition, got {:?}",
                            i, other
                        ),
                    }
                }
            }
            (Some(Fragment::Component(c)), FragmentMatcher::Component(id)) => {
                assert_eq!(c.fragment_id, *id, "Fragment[{}]: component id mismatch", i);
            }
            (
                Some(Fragment::ForLoop(fl)),
                FragmentMatcher::ForLoop {
                    item,
                    collection,
                    template,
                },
            ) => {
                assert_eq!(fl.item, *item, "Fragment[{}]: for item mismatch", i);
                assert_eq!(
                    fl.collection, *collection,
                    "Fragment[{}]: for collection mismatch",
                    i
                );
                assert_eq!(
                    fl.fragment_id, *template,
                    "Fragment[{}]: for template mismatch",
                    i
                );
            }
            (Some(Fragment::IfCond(ic)), FragmentMatcher::IfCond { template }) => {
                assert_eq!(
                    ic.fragment_id, *template,
                    "Fragment[{}]: if template mismatch",
                    i
                );
            }
            (_actual, expected) => {
                panic!(
                    "Fragment[{}]: type mismatch\n  expected: {:?}\n  actual: {}",
                    i,
                    expected,
                    format_fragment(frag)
                );
            }
        }
    }
}

fn format_fragment(frag: &webui_protocol::WebUIFragment) -> String {
    use webui_protocol::web_ui_fragment::Fragment;
    match frag.fragment.as_ref() {
        Some(Fragment::Raw(r)) => format!("raw({:?})", r.value),
        Some(Fragment::Signal(s)) => format!("signal({:?}, raw={})", s.value, s.raw),
        Some(Fragment::Attribute(a)) => format!(
            "attr({:?}, value={:?}, template={:?}, complex={}, start={}, skip={}, raw_value={})",
            a.name, a.value, a.template, a.complex, a.attr_start, a.attr_skip, a.raw_value
        ),
        Some(Fragment::Component(c)) => format!("component({:?})", c.fragment_id),
        Some(Fragment::ForLoop(f)) => format!(
            "for({:?} in {:?}, template={:?})",
            f.item, f.collection, f.fragment_id
        ),
        Some(Fragment::IfCond(i)) => format!("if(template={:?})", i.fragment_id),
        None => "None".to_string(),
    }
}

/// A test file system that manages temporary files and directories
pub struct TestFileSystem {
    files: HashMap<String, PathBuf>,

    // Keep directories alive for the lifetime of this struct
    _temp_dirs: Vec<TempDir>,
}

impl Default for TestFileSystem {
    fn default() -> Self {
        Self::new()
    }
}

impl TestFileSystem {
    /// Create a new empty test file system
    pub fn new() -> Self {
        Self {
            files: HashMap::new(),
            _temp_dirs: Vec::new(),
        }
    }

    /// Add a file to the test file system at the specified path
    pub fn add_file(&mut self, path: &str, content: &str) -> PathBuf {
        // Create a new temporary directory for this file
        let temp_dir = tempfile::tempdir().expect("Failed to create temporary directory");

        // Parse the path to separate directories and filename
        let path_parts: Vec<&str> = path.split('/').collect();
        let filename = path_parts.last().expect("Path must contain a filename");

        // Create the file within the temporary directory
        let file_path = temp_dir.path().join(filename);
        fs::write(&file_path, content).expect("Failed to write content to file");

        // Store the path and keep the directory alive
        self.files.insert(path.to_string(), file_path.clone());
        self._temp_dirs.push(temp_dir);

        // Return a reference to the stored path
        self.files
            .get(path)
            .expect("File path not found in the test file system");

        // Return the path by value (clone it)
        file_path
    }
}

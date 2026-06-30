// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

//! Shared state loading and merge policy for WebUI Press builds.

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

use serde_json::Value;

use crate::error::{Error, Result};
use crate::types::DocsConfig;

/// Loaded render state inputs shared by all generated pages.
#[derive(Debug, Default)]
pub(crate) struct LoadedStates {
    global: Option<Value>,
    custom_pages: HashMap<String, Value>,
}

impl LoadedStates {
    pub(crate) fn global(&self) -> Option<&Value> {
        self.global.as_ref()
    }

    pub(crate) fn custom_page_state(&self, logical_path: &str) -> Option<&Value> {
        self.custom_pages
            .get(logical_path)
            .or_else(|| self.custom_pages.get(logical_path.trim_end_matches('/')))
            .or_else(|| {
                if logical_path.ends_with('/') {
                    None
                } else {
                    let mut with_slash = String::with_capacity(logical_path.len() + 1);
                    with_slash.push_str(logical_path);
                    with_slash.push('/');
                    self.custom_pages.get(with_slash.as_str())
                }
            })
    }
}

/// Top-level state keys reserved by the docs renderer. Global and custom-page
/// state objects whose top-level fields are flattened onto the page state must
/// not shadow these names so the canonical docs state always wins.
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
    "headTags",
    "tokens",
    "label",
    "icon",
];

struct StateLoader {
    cache: HashMap<PathBuf, Value>,
}

impl StateLoader {
    fn new() -> Self {
        Self {
            cache: HashMap::new(),
        }
    }

    fn load_state_value(
        &mut self,
        label: &str,
        inline: Option<&Value>,
        path: Option<&str>,
        config_dir: &Path,
    ) -> Result<Option<Value>> {
        if inline.is_some() && path.is_some() {
            return Err(Error::Build(format!(
                "{label}: 'state' and 'stateFile' are mutually exclusive - pick one."
            )));
        }

        if let Some(value) = inline {
            return Ok(Some(value.clone()));
        }

        let Some(rel) = path else {
            return Ok(None);
        };

        let rel_path = Path::new(rel);
        if rel_path.is_absolute() {
            return Err(Error::Build(format!(
                "{label}: stateFile must be a path relative to config.json, got absolute path {}",
                rel_path.display()
            )));
        }

        let abs = config_dir.join(rel_path);
        let key = fs::canonicalize(&abs).unwrap_or_else(|_| abs.clone());
        if let Some(cached) = self.cache.get(&key) {
            return Ok(Some(cached.clone()));
        }

        let raw = fs::read_to_string(&abs).map_err(|e| {
            Error::Build(format!(
                "{label}: cannot read stateFile {}: {e}",
                abs.display()
            ))
        })?;
        let parsed: Value = serde_json::from_str(&raw).map_err(|e| {
            Error::Build(format!(
                "{label}: stateFile {} is not valid JSON: {e}",
                abs.display()
            ))
        })?;
        self.cache.insert(key, parsed.clone());
        Ok(Some(parsed))
    }

    fn load_global_state(
        &mut self,
        config: &DocsConfig,
        config_dir: &Path,
    ) -> Result<Option<Value>> {
        let Some(value) = self.load_state_value(
            "Global state",
            config.state.as_ref(),
            config.state_file.as_deref(),
            config_dir,
        )?
        else {
            return Ok(None);
        };

        if !value.is_object() {
            return Err(Error::Build(
                "Global state: state/stateFile must be a JSON object.".to_string(),
            ));
        }

        Ok(Some(value))
    }

    fn load_custom_page_states(
        &mut self,
        config: &DocsConfig,
        config_dir: &Path,
    ) -> Result<HashMap<String, Value>> {
        let mut out: HashMap<String, Value> = HashMap::with_capacity(config.custom_pages.len());

        for (link, page) in &config.custom_pages {
            if let Some(value) = self.load_state_value(
                &format!("Custom page {link}"),
                page.inline_state(),
                page.state_file(),
                config_dir,
            )? {
                out.insert(link.clone(), value);
            }
        }

        Ok(out)
    }
}

pub(crate) fn load_render_states(config: &DocsConfig, config_dir: &Path) -> Result<LoadedStates> {
    let mut loader = StateLoader::new();
    let global = loader.load_global_state(config, config_dir)?;
    let custom_pages = loader.load_custom_page_states(config, config_dir)?;
    Ok(LoadedStates {
        global,
        custom_pages,
    })
}

/// Merge docs state with global state first and custom-page state second.
///
/// Global state fills missing non-reserved top-level keys. Custom-page state is
/// applied afterward and may replace global keys, but reserved docs keys are
/// never replaced.
pub(crate) fn merge_page_state(
    mut state: Value,
    global: Option<&Value>,
    custom_page: Option<&Value>,
) -> Value {
    merge_top_level_state(&mut state, global, false);
    merge_top_level_state(&mut state, custom_page, true);
    state
}

fn merge_top_level_state(state: &mut Value, extra: Option<&Value>, overwrite_existing: bool) {
    let Some(extra) = extra.and_then(Value::as_object) else {
        return;
    };

    let Value::Object(map) = state else {
        return;
    };

    for (key, value) in extra {
        if RESERVED_STATE_KEYS.contains(&key.as_str()) {
            continue;
        }
        if overwrite_existing || !map.contains_key(key) {
            map.insert(key.clone(), value.clone());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{CustomPage, SiteConfig};
    use serde_json::Map;
    use std::sync::atomic::{AtomicU64, Ordering};

    type TestResult<T = ()> = std::result::Result<T, Box<dyn std::error::Error>>;

    static TEMP_COUNTER: AtomicU64 = AtomicU64::new(0);

    fn empty_config() -> DocsConfig {
        DocsConfig {
            site: SiteConfig {
                title: "Docs".to_string(),
                description: String::new(),
            },
            base_path: "/".to_string(),
            content_dir: ".".to_string(),
            out_dir: "./dist".to_string(),
            public_dir: "./public".to_string(),
            css: None,
            theme: None,
            components: None,
            head: Vec::new(),
            nav: Vec::new(),
            sidebar: Vec::new(),
            sidebar_groups: std::collections::BTreeMap::new(),
            custom_pages: HashMap::new(),
            state: None,
            state_file: None,
            hero: None,
            footer: None,
            bundler: None,
        }
    }

    fn test_obj<const N: usize>(entries: [(&str, Value); N]) -> Value {
        let mut map = Map::with_capacity(N);
        for (key, value) in entries {
            map.insert(key.to_string(), value);
        }
        Value::Object(map)
    }

    fn string_value(value: &str) -> Value {
        Value::String(value.to_string())
    }

    fn temp_config_dir(name: &str) -> TestResult<PathBuf> {
        let id = TEMP_COUNTER.fetch_add(1, Ordering::Relaxed);
        let dir =
            std::env::temp_dir().join(format!("webui-press-{name}-{}-{id}", std::process::id()));
        fs::create_dir_all(&dir)?;
        Ok(dir)
    }

    #[test]
    fn load_render_states_reads_global_state_file_object() -> TestResult {
        let dir = temp_config_dir("global-state-file")?;
        fs::create_dir_all(dir.join("state"))?;
        fs::write(
            dir.join("state/site.json"),
            r#"{ "release": { "version": "v1.2.3" } }"#,
        )?;

        let mut config = empty_config();
        config.state_file = Some("./state/site.json".to_string());

        let states = load_render_states(&config, &dir)?;
        let state = states.global().ok_or_else(|| {
            std::io::Error::new(std::io::ErrorKind::NotFound, "missing global state")
        })?;
        assert_eq!(state["release"]["version"], "v1.2.3");

        fs::remove_dir_all(dir).ok();
        Ok(())
    }

    #[test]
    fn load_render_states_rejects_global_state_and_state_file_together() {
        let mut config = empty_config();
        config.state = Some(test_obj([(
            "release",
            test_obj([("version", string_value("v1.2.3"))]),
        )]));
        config.state_file = Some("./state/site.json".to_string());

        let error = load_render_states(&config, Path::new(".")).err();
        let Some(error) = error else {
            panic!("expected mutually exclusive state error");
        };
        assert!(
            error
                .to_string()
                .contains("'state' and 'stateFile' are mutually exclusive"),
            "{error}"
        );
    }

    #[test]
    fn load_render_states_requires_global_json_object() {
        let mut config = empty_config();
        config.state = Some(Value::Array(vec![
            string_value("not"),
            string_value("object"),
        ]));

        let error = load_render_states(&config, Path::new(".")).err();
        let Some(error) = error else {
            panic!("expected object validation error");
        };
        assert!(
            error
                .to_string()
                .contains("state/stateFile must be a JSON object"),
            "{error}"
        );
    }

    #[test]
    fn load_render_states_reads_custom_page_state_file() -> TestResult {
        let dir = temp_config_dir("custom-state-file")?;
        fs::create_dir_all(dir.join("state"))?;
        fs::write(
            dir.join("state/playground.json"),
            r#"{ "files": ["a.rs"] }"#,
        )?;

        let mut config = empty_config();
        config.custom_pages.insert(
            "/playground/".to_string(),
            CustomPage::Full {
                html: "<docs-playground></docs-playground>".to_string(),
                layout: Some("full".to_string()),
                state: None,
                state_file: Some("./state/playground.json".to_string()),
                script_file: None,
            },
        );

        let states = load_render_states(&config, &dir)?;
        let state = states.custom_page_state("/playground").ok_or_else(|| {
            std::io::Error::new(std::io::ErrorKind::NotFound, "missing custom state")
        })?;
        assert_eq!(state["files"][0], "a.rs");

        fs::remove_dir_all(dir).ok();
        Ok(())
    }

    #[test]
    fn merge_page_state_applies_global_then_custom_page_state() {
        let base = test_obj([
            ("site", test_obj([("title", string_value("Docs"))])),
            ("headTags", string_value("<meta name=\"docs\">")),
        ]);
        let global = test_obj([
            ("release", test_obj([("version", string_value("global"))])),
            ("shared", string_value("global")),
        ]);
        let custom = test_obj([
            ("release", test_obj([("version", string_value("custom"))])),
            ("local", string_value("custom")),
        ]);

        let merged = merge_page_state(base, Some(&global), Some(&custom));
        assert_eq!(merged["release"]["version"], "custom");
        assert_eq!(merged["shared"], "global");
        assert_eq!(merged["local"], "custom");
    }

    #[test]
    fn merge_page_state_keeps_reserved_docs_keys() {
        let base = test_obj([
            ("site", test_obj([("title", string_value("Docs"))])),
            ("headTags", string_value("<meta name=\"docs\">")),
        ]);
        let custom = test_obj([
            ("site", test_obj([("title", string_value("Override"))])),
            ("headTags", string_value("<script>bad()</script>")),
            ("tokens", string_value("not-token-css")),
        ]);

        let merged = merge_page_state(base, None, Some(&custom));
        assert_eq!(merged["site"]["title"], "Docs");
        assert_eq!(merged["headTags"], "<meta name=\"docs\">");
        assert_eq!(merged.get("tokens"), None);
    }
}

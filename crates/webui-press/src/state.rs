// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

//! Shared state loading and merge policy for WebUI Press builds.

use std::collections::{hash_map::DefaultHasher, HashMap};
use std::fs;
use std::hash::Hasher;
use std::io::Read;
use std::path::{Path, PathBuf};

use serde_json::Value;

use crate::error::{Error, Result};
use crate::types::DocsConfig;

pub(crate) fn parse_markdown_state_file(value: &serde_yaml::Value) -> Result<String> {
    value
        .as_str()
        .map(str::to_string)
        .ok_or_else(|| markdown_state_error("stateFile must be a relative JSON file path string"))
}

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

// Flattened user state must not shadow the canonical docs state.
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
    "regions",
    "headTags",
    "tokens",
    "label",
    "icon",
    "__proto__",
    "prototype",
    "constructor",
];

pub(crate) fn validate_application_state(value: &Value) -> Result<()> {
    let object = value
        .as_object()
        .ok_or_else(|| markdown_state_error("state/stateFile must be JSON objects"))?;
    for key in object.keys() {
        if RESERVED_STATE_KEYS.contains(&key.as_str()) {
            return Err(markdown_state_error(&format!(
                "top-level key '{key}' is reserved by Press; move it under an application-specific key"
            )));
        }
    }
    Ok(())
}

pub(crate) fn parse_markdown_state(value: &serde_yaml::Value) -> Result<Value> {
    let mapping = value.as_mapping().ok_or_else(|| {
        markdown_state_error("state must be an object, not a scalar, array, or null")
    })?;
    for key in mapping.keys().filter_map(serde_yaml::Value::as_str) {
        if RESERVED_STATE_KEYS.contains(&key) {
            return Err(markdown_state_error(&format!(
                "top-level key '{key}' is reserved by Press; move it under an application-specific key"
            )));
        }
    }

    // Validate before serialization: JSON otherwise coerces numeric keys and
    // non-finite numbers, and YAML tags carry semantics JSON cannot preserve.
    let mut pending = vec![value];
    while let Some(value) = pending.pop() {
        match value {
            serde_yaml::Value::Mapping(map) => {
                for (key, value) in map {
                    if !matches!(key, serde_yaml::Value::String(_)) {
                        return Err(markdown_state_error(
                            "all state object keys must be strings; quote numeric or boolean keys",
                        ));
                    }
                    pending.push(value);
                }
            }
            serde_yaml::Value::Sequence(items) => pending.extend(items),
            serde_yaml::Value::Number(number) => {
                if number.as_f64().is_some_and(|value| !value.is_finite()) {
                    return Err(markdown_state_error(
                        "state numbers must be finite; replace .nan and .inf with JSON-compatible values",
                    ));
                }
            }
            serde_yaml::Value::Tagged(_) => {
                return Err(markdown_state_error(
                    "custom YAML tags are not supported in state; use plain JSON-compatible values",
                ));
            }
            _ => {}
        }
    }
    serde_json::to_value(value).map_err(|error| markdown_state_error(&error.to_string()))
}

#[cold]
#[inline(never)]
fn markdown_state_error(reason: &str) -> Error {
    Error::Markdown(format!(
        "Invalid frontmatter state: {reason}.\nhelp: Use a JSON-compatible object for state, or a relative JSON object file path for stateFile."
    ))
}

const MAX_STATE_BYTES: u64 = 16 * 1024 * 1024;

#[derive(Default)]
pub(crate) struct StateDependencies {
    pub(crate) paths: Vec<PathBuf>,
    versions: HashMap<PathBuf, Option<u64>>,
}

impl StateDependencies {
    pub(crate) fn begin_build(&mut self) {
        self.versions.clear();
    }

    pub(crate) fn changed_paths(&self) -> Vec<PathBuf> {
        let mut buffer = [0_u8; 8 * 1024];
        let mut changed: Vec<_> = self
            .versions
            .iter()
            .filter(|(path, version)| state_file_version(path, &mut buffer) != **version)
            .map(|(path, _)| path.clone())
            .collect();
        changed.sort();
        changed
    }
}

fn state_file_version(path: &Path, buffer: &mut [u8]) -> Option<u64> {
    // An unavailable input is a version too; a transition queues a build that
    // surfaces its read error, while an unchanged failure must not loop.
    let metadata = fs::metadata(path).ok()?;
    if !metadata.is_file() || metadata.len() > MAX_STATE_BYTES {
        return None;
    }
    let mut file = fs::File::open(path).ok()?.take(MAX_STATE_BYTES + 1);
    let mut hasher = DefaultHasher::new();
    let mut total = 0;
    loop {
        let count = match file.read(buffer) {
            Ok(count) => count,
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(_) => return None,
        };
        if count == 0 {
            return Some(hasher.finish());
        }
        total += count as u64;
        if total > MAX_STATE_BYTES {
            return None;
        }
        hasher.write(&buffer[..count]);
    }
}

pub(crate) struct StateLoader {
    cache: HashMap<PathBuf, Value>,
    dependencies: HashMap<PathBuf, Option<u64>>,
}

impl StateLoader {
    pub(crate) fn new() -> Self {
        Self {
            cache: HashMap::new(),
            dependencies: HashMap::new(),
        }
    }

    #[cfg(test)]
    fn dependencies(&self) -> impl Iterator<Item = &PathBuf> {
        self.dependencies.keys()
    }

    pub(crate) fn into_dependencies(self) -> StateDependencies {
        let mut paths: Vec<_> = self.dependencies.keys().cloned().collect();
        paths.sort();
        StateDependencies {
            paths,
            versions: self.dependencies,
        }
    }

    pub(crate) fn load_markdown_state(
        &mut self,
        file: Option<&str>,
        inline: Option<Value>,
        source: &Path,
    ) -> Result<Option<Value>> {
        let Some(file) = file else {
            return Ok(inline);
        };
        let directory = source.parent().unwrap_or_else(|| Path::new("."));
        let value = self.load_json_file(file, directory)?;
        validate_application_state(value)?;
        let mut state = value.clone();
        if let (Value::Object(state), Some(Value::Object(inline))) = (&mut state, inline) {
            state.extend(inline);
        }
        Ok(Some(state))
    }

    fn load_json_file(&mut self, relative: &str, directory: &Path) -> Result<&Value> {
        let path = Path::new(relative);
        if relative.trim().is_empty()
            || path.is_absolute()
            || path.has_root()
            || path
                .components()
                .any(|part| matches!(part, std::path::Component::Prefix(_)))
        {
            return Err(markdown_state_error(&format!(
                "stateFile '{relative}' must be a non-empty relative JSON file path"
            )));
        }
        let path = directory.join(path);
        let absolute = std::path::absolute(&path).map_err(|error| {
            markdown_state_error(&format!(
                "cannot resolve stateFile {}: {error}",
                path.display()
            ))
        })?;
        let key = fs::canonicalize(&path).map_err(|error| {
            self.dependencies.insert(absolute, None);
            markdown_state_error(&format!(
                "cannot resolve stateFile {}: {error}",
                path.display()
            ))
        })?;
        self.dependencies.entry(key.clone()).or_insert(None);
        if !self.cache.contains_key(&key) {
            let file = fs::File::open(&key).map_err(|error| {
                markdown_state_error(&format!(
                    "cannot read stateFile {}: {error}",
                    path.display()
                ))
            })?;
            let mut bytes = Vec::new();
            file.take(MAX_STATE_BYTES + 1)
                .read_to_end(&mut bytes)
                .map_err(|error| {
                    markdown_state_error(&format!(
                        "cannot read stateFile {}: {error}",
                        path.display()
                    ))
                })?;
            if bytes.len() as u64 > MAX_STATE_BYTES {
                return Err(markdown_state_error(&format!(
                    "stateFile {} exceeds the 16 MiB input limit",
                    path.display()
                )));
            }
            let mut hasher = DefaultHasher::new();
            hasher.write(&bytes);
            self.dependencies.insert(key.clone(), Some(hasher.finish()));
            let value: Value = serde_json::from_slice(&bytes).map_err(|error| {
                markdown_state_error(&format!(
                    "stateFile {} is not valid JSON: {error}",
                    path.display()
                ))
            })?;
            self.cache.insert(key.clone(), value);
        }
        self.cache
            .get(&key)
            .ok_or_else(|| markdown_state_error("stateFile cache entry was not loaded"))
    }

    pub(crate) fn load_state_value(
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

// Global state fills missing non-reserved keys. Markdown or custom-page state
// replaces global keys shallowly, but never replaces canonical docs keys.
pub(crate) fn merge_page_state(
    mut state: Value,
    global: Option<&Value>,
    local: Option<&Value>,
) -> Value {
    merge_top_level_state(&mut state, global, false);
    merge_top_level_state(&mut state, local, true);
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
            show: Default::default(),
            outline: false,
            site: SiteConfig {
                title: "Docs".to_string(),
                description: String::new(),
                url: None,
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
            regions: std::collections::BTreeMap::new(),
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
                title: None,
                description: None,
                outline: None,
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
            ("regions", test_obj([("bad", string_value("override"))])),
        ]);

        let merged = merge_page_state(base, None, Some(&custom));
        assert_eq!(merged["site"]["title"], "Docs");
        assert_eq!(merged["headTags"], "<meta name=\"docs\">");
        assert_eq!(merged.get("tokens"), None);
        assert_eq!(merged.get("regions"), None);
    }

    #[test]
    fn markdown_state_file_cache_and_shallow_precedence() -> TestResult {
        let root = tempfile::tempdir()?;
        let source = root.path().join("button.md");
        let metadata = root.path().join("button.json");
        fs::write(
            &metadata,
            r#"{"component":{"title":"Button","page":{"allowed":"nested"},"options":[true,false]}}"#,
        )?;
        let file = parse_markdown_state_file(&serde_yaml::from_str("./button.json")?)?;
        let inline = parse_markdown_state(&serde_yaml::from_str("{example: {count: 2}}")?)?;
        let mut loader = StateLoader::new();
        let state = loader
            .load_markdown_state(Some(&file), Some(inline), &source)?
            .ok_or("missing local state")?;
        assert_eq!(state["component"]["title"], "Button");
        assert_eq!(state["component"]["page"]["allowed"], "nested");
        assert_eq!(state["example"]["count"], 2);
        assert_eq!(loader.dependencies().count(), 1);
        fs::write(&metadata, "not JSON")?;
        let cached = loader
            .load_markdown_state(Some(&file), None, &source)?
            .ok_or("missing cached state")?;
        assert_eq!(cached["component"]["title"], "Button");
        let replacement =
            parse_markdown_state(&serde_yaml::from_str("{component: {title: Inline}}")?)?;
        let replaced = loader
            .load_markdown_state(Some(&file), Some(replacement), &source)?
            .ok_or("missing state")?;
        assert_eq!(replaced["component"]["title"], "Inline");
        assert_eq!(replaced["component"].get("options"), None);
        assert!(StateLoader::new()
            .load_markdown_state(Some(&file), None, &source)
            .is_err());
        Ok(())
    }

    #[test]
    fn markdown_state_file_rejects_invalid_paths_and_objects() -> TestResult {
        for yaml in ["null", "[]", "{component: x.json}", "42", "true"] {
            assert!(
                parse_markdown_state_file(&serde_yaml::from_str(yaml)?).is_err(),
                "{yaml}"
            );
        }
        let root = tempfile::tempdir()?;
        let source = root.path().join("button.md");
        for (content, yaml) in [
            ("[]", "button.json"),
            ("null", "button.json"),
            ("{", "button.json"),
            (r#"{"site":{}}"#, "button.json"),
            ("{}", "missing.json"),
            ("{}", "''"),
        ] {
            fs::write(root.path().join("button.json"), content)?;
            let file = parse_markdown_state_file(&serde_yaml::from_str(yaml)?)?;
            let error = StateLoader::new()
                .load_markdown_state(Some(&file), None, &source)
                .err()
                .ok_or("expected invalid input")?
                .to_string();
            assert!(error.contains("help:"), "{error}");
        }
        let mut loader = StateLoader::new();
        assert!(loader
            .load_json_file(&source.to_string_lossy(), root.path())
            .is_err());
        #[cfg(windows)]
        for path in [
            r"C:button.json",
            r"\button.json",
            r"\\server\share\button.json",
        ] {
            assert!(loader.load_json_file(path, root.path()).is_err(), "{path}");
        }
        Ok(())
    }

    #[test]
    fn markdown_state_file_bounds_reads_and_retains_missing_dependencies() -> TestResult {
        let root = tempfile::tempdir()?;
        let mut loader = StateLoader::new();
        assert!(loader
            .load_json_file("new/missing.json", root.path())
            .is_err());
        assert_eq!(loader.dependencies().count(), 1);
        let file = root.path().join("large.json");
        fs::File::create(file)?.set_len(16 * 1024 * 1024 + 1)?;
        let error = loader
            .load_json_file("large.json", root.path())
            .err()
            .ok_or("expected size limit error")?
            .to_string();
        assert!(error.contains("16 MiB input limit"), "{error}");
        assert_eq!(loader.dependencies().count(), 2);
        Ok(())
    }

    #[test]
    fn markdown_state_file_combines_with_inline_state() -> TestResult {
        let root = tempfile::tempdir()?;
        fs::write(
            root.path().join("state.json"),
            r#"{"example":{"label":"File","count":1},"shared":true}"#,
        )?;
        let inline = parse_markdown_state(&serde_yaml::from_str("{example: {label: Inline}}")?)?;
        let state = StateLoader::new()
            .load_markdown_state(
                Some("state.json"),
                Some(inline),
                &root.path().join("page.md"),
            )?
            .ok_or("missing state")?;
        assert_eq!(state["example"]["label"], "Inline");
        assert_eq!(state["example"].get("count"), None);
        assert_eq!(state["shared"], true);
        Ok(())
    }
}

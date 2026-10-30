// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

//! Normalize appearance once for SSR, pre-paint restoration, CSS, and validation.

use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::Path;

use serde::Serialize;
use serde_json::Value;
use webui_tokens::TokenFile;

use crate::error::{Error, Result};
use crate::state::StateDependencies;
use crate::types::{AppearanceMode, AppearanceStorage, DocsConfig, ShowMode};

#[derive(Serialize)]
pub(crate) struct FamilyMetadata {
    pub(crate) id: String,
    pub(crate) label: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct AppearanceMetadata {
    themes: Vec<FamilyMetadata>,
    default_theme: String,
    default_mode: AppearanceMode,
    storage: AppearanceStorage,
}

pub(crate) struct PreparedAppearance {
    metadata: AppearanceMetadata,
    pub(crate) css: String,
    legacy_tokens: Option<TokenFile>,
    // The compiler only checks token presence, never values. Keep a compact
    // validation inventory instead of copying full snapshots into every page.
    validation: Option<TokenFile>,
}

impl PreparedAppearance {
    pub(crate) fn prepare(
        config: &DocsConfig,
        config_dir: &Path,
        dependencies: &mut StateDependencies,
    ) -> Result<Self> {
        if config.theme.is_some() && config.appearance.is_some() {
            return Err(invalid("theme and appearance cannot be combined; move the legacy theme into appearance.themes[].source"));
        }
        let metadata = normalize(config)?;
        let legacy_tokens = config
            .theme
            .as_deref()
            .map(|source| load_source(source, config_dir, dependencies))
            .transpose()?;
        let mut sources = Vec::with_capacity(metadata.themes.len());
        if let Some(appearance) = &config.appearance {
            for family in &appearance.themes {
                let tokens = family
                    .source
                    .as_deref()
                    .map(|source| load_source(source, config_dir, dependencies))
                    .transpose()?;
                if let Some(tokens) = &tokens {
                    for mode in ["light", "dark"] {
                        if !tokens.themes.contains_key(mode) {
                            return Err(invalid(&format!("family '{}' is missing its '{mode}' map; add themes.{mode} to its source", family.id)));
                        }
                    }
                }
                sources.push(tokens);
            }
        }
        let validation = validation_inventory(&sources, legacy_tokens.as_ref());
        let css = crate::appearance_css::compile(
            &metadata.themes,
            &sources,
            &metadata.default_theme,
            config.show,
        )?;
        Ok(Self {
            metadata,
            css,
            legacy_tokens,
            validation,
        })
    }

    pub(crate) fn has_multiple_themes(&self) -> bool {
        self.metadata.themes.len() > 1
    }

    pub(crate) fn legacy_tokens(&self) -> Option<&TokenFile> {
        self.legacy_tokens.as_ref()
    }

    pub(crate) fn validation_tokens(&self) -> Option<TokenFile> {
        self.validation.clone()
    }

    /// External styles are linked through render state, so the component
    /// compiler cannot discover their declarations itself. Augment only its
    /// validation inventory, never the family's emitted token snapshot.
    pub(crate) fn add_css_validation_context(&mut self, css: &str) -> Result<()> {
        let Some(validation) = self.validation.as_mut() else {
            return Ok(());
        };
        let definitions = webui_parser::CssParser::new()
            .extract_definitions(css)
            .map_err(|error| {
                invalid(&format!(
                    "cannot parse site CSS for token validation: {error}"
                ))
            })?;
        for tokens in validation.themes.values_mut() {
            for name in &definitions {
                tokens.entry(name.clone()).or_default();
            }
        }
        Ok(())
    }

    pub(crate) fn state(&self, show: ShowMode) -> Value {
        let mode = if show == ShowMode::Content {
            AppearanceMode::System
        } else {
            self.metadata.default_mode
        };
        let themes = self
            .metadata
            .themes
            .iter()
            .map(|family| {
                Self::object([
                    ("id", Value::String(family.id.clone())),
                    ("label", Value::String(family.label.clone())),
                ])
            })
            .collect();
        Self::object([
            ("themes", Value::Array(themes)),
            ("theme", Value::String(self.metadata.default_theme.clone())),
            (
                "selected",
                Self::object([
                    ("system", Value::Bool(mode == AppearanceMode::System)),
                    ("light", Value::Bool(mode == AppearanceMode::Light)),
                    ("dark", Value::Bool(mode == AppearanceMode::Dark)),
                ]),
            ),
            (
                "mode",
                Value::String(
                    match mode {
                        AppearanceMode::System => "system",
                        AppearanceMode::Light => "light",
                        AppearanceMode::Dark => "dark",
                    }
                    .into(),
                ),
            ),
            (
                "resolvedMode",
                Value::String(
                    if mode == AppearanceMode::Dark {
                        "dark"
                    } else {
                        "light"
                    }
                    .into(),
                ),
            ),
        ])
    }

    fn object<const N: usize>(entries: [(&str, Value); N]) -> Value {
        Value::Object(
            entries
                .into_iter()
                .map(|(key, value)| (key.to_string(), value))
                .collect(),
        )
    }

    pub(crate) fn head_tags(&self, show: ShowMode) -> Result<String> {
        if show == ShowMode::Content {
            return Ok(String::new());
        }
        let json = serde_json::to_string(&self.metadata)
            .map_err(|error| invalid(&format!("cannot serialize appearance: {error}")))?
            .replace('<', "\\u003c")
            .replace('>', "\\u003e")
            .replace('&', "\\u0026")
            .replace('\u{2028}', "\\u2028")
            .replace('\u{2029}', "\\u2029");
        Ok(format!(
            "<script type=\"application/json\" id=\"webui-press-appearance\">{json}</script>\n<script>{}</script>",
            include_str!("../template/appearance-bootstrap.js")
        ))
    }
}

fn normalize(config: &DocsConfig) -> Result<AppearanceMetadata> {
    let Some(appearance) = &config.appearance else {
        return Ok(AppearanceMetadata {
            themes: vec![FamilyMetadata {
                id: "default".into(),
                label: "Default".into(),
            }],
            default_theme: "default".into(),
            default_mode: AppearanceMode::System,
            storage: default_storage(config),
        });
    };
    let mut ids = HashSet::with_capacity(appearance.themes.len());
    for theme in &appearance.themes {
        if !valid_id(&theme.id) || !ids.insert(theme.id.as_str()) {
            return Err(invalid(&format!("family ID '{}' is unsafe or duplicated; use unique ASCII letters, digits, hyphens or underscores, starting with a letter", theme.id)));
        }
        if theme.label.trim().is_empty() {
            return Err(invalid(&format!(
                "family '{}' needs a nonempty label",
                theme.id
            )));
        }
    }
    if !ids.contains(appearance.default_theme.as_str()) {
        return Err(invalid("defaultTheme must name a configured family; add that family to themes or correct defaultTheme"));
    }
    let storage = appearance
        .storage
        .clone()
        .unwrap_or_else(|| default_storage(config));
    if storage.theme.trim().is_empty()
        || storage.mode.trim().is_empty()
        || storage.theme == storage.mode
    {
        return Err(invalid(
            "storage.theme and storage.mode must be distinct, nonempty keys",
        ));
    }
    Ok(AppearanceMetadata {
        themes: appearance
            .themes
            .iter()
            .map(|theme| FamilyMetadata {
                id: theme.id.clone(),
                label: theme.label.clone(),
            })
            .collect(),
        default_theme: appearance.default_theme.clone(),
        default_mode: appearance.default_mode,
        storage,
    })
}

fn valid_id(id: &str) -> bool {
    id.as_bytes().first().is_some_and(u8::is_ascii_alphabetic)
        && id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
        && !matches!(id, "__proto__" | "prototype" | "constructor")
}

fn default_storage(config: &DocsConfig) -> AppearanceStorage {
    // localStorage is origin-scoped already; canonical deployment base separates
    // sites on the same origin without tying preferences to a build machine.
    let base = config.base_path.trim_matches('/');
    let scope = if base.is_empty() {
        "webui-press:/".to_string()
    } else {
        format!("webui-press:/{base}/")
    };
    AppearanceStorage {
        theme: format!("{scope}:theme"),
        mode: format!("{scope}:mode"),
    }
}

fn load_source(
    source: &str,
    config_dir: &Path,
    dependencies: &mut StateDependencies,
) -> Result<TokenFile> {
    let candidate = config_dir.join(source);
    let path = if candidate.exists() || source.starts_with('.') || Path::new(source).is_absolute() {
        candidate.canonicalize().unwrap_or(candidate)
    } else {
        webui_tokens::resolve_theme_path(source, config_dir)
            .map_err(|error| invalid(&format!("cannot resolve source '{source}': {error}; install its npm package or use a config-relative file")))?
    };
    let content = fs::read_to_string(&path);
    dependencies.record_input(
        path.clone(),
        content.as_ref().ok().map(|value| value.as_bytes()),
    );
    let content = content.map_err(|error| {
        invalid(&format!(
            "cannot read {}: {error}; check the source path and permissions",
            path.display()
        ))
    })?;
    webui_tokens::parse_token_content(&content, &path).map_err(|error| {
        invalid(&format!(
            "cannot load {}: {error}; provide a token JSON object",
            path.display()
        ))
    })
}

fn validation_inventory(
    sources: &[Option<TokenFile>],
    legacy: Option<&TokenFile>,
) -> Option<TokenFile> {
    let mut themes = HashMap::new();
    for (index, source) in sources
        .iter()
        .filter_map(Option::as_ref)
        .chain(legacy)
        .enumerate()
    {
        for (scheme, tokens) in &source.themes {
            themes.insert(
                format!("{index}:{scheme}"),
                tokens
                    .keys()
                    .map(|key| (key.clone(), String::new()))
                    .collect(),
            );
        }
    }
    (!themes.is_empty()).then_some(TokenFile { themes })
}

#[cold]
#[inline(never)]
fn invalid(message: &str) -> Error {
    Error::Build(format!("Invalid Press appearance: {message}.\nhelp: Configure appearance with a defaultTheme, ordered themes and light/dark token sources; omit source for CSS-only families."))
}

#[cfg(test)]
#[path = "appearance_tests.rs"]
mod tests;

// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

//! Deterministic baseline/delta CSS, borrowing complete source token snapshots.

use std::collections::{BTreeMap, BTreeSet};

use webui_tokens::TokenFile;

use crate::appearance::FamilyMetadata;
use crate::error::{Error, Result};
use crate::types::ShowMode;

const EXPLICIT_DARK: &str =
    ":where([data-theme-mode=\"dark\"], [data-theme=\"dark\"]:not([data-theme-mode]))";
const SYSTEM: &str =
    ":where([data-theme-mode=\"system\"], :not([data-theme-mode]):not([data-theme]))";

type DeclarationGroups = BTreeMap<Vec<usize>, String>;

pub(crate) fn compile(
    families: &[FamilyMetadata],
    sources: &[Option<TokenFile>],
    default_theme: &str,
    show: ShowMode,
) -> Result<String> {
    let default = families
        .iter()
        .position(|family| family.id == default_theme)
        .ok_or_else(|| {
            Error::Build(
                "Cannot compile appearance: defaultTheme must identify a configured family".into(),
            )
        })?;
    let names: BTreeSet<&str> = sources
        .iter()
        .filter_map(Option::as_ref)
        .flat_map(|source| source.themes.iter())
        .filter(|(scheme, _)| matches!(scheme.as_str(), "light" | "dark"))
        .flat_map(|(_, tokens)| tokens.keys().map(String::as_str))
        .collect();
    let active: Vec<usize> = (0..families.len())
        .filter(|index| show != ShowMode::Content || *index == default)
        .collect();
    let mut css = String::with_capacity(names.len().saturating_mul(48));
    css.push_str(":root {\ncolor-scheme: light;\n");
    // Every source token is emitted, so no transitive closure or cloned token
    // maps are needed. var() values remain literal, including cycles/initial.
    for name in &names {
        declaration(&mut css, name, value(sources, default, "light", name));
    }
    css.push_str("}\n");

    let light = deltas(sources, &names, &active, Some(default));
    write_groups(&mut css, &light, families, default, "");
    drop(light);
    let dark = deltas(sources, &names, &active, None);
    if show == ShowMode::Content {
        css.push_str("@media (prefers-color-scheme: dark) {\n:root {\ncolor-scheme: dark;\n");
        for declarations in dark.values() {
            css.push_str(declarations);
        }
        css.push_str("}\n}\n");
        return Ok(css);
    }
    css.push_str(":root");
    css.push_str(EXPLICIT_DARK);
    css.push_str(" {\ncolor-scheme: dark;\n}\n");
    write_groups(&mut css, &dark, families, default, EXPLICIT_DARK);
    css.push_str("@media (prefers-color-scheme: dark) {\n:root");
    css.push_str(SYSTEM);
    css.push_str(" {\ncolor-scheme: dark;\n}\n");
    write_groups(&mut css, &dark, families, default, SYSTEM);
    css.push_str("}\n");
    Ok(css)
}

/// Group declarations by the exact set of families where they apply. Common
/// values are emitted once, not repeated in each family's rule.
fn deltas(
    sources: &[Option<TokenFile>],
    names: &BTreeSet<&str>,
    active: &[usize],
    light_baseline: Option<usize>,
) -> DeclarationGroups {
    let mut groups = DeclarationGroups::new();
    for name in names {
        let mut values: BTreeMap<&str, Vec<usize>> = BTreeMap::new();
        for &family in active {
            let (scheme, baseline) =
                light_baseline.map_or(("dark", family), |index| ("light", index));
            let selected = value(sources, family, scheme, name);
            if selected != value(sources, baseline, "light", name) {
                values.entry(selected).or_default().push(family);
            }
        }
        for (selected, families) in values {
            declaration(groups.entry(families).or_default(), name, selected);
        }
    }
    groups
}

fn value<'a>(sources: &'a [Option<TokenFile>], family: usize, scheme: &str, name: &str) -> &'a str {
    sources
        .get(family)
        .and_then(Option::as_ref)
        .and_then(|source| source.themes.get(scheme))
        .and_then(|tokens| tokens.get(name))
        .map_or("initial", String::as_str)
}

fn declaration(css: &mut String, name: &str, value: &str) {
    css.push_str("--");
    css.push_str(name);
    css.push_str(": ");
    css.push_str(value);
    css.push_str(";\n");
}

fn write_groups(
    css: &mut String,
    groups: &DeclarationGroups,
    families: &[FamilyMetadata],
    default: usize,
    mode: &str,
) {
    for (indices, declarations) in groups {
        for (position, &index) in indices.iter().enumerate() {
            if position != 0 {
                css.push_str(",\n");
            }
            css.push_str(":root:where([data-press-theme=\"");
            css.push_str(&families[index].id);
            css.push_str("\"]");
            if index == default {
                css.push_str(", :not([data-press-theme])");
            }
            css.push(')');
            css.push_str(mode);
        }
        css.push_str(" {\n");
        css.push_str(declarations);
        css.push_str("}\n");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use std::path::Path;

    #[test]
    fn grouped_deltas_reconstruct_every_literal_and_missing_reset() -> Result<()> {
        let sources = vec![
            Some(webui_tokens::parse_token_content(
                r#"{"themes":{"light":{"common":"same","tone":"light","missing":"4px","initial":"initial","alias":"var(--tone)"},"dark":{"common":"same","tone":"dark","initial":"initial","alias":"var(--tone)"}}}"#,
                Path::new("first.json"),
            ).map_err(|error| Error::Build(error.to_string()))?),
            Some(webui_tokens::parse_token_content(
                r#"{"themes":{"light":{"common":"same","tone":"light","initial":"2px","alias":"var(--tone)"},"dark":{"common":"same","tone":"dark","initial":"initial","alias":"var(--tone)"}}}"#,
                Path::new("second.json"),
            ).map_err(|error| Error::Build(error.to_string()))?),
            None,
        ];
        let families: Vec<_> = (0..3)
            .map(|index| FamilyMetadata {
                id: format!("family-{index}"),
                label: format!("Family {index}"),
            })
            .collect();
        let names = BTreeSet::from(["alias", "common", "initial", "missing", "tone"]);
        let active = [0, 1, 2];
        let light = deltas(&sources, &names, &active, Some(0));
        let dark = deltas(&sources, &names, &active, None);
        for family in active {
            let mut effective: HashMap<&str, &str> = names
                .iter()
                .map(|&name| (name, value(&sources, 0, "light", name)))
                .collect();
            for (scheme, groups) in [("light", &light), ("dark", &dark)] {
                for (indices, declarations) in groups {
                    if indices.contains(&family) {
                        for declaration in declarations.lines() {
                            let (name, selected) = declaration
                                .split_once(": ")
                                .ok_or_else(|| Error::Build("Invalid declaration".into()))?;
                            effective.insert(
                                name.trim_start_matches("--"),
                                selected.trim_end_matches(';'),
                            );
                        }
                    }
                }
                for name in &names {
                    assert_eq!(
                        effective.get(name).copied(),
                        Some(value(&sources, family, scheme, name)),
                        "family {family}, {scheme}, {name}"
                    );
                }
            }
        }
        let css = compile(&families, &sources, "family-0", ShowMode::All)?;
        assert_eq!(css.matches("--common: same;").count(), 1);
        assert_eq!(css.matches("--tone: dark;").count(), 2);
        assert_eq!(css.matches("--alias: var(--tone);").count(), 1);
        Ok(())
    }
}

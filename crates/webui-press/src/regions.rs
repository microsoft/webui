// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

//! Compile-time named template regions.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};

use serde_json::{Map, Value};

use crate::error::{Error, Result};
use crate::types::RegionConfig;

mod parser;

use parser::{parse_declarations, RegionDeclaration};

const REGION_TAG: &str = "webui-press-region";
const REGION_STATE_ROOT: &str = "regions";

#[derive(Debug)]
struct LoadedRegion {
    html: String,
    state: Option<Value>,
    script_file: Option<String>,
}

/// Parsed template declarations plus site-owned region content.
#[derive(Debug)]
pub(crate) struct RegionSet {
    template: String,
    declarations: Vec<RegionDeclaration>,
    regions: BTreeMap<String, LoadedRegion>,
}

impl RegionSet {
    pub(crate) fn load(
        configs: &BTreeMap<String, RegionConfig>,
        config_dir: &Path,
        template: String,
    ) -> Result<Self> {
        let declarations = parse_declarations(&template)?;
        let declared: HashSet<&str> = declarations.iter().map(|item| item.name.as_str()).collect();
        for name in configs.keys() {
            if !declared.contains(name.as_str()) {
                return Err(Error::Build(format!(
                    "Region '{name}' is configured but the template does not declare it. \
                     Add <{REGION_TAG} name=\"{name}\"></{REGION_TAG}> or remove the config entry."
                )));
            }
        }

        let mut state_cache = HashMap::new();
        let mut regions = BTreeMap::new();
        for (name, config) in configs {
            let html = load_html(name, config, config_dir)?;
            let state = load_state(name, config, config_dir, &mut state_cache)?;
            regions.insert(
                name.clone(),
                LoadedRegion {
                    html,
                    state,
                    script_file: config.script_file.clone(),
                },
            );
        }

        validate_state_paths(&regions)?;

        Ok(Self {
            template,
            declarations,
            regions,
        })
    }

    pub(crate) fn render(&self, layout: &str) -> String {
        if self.declarations.is_empty() {
            return self.template.clone();
        }

        let extra = self
            .declarations
            .iter()
            .filter(|declaration| declaration_applies(declaration, layout))
            .filter_map(|declaration| self.regions.get(&declaration.name))
            .map(|region| region.html.len())
            .sum();
        let mut output = String::with_capacity(self.template.len().saturating_add(extra));
        let mut cursor = 0;
        for declaration in &self.declarations {
            output.push_str(&self.template[cursor..declaration.start]);
            if declaration_applies(declaration, layout) {
                if let Some(region) = self.regions.get(&declaration.name) {
                    output.push_str(&region.html);
                }
            }
            cursor = declaration.end;
        }
        output.push_str(&self.template[cursor..]);
        output
    }

    pub(crate) fn html_fragments(&self, layout: &str) -> Vec<&str> {
        self.active_regions(layout)
            .into_iter()
            .map(|region| region.html.as_str())
            .collect()
    }

    pub(crate) fn script_files(&self, layout: &str) -> Vec<&str> {
        self.active_regions(layout)
            .into_iter()
            .filter_map(|region| region.script_file.as_deref())
            .collect()
    }

    pub(crate) fn has_state(&self, layout: &str) -> bool {
        self.declarations.iter().any(|declaration| {
            declaration_applies(declaration, layout)
                && self
                    .regions
                    .get(&declaration.name)
                    .is_some_and(|region| region.state.is_some())
        })
    }

    pub(crate) fn apply_state(&self, layout: &str, state: &mut Value) -> Result<()> {
        for declaration in &self.declarations {
            if !declaration_applies(declaration, layout) {
                continue;
            }
            let Some(region_state) = self
                .regions
                .get(&declaration.name)
                .and_then(|region| region.state.as_ref())
            else {
                continue;
            };
            insert_region_state(state, &declaration.name, region_state.clone())?;
        }
        Ok(())
    }

    fn active_regions<'a>(&'a self, layout: &str) -> Vec<&'a LoadedRegion> {
        self.declarations
            .iter()
            .filter(move |declaration| declaration_applies(declaration, layout))
            .filter_map(|declaration| self.regions.get(&declaration.name))
            .collect()
    }
}

fn declaration_applies(declaration: &RegionDeclaration, layout: &str) -> bool {
    declaration
        .layout
        .as_deref()
        .is_none_or(|value| value == layout)
}

fn load_html(name: &str, config: &RegionConfig, config_dir: &Path) -> Result<String> {
    match (&config.html, &config.html_file) {
        (Some(_), Some(_)) => Err(Error::Build(format!(
            "Region '{name}': 'html' and 'htmlFile' are mutually exclusive - pick one."
        ))),
        (Some(html), None) => Ok(html.clone()),
        (None, Some(path)) => {
            read_relative_file(&format!("Region '{name}' htmlFile"), path, config_dir)
        }
        (None, None) => Err(Error::Build(format!(
            "Region '{name}' must define either 'html' or 'htmlFile'."
        ))),
    }
}

fn load_state(
    name: &str,
    config: &RegionConfig,
    config_dir: &Path,
    cache: &mut HashMap<PathBuf, Value>,
) -> Result<Option<Value>> {
    let value = match (&config.state, &config.state_file) {
        (Some(_), Some(_)) => {
            return Err(Error::Build(format!(
                "Region '{name}': 'state' and 'stateFile' are mutually exclusive - pick one."
            )));
        }
        (Some(value), None) => Some(value.clone()),
        (None, Some(path)) => Some(read_json_file(
            &format!("Region '{name}' stateFile"),
            path,
            config_dir,
            cache,
        )?),
        (None, None) => None,
    };
    if value.as_ref().is_some_and(|item| !item.is_object()) {
        return Err(Error::Build(format!(
            "Region '{name}': state/stateFile must be a JSON object."
        )));
    }
    Ok(value)
}

fn read_relative_file(label: &str, path: &str, config_dir: &Path) -> Result<String> {
    let relative = Path::new(path);
    if relative.is_absolute() {
        return Err(Error::Build(format!(
            "{label} must be relative to config.json, got {}",
            relative.display()
        )));
    }
    let absolute = config_dir.join(relative);
    fs::read_to_string(&absolute).map_err(|error| {
        Error::Build(format!(
            "{label} {} cannot be read: {error}",
            absolute.display()
        ))
    })
}

fn read_json_file(
    label: &str,
    path: &str,
    config_dir: &Path,
    cache: &mut HashMap<PathBuf, Value>,
) -> Result<Value> {
    let relative = Path::new(path);
    if relative.is_absolute() {
        return Err(Error::Build(format!(
            "{label} must be relative to config.json, got {}",
            relative.display()
        )));
    }
    let absolute = config_dir.join(relative);
    let key = fs::canonicalize(&absolute).unwrap_or_else(|_| absolute.clone());
    if let Some(value) = cache.get(&key) {
        return Ok(value.clone());
    }
    let raw = fs::read_to_string(&absolute).map_err(|error| {
        Error::Build(format!(
            "{label} {} cannot be read: {error}",
            absolute.display()
        ))
    })?;
    let value: Value = serde_json::from_str(&raw).map_err(|error| {
        Error::Build(format!(
            "{label} {} is not valid JSON: {error}",
            absolute.display()
        ))
    })?;
    cache.insert(key, value.clone());
    Ok(value)
}

fn validate_state_paths(regions: &BTreeMap<String, LoadedRegion>) -> Result<()> {
    for (ancestor_name, ancestor) in regions {
        if ancestor.state.is_none() {
            continue;
        }
        for (descendant_name, descendant) in regions {
            if descendant.state.is_some() && is_dotted_path_prefix(ancestor_name, descendant_name) {
                return Err(Error::Build(format!(
                    "Region state paths '{ancestor_name}' and '{descendant_name}' conflict. \
                     Rename one region or remove its state/stateFile so each state-bearing region \
                     owns a distinct path."
                )));
            }
        }
    }
    Ok(())
}

fn is_dotted_path_prefix(ancestor: &str, descendant: &str) -> bool {
    descendant.len() > ancestor.len()
        && descendant.starts_with(ancestor)
        && descendant.as_bytes().get(ancestor.len()) == Some(&b'.')
}

fn insert_region_state(state: &mut Value, name: &str, value: Value) -> Result<()> {
    let root = state
        .as_object_mut()
        .ok_or_else(|| Error::Build("Page render state must be a JSON object.".to_string()))?;
    let regions = root
        .entry(REGION_STATE_ROOT)
        .or_insert_with(|| Value::Object(Map::new()))
        .as_object_mut()
        .ok_or_else(|| Error::Build("Reserved 'regions' state must be an object.".to_string()))?;

    let mut current = regions;
    let mut segments = name.split('.').peekable();
    while let Some(segment) = segments.next() {
        if segments.peek().is_none() {
            current.insert(segment.to_string(), value);
            return Ok(());
        }
        current = current
            .entry(segment)
            .or_insert_with(|| Value::Object(Map::new()))
            .as_object_mut()
            .ok_or_else(|| {
                Error::Build(format!(
                    "Region state path '{name}' conflicts with another region."
                ))
            })?;
    }
    Ok(())
}

#[cfg(test)]
mod tests;

// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

//! Compile-time named template regions.

use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use serde_json::{Map, Value};

use crate::error::{Error, Result};
use crate::state::StateLoader;
use crate::types::RegionConfig;

mod parser;

use parser::parse_declarations;

const REGION_TAG: &str = "webui-press-region";
const REGION_STATE_ROOT: &str = "regions";

#[derive(Debug)]
struct Region {
    start: usize,
    end: usize,
    name: String,
    layout: Option<String>,
    html: Option<String>,
    state: Option<Value>,
    script_file: Option<String>,
}

/// Parsed template declarations plus site-owned region content.
#[derive(Debug)]
pub(crate) struct RegionSet {
    template: String,
    regions: Vec<Region>,
}

impl RegionSet {
    pub(crate) fn load(
        configs: &BTreeMap<String, RegionConfig>,
        config_dir: &Path,
        template: String,
    ) -> Result<Self> {
        let mut regions = parse_declarations(&template)?;
        for name in configs.keys() {
            if !regions.iter().any(|item| item.name == *name) {
                return Err(Error::Build(format!(
                    "Region '{name}' is configured but the template does not declare it. \
                     Add <{REGION_TAG} name=\"{name}\" /> or \
                     <{REGION_TAG} name=\"{name}\">fallback HTML</{REGION_TAG}>, \
                     or remove the config entry."
                )));
            }
        }

        let mut state_loader = StateLoader::new();
        for region in &mut regions {
            let Some(config) = configs.get(&region.name) else {
                continue;
            };
            if let Some(html) = load_html(&region.name, config, config_dir)? {
                region.html = Some(html);
            }
            region.state = load_state(&region.name, config, config_dir, &mut state_loader)?;
            region.script_file.clone_from(&config.script_file);
        }
        validate_state_paths(&regions)?;

        Ok(Self { template, regions })
    }

    pub(crate) fn render(&self, layout: &str) -> String {
        if self.regions.is_empty() {
            return self.template.clone();
        }

        let extra = self
            .active_regions(layout)
            .filter_map(|region| region.html.as_ref())
            .map(String::len)
            .sum();
        let mut output = String::with_capacity(self.template.len().saturating_add(extra));
        let mut cursor = 0;
        for region in &self.regions {
            output.push_str(&self.template[cursor..region.start]);
            if region_applies(region, layout) {
                if let Some(html) = &region.html {
                    output.push_str(html);
                }
            }
            cursor = region.end;
        }
        output.push_str(&self.template[cursor..]);
        output
    }

    pub(crate) fn template_shell(&self) -> String {
        let mut output = String::with_capacity(self.template.len());
        let mut cursor = 0;
        for region in &self.regions {
            output.push_str(&self.template[cursor..region.start]);
            cursor = region.end;
        }
        output.push_str(&self.template[cursor..]);
        output
    }

    pub(crate) fn html_fragments<'a>(&'a self, layout: &'a str) -> Vec<&'a str> {
        self.active_regions(layout)
            .filter_map(|region| region.html.as_deref())
            .collect()
    }

    pub(crate) fn script_files<'a>(
        &'a self,
        layout: &'a str,
    ) -> impl Iterator<Item = &'a str> + 'a {
        self.active_regions(layout)
            .filter_map(|region| region.script_file.as_deref())
    }

    pub(crate) fn apply_state(&self, layout: &str, state: &mut Value) -> Result<()> {
        for region in &self.regions {
            if !region_applies(region, layout) {
                continue;
            }
            if let Some(region_state) = &region.state {
                insert_region_state(state, &region.name, region_state.clone())?;
            }
        }
        Ok(())
    }

    fn active_regions<'a>(&'a self, layout: &'a str) -> impl Iterator<Item = &'a Region> + 'a {
        self.regions
            .iter()
            .filter(move |region| region_applies(region, layout))
    }
}

fn region_applies(region: &Region, layout: &str) -> bool {
    region.layout.as_deref().is_none_or(|value| value == layout)
}

fn load_html(name: &str, config: &RegionConfig, config_dir: &Path) -> Result<Option<String>> {
    match (&config.html, &config.html_file) {
        (Some(_), Some(_)) => Err(Error::Build(format!(
            "Region '{name}': 'html' and 'htmlFile' are mutually exclusive - pick one."
        ))),
        (Some(html), None) => Ok(Some(html.clone())),
        (None, Some(path)) => {
            read_relative_file(&format!("Region '{name}' htmlFile"), path, config_dir).map(Some)
        }
        (None, None) => Ok(None),
    }
}

fn load_state(
    name: &str,
    config: &RegionConfig,
    config_dir: &Path,
    loader: &mut StateLoader,
) -> Result<Option<Value>> {
    let value = loader.load_state_value(
        &format!("Region '{name}'"),
        config.state.as_ref(),
        config.state_file.as_deref(),
        config_dir,
    )?;
    if value.as_ref().is_some_and(Value::is_null) {
        return Ok(None);
    }
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

fn validate_state_paths(regions: &[Region]) -> Result<()> {
    for ancestor in regions {
        if ancestor.state.is_none() {
            continue;
        }
        for descendant in regions {
            if descendant.state.is_some() && is_dotted_path_prefix(&ancestor.name, &descendant.name)
            {
                return Err(Error::Build(format!(
                    "Region state paths '{}' and '{}' conflict. \
                     Rename one region or remove its state/stateFile so each state-bearing region \
                     owns a distinct path.",
                    ancestor.name, descendant.name
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

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
    content_start: usize,
    content_end: usize,
    parent: Option<usize>,
    name: String,
    layout: Option<String>,
    html: Option<String>,
    state: Option<Value>,
    script_file: Option<String>,
    inactive: bool,
}

/// Parsed template declarations plus site-owned region content.
#[derive(Debug)]
pub(crate) struct RegionSet {
    template: String,
    regions: Vec<Region>,
}

impl RegionSet {
    pub(crate) fn contains(&self, name: &str) -> bool {
        self.regions.iter().any(|region| region.name == name)
    }

    pub(crate) fn deactivate(&mut self, name: &str) {
        for region in &mut self.regions {
            if region.name == name {
                region.inactive = true;
            }
        }
    }

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

    pub(crate) fn render(&self, layout: &str, omitted: Option<&str>) -> String {
        let fragments = self.html_fragments(layout, omitted);
        let mut output = String::with_capacity(fragments.iter().map(|html| html.len()).sum());
        for fragment in fragments {
            output.push_str(fragment);
        }
        output
    }

    pub(crate) fn template_shell(&self) -> String {
        let mut output = String::with_capacity(self.template.len());
        let mut cursor = 0;
        for region in self.regions.iter().filter(|region| region.parent.is_none()) {
            output.push_str(&self.template[cursor..region.start]);
            cursor = region.end;
        }
        output.push_str(&self.template[cursor..]);
        output
    }

    pub(crate) fn html_fragments<'a>(
        &'a self,
        layout: &'a str,
        omitted: Option<&'a str>,
    ) -> Vec<&'a str> {
        let mut fragments = Vec::with_capacity(self.regions.len() * 2 + 1);
        let mut parents: Vec<&Region> = Vec::new();
        let mut cursor = 0;
        for region in &self.regions {
            if region.start < cursor {
                continue;
            }
            while let Some(parent) = parents.last() {
                if parent.content_end > region.start {
                    break;
                }
                push_fragment(&mut fragments, &self.template[cursor..parent.content_end]);
                cursor = parent.end;
                parents.pop();
            }
            push_fragment(&mut fragments, &self.template[cursor..region.start]);
            if !region_applies(region, layout, omitted) {
                cursor = region.end;
            } else if region.html.is_some() {
                if let Some(html) = &region.html {
                    push_fragment(&mut fragments, html);
                }
                cursor = region.end;
            } else {
                cursor = region.content_start;
                if region.content_end < region.end {
                    parents.push(region);
                }
            }
        }
        while let Some(parent) = parents.pop() {
            push_fragment(&mut fragments, &self.template[cursor..parent.content_end]);
            cursor = parent.end;
        }
        push_fragment(&mut fragments, &self.template[cursor..]);
        fragments
    }

    pub(crate) fn script_files<'a>(
        &'a self,
        layout: &'a str,
        omitted: Option<&'a str>,
    ) -> impl Iterator<Item = &'a str> + 'a {
        self.active_regions(layout, omitted)
            .filter_map(|region| region.script_file.as_deref())
    }

    pub(crate) fn apply_state(
        &self,
        layout: &str,
        state: &mut Value,
        omitted: Option<&str>,
    ) -> Result<()> {
        for region in self.active_regions(layout, omitted) {
            if let Some(region_state) = &region.state {
                insert_region_state(state, &region.name, region_state.clone())?;
            }
        }
        Ok(())
    }

    fn active_regions<'a>(
        &'a self,
        layout: &'a str,
        omitted: Option<&'a str>,
    ) -> impl Iterator<Item = &'a Region> + 'a {
        let mut skip_until = 0;
        self.regions.iter().filter(move |region| {
            if region.start < skip_until {
                return false;
            }
            let active = region_applies(region, layout, omitted);
            if !active || region.html.is_some() {
                skip_until = region.end;
            }
            active
        })
    }
}

fn push_fragment<'a>(fragments: &mut Vec<&'a str>, html: &'a str) {
    if !html.is_empty() {
        fragments.push(html);
    }
}

fn region_applies(region: &Region, layout: &str, omitted: Option<&str>) -> bool {
    !region.inactive
        && omitted != Some(region.name.as_str())
        && region.layout.as_deref().is_none_or(|value| {
            value == layout || value.split_ascii_whitespace().any(|name| name == layout)
        })
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

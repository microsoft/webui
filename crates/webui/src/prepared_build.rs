// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

use std::collections::HashSet;
use std::time::{Duration, Instant};

use crate::{
    build_protocol_inner, component_assets, invalid_asset_base_option, lazy_report, lazy_scripts,
    module_preload, projection, streaming_without_projection_warning, validate_asset_base,
    BuildOptions, BuildResult, BuildStats, ComponentAssetFile, GeneratedModule, LazyPackFile,
    Plugin, ProjectionManifestSource, RawBuildOutput, WebUIError,
};

/// A compiled application retained while its generated modules are bundled.
///
/// Preparation owns the template/CSS snapshot; finishing consumes it without
/// rediscovery or recompilation. No protocol bytes are published before finishing.
#[must_use]
pub struct PreparedBuild {
    options: BuildOptions,
    raw: RawBuildOutput,
    lazy_pack_files: Vec<LazyPackFile>,
    lazy_scripts_module: Option<GeneratedModule>,
    duration: Duration,
}

/// Compile once and expose projection-independent inputs for the client bundler.
///
/// Write the generated module and component assets, bundle the client, then pass
/// its completed projection manifests to [`PreparedBuild::finish`].
pub fn prepare_build(options: BuildOptions) -> Result<PreparedBuild, WebUIError> {
    let started = Instant::now();
    if let Some(base) = options.asset_base.as_deref() {
        validate_asset_base(base).map_err(invalid_asset_base_option)?;
    }
    lazy_scripts::validate_mappings(&options.component_modules)?;
    let mut raw = build_protocol_inner(&options)?;
    // Pack payloads and generated imports do not depend on state projection.
    let mut lazy_pack_files = webui_handler::lazy_packs::emit(&mut raw.protocol)?;
    let startup = if lazy_pack_files.is_empty() {
        HashSet::new()
    } else {
        webui_handler::lazy_packs::startup_components(&raw.protocol, &options.entry)
    };
    let lazy_scripts_module = lazy_scripts::resolve(
        &mut lazy_pack_files,
        &lazy_scripts::ScriptSources {
            scripts: &raw.component_scripts,
            mappings: &options.component_modules,
            modules_dir: options.modules_out.as_deref().unwrap_or(&options.app_dir),
            startup: &startup,
        },
        &mut raw.warnings,
    )?;
    Ok(PreparedBuild {
        options,
        raw,
        lazy_pack_files,
        lazy_scripts_module,
        duration: started.elapsed(),
    })
}

impl PreparedBuild {
    /// Generated lazy behavior imports to write before bundling the client.
    #[must_use]
    pub fn lazy_scripts_module(&self) -> Option<&GeneratedModule> {
        self.lazy_scripts_module.as_ref()
    }

    /// Projection-independent component modules to write before client bundling.
    #[must_use]
    pub fn component_asset_files(&self) -> &[ComponentAssetFile] {
        &self.raw.component_asset_files
    }

    /// Projection-independent pack payloads and region metadata.
    #[must_use]
    pub fn lazy_pack_files(&self) -> &[LazyPackFile] {
        &self.lazy_pack_files
    }

    /// Atomically publish bundler inputs to `modules_out` (or `app_dir`).
    ///
    /// Uses the same collision checks, dependency-first publication, and
    /// unchanged-file handling as [`crate::build_to_disk`].
    pub fn write_modules(&self) -> Result<(), WebUIError> {
        crate::component_asset_output::publish_modules_with(
            self.options
                .modules_out
                .as_deref()
                .unwrap_or(&self.options.app_dir),
            &self.raw.component_asset_files,
            self.lazy_scripts_module.as_slice(),
            || Ok(()),
        )
    }

    /// Validate completed manifests and finalize the retained compilation.
    ///
    /// These sources are merged with any supplied in the preparation options.
    /// Validation retains the one-shot build's input/output hashes and coverage
    /// checks. Final identity and protobuf serialization happen only afterward.
    pub fn finish(
        mut self,
        projection_manifests: Vec<ProjectionManifestSource>,
    ) -> Result<BuildResult, WebUIError> {
        let started = Instant::now();
        self.options
            .projection_manifests
            .extend(projection_manifests);
        apply_projection(&mut self.raw, &self.options)?;
        inherit_fragment_state(&mut self.raw);
        self.raw.protocol.asset_base = self.options.asset_base.unwrap_or_default();
        self.raw.protocol.assign_build_id()?;
        let protocol_bytes = self.raw.protocol.to_protobuf()?;
        let metafile = if self.options.metafile
            && (self.raw.asset_metafile.is_some() || !self.lazy_pack_files.is_empty())
        {
            lazy_report::measure(&mut self.lazy_pack_files)?;
            Some(component_assets::render_metafile(
                self.raw.asset_metafile.as_ref(),
                &self.lazy_pack_files,
            )?)
        } else {
            None
        };
        let stats = BuildStats {
            duration: self.duration + started.elapsed(),
            fragment_count: self.raw.fragment_count,
            component_count: self.raw.component_count,
            css_file_count: self.raw.css_files.len(),
            protocol_size_bytes: protocol_bytes.len(),
            token_count: self.raw.token_count,
        };
        Ok(BuildResult {
            protocol: self.raw.protocol,
            protocol_bytes,
            css_files: self.raw.css_files,
            component_asset_files: self.raw.component_asset_files,
            metafile,
            lazy_pack_files: self.lazy_pack_files,
            lazy_scripts_module: self.lazy_scripts_module,
            component_templates: self.raw.component_templates,
            warnings: self.raw.warnings,
            stats,
        })
    }
}

fn apply_projection(raw: &mut RawBuildOutput, options: &BuildOptions) -> Result<(), WebUIError> {
    if !options.projection_manifests.is_empty() && options.plugin != Some(Plugin::WebUI) {
        return Err(projection::incompatible_plugin_error());
    }
    let Some(merged) = projection::load_and_merge(&options.projection_manifests)? else {
        if raw.boundary_count > 0 {
            raw.warnings.push(streaming_without_projection_warning(
                &options.entry,
                raw.boundary_count,
            ));
        }
        return Ok(());
    };
    let scripted_tags: Vec<&str> = raw
        .component_templates
        .iter()
        .filter(|artifact| {
            artifact.is_scripted && raw.protocol.fragments.contains_key(&artifact.tag_name)
        })
        .map(|artifact| artifact.tag_name.as_str())
        .collect();
    projection::validate_coverage(&merged.components, &scripted_tags)?;
    raw.protocol.initial_state_strategy = webui_protocol::InitialStateStrategy::Components as i32;
    for artifact in &raw.component_templates {
        if !artifact.is_scripted {
            continue;
        }
        let Some(entry) = merged.components.get(&artifact.tag_name) else {
            continue;
        };
        let Some(component) = raw.protocol.components.get_mut(&artifact.tag_name) else {
            continue;
        };
        component.hydration_mode = webui_protocol::StateProjectionMode::Keys as i32;
        component.hydration_keys.clone_from(&entry.hydration_keys);
        component.navigation_mode = Some(webui_protocol::StateProjectionMode::Keys as i32);
        component.navigation_keys =
            projection::union_keys(&entry.navigation_keys, &artifact.template_roots);
    }
    let mut preloads = module_preload::resolve(&merged.entry_closures, &raw.module_entry_srcs);
    raw.protocol.module_preloads = preloads.hrefs;
    raw.warnings.append(&mut preloads.warnings);
    Ok(())
}

fn inherit_fragment_state(raw: &mut RawBuildOutput) {
    for artifact in &raw.component_templates {
        let Some(component) = raw.protocol.components.get(&artifact.tag_name) else {
            continue;
        };
        for fragment in &artifact.lazy_templates {
            if let Some(data) = raw.protocol.client_fragments.get_mut(&fragment.id) {
                data.inherit_owner_state(component);
            }
        }
    }
}

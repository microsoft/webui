// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

//! Build-only browser compilation and same-invocation source evidence.
//! Outputs remain in memory; callers own proof finalization and publication.

mod config;
mod decorators;
mod graph;
mod loaders;
mod manifest;
mod metadata;
mod options;
mod output;
mod parallel;
mod publish;
mod resolver_path;
mod resources;
mod urls;
mod warnings;

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use rolldown::Bundler;

pub use graph::{Import, Module, ModuleGraph, ScriptSyntax};
pub use metadata::{
    BundleMetadata, ImportKind, InputMetadata, MetadataImport, ModuleContribution, OutputMetadata,
};
pub use options::{ClientBuildOptions, Format, Loader, SourceMap};
pub use warnings::BuildWarning;

/// Browser output and its source graph from the same bundler invocation.
pub struct ClientBuild {
    project: PathBuf,
    /// Final emitted files, keyed by their project-relative output paths.
    pub files: BTreeMap<PathBuf, Vec<u8>>,
    /// Original source and bundler-resolved edges used for state analysis.
    pub graph: ModuleGraph,
    /// Output paths to the modules retained after tree shaking.
    pub membership: BTreeMap<PathBuf, Vec<String>>,
    /// Project-relative internal static output dependencies.
    pub imports: BTreeMap<PathBuf, Vec<String>>,
    /// Entry chunks whose static closures may be preloaded.
    pub entries: BTreeSet<PathBuf>,
    /// Native source/output inventory.
    pub metadata: BundleMetadata,
    /// Canonical module IDs to exact original-byte SHA-256 hashes.
    pub input_hashes: BTreeMap<String, String>,
    /// Explicitly permitted native advisories; hosts must surface these.
    pub warnings: Vec<BuildWarning>,
    /// Canonical physical sources and native-reported configuration/package
    /// inputs. Pinned Rolldown does not expose inherited tsconfig read identities.
    pub dependencies: BTreeSet<PathBuf>,
}

impl ClientBuild {
    /// Canonical project boundary anchoring every returned output path.
    #[must_use]
    pub fn project(&self) -> &Path {
        &self.project
    }

    /// Prove this bundle's exact state surfaces and final bytes.
    pub fn projection_manifest(
        &self,
        path: &Path,
    ) -> Result<webui_protocol::projection_manifest::ProjectionManifest, ClientBuildError> {
        manifest::build(self, path)
    }

    /// Explicitly write guarded outputs and their native proof, manifest last.
    ///
    /// Accepts a project-relative or canonical-contained absolute manifest path.
    /// Individual files are atomically replaced; the caller owns generation-level
    /// promotion and must serialize concurrent publishers of the same destination.
    /// Existing outputs must be owned by the previous native proof; unrelated
    /// existing files and portable case-only collisions are rejected.
    pub fn write_outputs(
        &self,
        manifest_path: &Path,
    ) -> Result<webui_protocol::projection_manifest::ProjectionManifest, ClientBuildError> {
        publish::write(self, manifest_path)
    }

    /// Exact stable staging subtrees used by explicit publication. Watchers may
    /// ignore these roots, but must continue observing unrelated neighboring files.
    pub fn publication_temporary_roots(
        &self,
        manifest_path: &Path,
    ) -> Result<BTreeSet<PathBuf>, ClientBuildError> {
        publish::temporary_roots(self, manifest_path)
    }
}

/// A recoverable native asset-build failure.
#[derive(Debug, thiserror::Error)]
pub enum ClientBuildError {
    /// The project, resource mapping or graph violates the build contract.
    #[error("invalid native client build: {0}")]
    Invalid(String),
    /// A physical input could not be read.
    #[error("could not read native client input {}", path.display())]
    Read {
        /// Input that failed.
        path: PathBuf,
        /// Underlying filesystem failure.
        #[source]
        source: std::io::Error,
    },
    /// Explicit publication could not complete.
    #[error("could not write native client output {}", path.display())]
    Write {
        /// Destination that failed.
        path: PathBuf,
        /// Underlying filesystem failure.
        #[source]
        source: std::io::Error,
    },
    /// The native bundler rejected the application.
    #[error("native browser build failed: {0}")]
    Bundle(String),
    /// Exact state surfaces could not be proven.
    #[error(transparent)]
    Projection(#[from] webui_projection::ProjectionError),
    /// The completed manifest violates the shared protocol contract.
    #[error(transparent)]
    Manifest(#[from] webui_protocol::projection_manifest::ProjectionManifestError),
}

/// Compile browser assets without executing JavaScript or writing outputs.
///
/// Unexpected warnings, missing exports, escaping paths and unsupported evidence
/// fail. Valid ineffective dynamic imports return an explicit advisory.
pub async fn build(options: ClientBuildOptions) -> Result<ClientBuild, ClientBuildError> {
    let config::Prepared {
        project,
        out,
        framework,
        bundler_options,
        mut dependencies,
    } = config::prepare(&options)?;
    let urls = urls::Urls::new(
        options.public_path.clone(),
        options.parallel_dynamic_imports,
    );
    let decorators = decorators::LegacyDecorators {
        project: resolver_path::path(&project)?.into(),
    };
    let capture = graph::Capture::new(project.clone(), framework, options);
    let mut bundler = Bundler::with_plugins(
        bundler_options,
        vec![
            rolldown::plugin::Plugin::new_shared(capture.clone()),
            rolldown::plugin::Plugin::new_shared(decorators),
            rolldown::plugin::Plugin::new_shared(urls.clone()),
        ],
    )
    .map_err(|error| ClientBuildError::Bundle(error.to_string()))?;
    let output = bundler.generate().await.map_err(|errors| {
        ClientBuildError::Bundle(
            errors
                .into_vec()
                .iter()
                .map(|error| error.to_diagnostic().convert_to_string(false))
                .collect::<Vec<_>>()
                .join("\n"),
        )
    })?;
    let warnings = warnings::collect(output.warnings)?;
    for path in bundler.watch_files().iter() {
        dependencies.insert(resources::contained(&project, Path::new(path.as_str()))?);
    }
    // Release Rolldown's analysis/cache lifetime before assembling the public result.
    drop(bundler);
    let evidence = capture.finish()?;
    dependencies.extend(evidence.dependencies);
    drop(capture);
    let output_imports = urls.finish()?;
    let mut result = ClientBuild {
        project,
        files: BTreeMap::new(),
        graph: evidence.graph,
        membership: BTreeMap::new(),
        imports: BTreeMap::new(),
        entries: BTreeSet::new(),
        metadata: BundleMetadata {
            inputs: evidence.inputs,
            outputs: BTreeMap::new(),
        },
        input_hashes: evidence.hashes,
        warnings,
        dependencies,
    };
    output::collect(&mut result, &out, output.assets, &output_imports)?;
    for (id, input) in &mut result.metadata.inputs {
        if let Some(module) = result.graph.modules.get(id) {
            input.imports = module
                .imports
                .iter()
                .map(|edge| MetadataImport {
                    path: edge.resolved.clone(),
                    kind: if edge.dynamic {
                        ImportKind::Dynamic
                    } else {
                        ImportKind::Static
                    },
                    external: edge.external,
                })
                .collect();
        }
    }
    Ok(result)
}

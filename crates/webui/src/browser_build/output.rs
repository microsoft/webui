// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;
use std::sync::Arc;

use rolldown_common::Output;

use super::{
    resources, urls, ClientBuild, ClientBuildError, ImportKind, MetadataImport, ModuleContribution,
    OutputMetadata,
};

pub(super) fn collect(
    result: &mut ClientBuild,
    out: &Path,
    assets: Vec<Output>,
    imports: &BTreeMap<String, urls::InternalImports>,
) -> Result<(), ClientBuildError> {
    let filenames: BTreeSet<_> = assets
        .iter()
        .map(|asset| asset.filename().to_owned())
        .collect();
    let names = assets
        .iter()
        .filter_map(|asset| {
            if let Output::Chunk(chunk) = asset {
                Some(
                    resources::relative_output(result.project(), out, asset.filename()).map(
                        |path| {
                            (
                                chunk.preliminary_filename.clone(),
                                resources::portable(&path),
                            )
                        },
                    ),
                )
            } else {
                None
            }
        })
        .collect::<Result<BTreeMap<_, _>, ClientBuildError>>()?;
    for asset in assets {
        let relative = resources::relative_output(result.project(), out, asset.filename())?;
        let mut metadata = OutputMetadata {
            bytes: asset.content_as_bytes().len(),
            ..Default::default()
        };
        if let Output::Chunk(chunk) = &asset {
            let internal = imports.get(&chunk.preliminary_filename).ok_or_else(|| {
                ClientBuildError::Invalid(
                    "native chunk has no same-invocation import evidence".into(),
                )
            })?;
            if chunk.is_entry {
                result.entries.insert(relative.clone());
            }
            metadata.entry_point = chunk.facade_module_id.as_ref().map(ToString::to_string);
            result.membership.insert(
                relative.clone(),
                chunk.module_ids.iter().map(ToString::to_string).collect(),
            );
            for (id, module) in chunk.modules.keys.iter().zip(&chunk.modules.values) {
                metadata.inputs.insert(
                    id.to_string(),
                    ModuleContribution {
                        rendered_bytes_before_minify: module.rendered_byte_length(),
                    },
                );
            }
            let mut statics = Vec::new();
            for (imports, dynamic, count) in [
                (&chunk.imports, false, internal.static_count),
                (&chunk.dynamic_imports, true, internal.dynamic_count),
            ] {
                if imports.len() < count {
                    return Err(ClientBuildError::Invalid(
                        "native output lost its internal dependency partition".into(),
                    ));
                }
                for (index, import) in imports.iter().enumerate() {
                    let external = index >= count;
                    if !external && !filenames.contains(import.as_str()) {
                        return Err(ClientBuildError::Invalid(
                            "native output imports a missing emitted file".into(),
                        ));
                    }
                    let path = if external {
                        import.to_string()
                    } else {
                        resources::portable(&resources::relative_output(
                            result.project(),
                            out,
                            import,
                        )?)
                    };
                    if !external && !dynamic {
                        statics.push(path.clone());
                    }
                    metadata.imports.push(MetadataImport {
                        path,
                        external,
                        kind: if dynamic {
                            ImportKind::Dynamic
                        } else {
                            ImportKind::Static
                        },
                    });
                }
            }
            result.imports.insert(relative.clone(), statics);
            collect_parallel(&mut metadata, &internal.parallel, &names)?;
        }
        result.membership.entry(relative.clone()).or_default();
        result
            .metadata
            .outputs
            .insert(resources::portable(&relative), metadata);
        let bytes = match asset {
            Output::Chunk(chunk) => Arc::try_unwrap(chunk).map_or_else(
                |chunk| chunk.code.as_bytes().to_vec(),
                |chunk| chunk.code.into_bytes(),
            ),
            Output::Asset(asset) => Arc::try_unwrap(asset).map_or_else(
                |asset| asset.source.as_bytes().to_vec(),
                |asset| match asset.source {
                    rolldown_common::StrOrBytes::Str(text) => text.into_bytes(),
                    rolldown_common::StrOrBytes::Bytes(bytes) => bytes,
                },
            ),
        };
        if result.files.insert(relative, bytes).is_some() {
            return Err(ClientBuildError::Invalid(
                "bundler produced duplicate output paths".into(),
            ));
        }
    }
    Ok(())
}

fn collect_parallel(
    metadata: &mut OutputMetadata,
    plans: &BTreeMap<String, Vec<String>>,
    names: &BTreeMap<String, String>,
) -> Result<(), ClientBuildError> {
    let mut dynamic: BTreeSet<_> = metadata
        .imports
        .iter()
        .filter(|edge| !edge.external && matches!(edge.kind, ImportKind::Dynamic))
        .map(|edge| edge.path.clone())
        .collect();
    for (target, dependencies) in plans {
        let target = names.get(target).ok_or_else(|| {
            ClientBuildError::Invalid("parallel target has no final native output identity".into())
        })?;
        let mut final_dependencies = Vec::with_capacity(dependencies.len());
        for dependency in dependencies {
            let path = names.get(dependency).ok_or_else(|| {
                ClientBuildError::Invalid(
                    "parallel dependency has no final native output identity".into(),
                )
            })?;
            final_dependencies.push(path.clone());
            if dynamic.insert(path.clone()) {
                metadata.imports.push(MetadataImport {
                    path: path.clone(),
                    kind: ImportKind::Dynamic,
                    external: false,
                });
            }
        }
        metadata
            .parallel_dynamic_imports
            .insert(target.clone(), final_dependencies);
    }
    Ok(())
}

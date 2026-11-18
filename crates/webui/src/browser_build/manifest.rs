// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write;
use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};
use webui_protocol::projection_manifest::{
    ProjectionAdapter, ProjectionComponent, ProjectionManifest, ProjectionProducer, PRODUCER_NAME,
    SCHEMA_ID,
};

use super::{resources, ClientBuild, ClientBuildError, Import};

// Keep aligned with the exact workspace pin; this identity is part of the proof.
const BUNDLER: &str = "rolldown@1.3.2";

pub(super) fn build(
    bundle: &ClientBuild,
    path: &Path,
) -> Result<ProjectionManifest, ClientBuildError> {
    resources::output_path(&bundle.project, path)?;
    let parents = path
        .parent()
        .map_or(0, |parent| parent.components().count());
    let root = if parents == 0 {
        ".".to_owned()
    } else {
        std::iter::repeat_n("..", parents)
            .collect::<Vec<_>>()
            .join("/")
    };
    let identities: BTreeMap<_, _> = bundle
        .graph
        .modules
        .keys()
        .map(|id| {
            bundle
                .metadata
                .inputs
                .get(id)
                .map(|input| (id.as_str(), input.source.clone()))
                .ok_or_else(|| {
                    ClientBuildError::Invalid(format!(
                        "module {id:?} has no captured input identity"
                    ))
                })
        })
        .collect::<Result<_, ClientBuildError>>()?;
    validate_sources(bundle)?;
    let surfaces = webui_projection::analyze(&bundle.graph)?;
    let mut components = BTreeMap::new();
    for (tag, component) in surfaces {
        let outputs: Vec<_> = bundle
            .membership
            .iter()
            .filter(|(_, members)| members.contains(&component.module))
            .map(|(output, _)| portable(output))
            .collect();
        if outputs.is_empty() {
            continue;
        }
        let module = identities.get(component.module.as_str()).ok_or_else(|| {
            ClientBuildError::Invalid("component module is absent from the source graph".into())
        })?;
        components.insert(
            tag,
            ProjectionComponent {
                module: module.clone(),
                outputs,
                hydration_keys: component.hydration_keys.into_iter().collect(),
                navigation_keys: component.navigation_keys.into_iter().collect(),
            },
        );
    }
    let inputs = bundle
        .graph
        .modules
        .keys()
        .map(|id| {
            let hash = if id.starts_with('\0') {
                "virtual".to_owned()
            } else {
                bundle.input_hashes.get(id).cloned().ok_or_else(|| {
                    ClientBuildError::Invalid("physical input has no captured byte hash".into())
                })?
            };
            Ok((identities[id.as_str()].clone(), hash))
        })
        .collect::<Result<_, ClientBuildError>>()?;
    let outputs = bundle
        .files
        .iter()
        .map(|(path, bytes)| (portable(path), hash(bytes)))
        .collect();
    let analysis_hash = analysis_hash(bundle, &identities)?;
    let entry_closures = entry_closures(bundle)?;
    let mut manifest = ProjectionManifest {
        schema: SCHEMA_ID.into(),
        producer: ProjectionProducer {
            name: PRODUCER_NAME.into(),
            version: env!("CARGO_PKG_VERSION").into(),
        },
        adapter: ProjectionAdapter {
            name: "rolldown".into(),
            bundler: BUNDLER.into(),
        },
        root,
        analysis_hash,
        build_id: String::new(),
        inputs,
        outputs,
        components,
        entry_closures,
    };
    manifest.build_id = manifest.compute_build_id();
    manifest.validate()?;
    Ok(manifest)
}

fn portable(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

pub(super) fn hash(bytes: &[u8]) -> String {
    hex("sha256:", Sha256::digest(bytes).as_slice())
}

pub(super) fn hex(prefix: &str, bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut output = String::with_capacity(prefix.len() + bytes.len() * 2);
    output.push_str(prefix);
    for byte in bytes {
        output.push(char::from(HEX[usize::from(byte >> 4)]));
        output.push(char::from(HEX[usize::from(byte & 15)]));
    }
    output
}

fn validate_sources(bundle: &ClientBuild) -> Result<(), ClientBuildError> {
    for (id, module) in &bundle.graph.modules {
        let captured = bundle.metadata.inputs.get(id).ok_or_else(|| {
            ClientBuildError::Invalid("module has no native input classification".into())
        })?;
        if captured.opaque != module.opaque || captured.syntax != module.syntax {
            return Err(ClientBuildError::Invalid(
                "module classification changed after native capture; rebuild before requesting a proof".into(),
            ));
        }
        if let Some(source) = &module.source {
            if bundle
                .input_hashes
                .get(id)
                .is_none_or(|captured| *captured != hash(source.as_bytes()))
            {
                return Err(ClientBuildError::Invalid(
                    "the source graph changed after native capture; rebuild before requesting a proof".into(),
                ));
            }
        }
    }
    Ok(())
}

fn record<'a>(hash: &mut Sha256, name: &str, fields: impl IntoIterator<Item = &'a str>) {
    hash.update(name.as_bytes());
    let mut length = String::with_capacity(24);
    for field in fields {
        length.clear();
        let _ = write!(length, "{}:", field.len());
        hash.update(length.as_bytes());
        hash.update(field.as_bytes());
    }
    hash.update(b"\n");
}

fn analysis_hash(
    bundle: &ClientBuild,
    identities: &BTreeMap<&str, String>,
) -> Result<String, ClientBuildError> {
    let mut hash = Sha256::new();
    record(
        &mut hash,
        "entries",
        [bundle.graph.entries.len().to_string().as_str()],
    );
    let mut entries: Vec<_> = bundle
        .graph
        .entries
        .iter()
        .map(|id| &identities[id.as_str()])
        .collect();
    entries.sort_unstable();
    for entry in entries {
        record(&mut hash, "entry", [entry.as_str()]);
    }
    record(
        &mut hash,
        "modules",
        [bundle.graph.modules.len().to_string().as_str()],
    );
    let mut modules: Vec<_> = bundle.graph.modules.iter().collect();
    modules.sort_unstable_by_key(|(id, _)| &identities[id.as_str()]);
    for (id, module) in modules {
        let mut edges: Vec<_> = module.imports.iter().collect();
        edges.sort_unstable_by(|left, right| import_order(left).cmp(&import_order(right)));
        let mut fields = vec![
            identities[id.as_str()].clone(),
            if module.opaque {
                "opaque"
            } else if id.starts_with('\0') {
                "virtual"
            } else {
                "file"
            }
            .into(),
            module.package_name.clone().unwrap_or_default(),
            bundle.input_hashes.get(id).cloned().ok_or_else(|| {
                ClientBuildError::Invalid("module has no captured source hash".into())
            })?,
            edges.len().to_string(),
        ];
        for edge in edges {
            let target = if edge.resolved.is_empty() {
                String::new()
            } else {
                identities
                    .get(edge.resolved.as_str())
                    .cloned()
                    .map_or_else(
                        || {
                            if edge.external {
                                Ok(edge.resolved.replace('\\', "/"))
                            } else {
                                Err(ClientBuildError::Invalid(
                                    "internal edge has no captured identity".into(),
                                ))
                            }
                        },
                        Ok,
                    )?
            };
            fields.extend([
                edge.specifier.clone(),
                target,
                if edge.external {
                    "external"
                } else {
                    "internal"
                }
                .into(),
                if edge.dynamic { "dynamic" } else { "static" }.into(),
                edge.package_name.clone().unwrap_or_default(),
            ]);
        }
        record(&mut hash, "module", fields.iter().map(String::as_str));
        if let Some(syntax) = module.syntax {
            let name = match syntax {
                super::ScriptSyntax::Js => "js",
                super::ScriptSyntax::Jsx => "jsx",
                super::ScriptSyntax::Ts => "ts",
                super::ScriptSyntax::Tsx => "tsx",
            };
            record(&mut hash, "syntax", [&identities[id.as_str()], name]);
        }
    }
    record(
        &mut hash,
        "memberships",
        [bundle.membership.len().to_string().as_str()],
    );
    for (output, members) in &bundle.membership {
        let mut fields = vec![portable(output), members.len().to_string()];
        let mut members = members
            .iter()
            .map(|member| {
                identities.get(member.as_str()).cloned().ok_or_else(|| {
                    ClientBuildError::Invalid(
                        "output references an unavailable source module".into(),
                    )
                })
            })
            .collect::<Result<Vec<_>, _>>()?;
        members.sort_unstable();
        fields.extend(members);
        record(&mut hash, "membership", fields.iter().map(String::as_str));
    }
    Ok(hex("sha256:", hash.finalize().as_slice()))
}

fn import_order(edge: &Import) -> (&str, &str, bool, &str, &str) {
    (
        &edge.specifier,
        &edge.resolved,
        edge.external,
        if edge.dynamic { "dynamic" } else { "static" },
        edge.package_name.as_deref().unwrap_or(""),
    )
}

fn entry_closures(bundle: &ClientBuild) -> Result<BTreeMap<String, Vec<String>>, ClientBuildError> {
    let mut result = BTreeMap::new();
    for entry in &bundle.entries {
        let mut pending = vec![entry.clone()];
        let mut seen = BTreeSet::new();
        seen.insert(entry.clone());
        let mut closure = Vec::<PathBuf>::new();
        while let Some(module) = pending.pop() {
            for import in bundle.imports.get(&module).into_iter().flatten() {
                let path = PathBuf::from(import);
                resources::output_path(&bundle.project, &path)?;
                if bundle.files.contains_key(&path) && seen.insert(path.clone()) {
                    closure.push(path.clone());
                    pending.push(path);
                }
            }
        }
        closure.sort_unstable_by(|left, right| {
            bundle.files[right]
                .len()
                .cmp(&bundle.files[left].len())
                .then_with(|| portable(left).cmp(&portable(right)))
        });
        result.insert(
            portable(entry),
            closure.iter().map(|path| portable(path)).collect(),
        );
    }
    Ok(result)
}

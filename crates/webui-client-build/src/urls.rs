// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

use std::borrow::Cow;
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use oxc::allocator::Allocator;
use oxc::ast::{ast::Expression, AstKind};
use oxc::parser::Parser;
use oxc::semantic::SemanticBuilder;
use oxc::span::SourceType;
use rolldown::plugin::{
    HookRenderChunkArgs, HookRenderChunkOutput, HookRenderChunkReturn, HookResolveFileUrlArgs,
    HookResolveFileUrlReturn, HookTransformOutputMap, HookUsage, Plugin, PluginContext,
};
use string_wizard::{MagicString, SourceMapOptions};

use crate::{loaders::ascii_literal, parallel, resources, ClientBuildError};

#[derive(Debug)]
pub(super) struct InternalImports {
    pub static_count: usize,
    pub dynamic_count: usize,
    pub parallel: BTreeMap<String, Vec<String>>,
}

#[derive(Debug, Clone)]
pub(super) struct Urls {
    prefix: Option<String>,
    parallel: bool,
    plan_bytes: Arc<AtomicUsize>,
    imports: Arc<Mutex<BTreeMap<String, InternalImports>>>,
}

impl Urls {
    pub(super) fn new(prefix: Option<String>, parallel: bool) -> Self {
        Self {
            prefix,
            parallel,
            plan_bytes: Arc::default(),
            imports: Arc::default(),
        }
    }

    fn reserve_plan(&self, bytes: usize) -> anyhow::Result<()> {
        self.plan_bytes.try_update(Ordering::Relaxed, Ordering::Relaxed, |current| {
            current.checked_add(bytes).filter(|next| *next <= 64 * 1024 * 1024)
        }).map_err(|_| anyhow::anyhow!("parallel dynamic import plans exceed the supported 64 MiB graph evidence limit"))?;
        Ok(())
    }

    pub(super) fn finish(&self) -> Result<BTreeMap<String, InternalImports>, ClientBuildError> {
        let mut imports = self
            .imports
            .lock()
            .map_err(|error| ClientBuildError::Invalid(error.to_string()))?;
        Ok(std::mem::take(&mut *imports))
    }
}

impl Plugin for Urls {
    fn name(&self) -> Cow<'static, str> {
        Cow::Borrowed("webui-native-output-urls")
    }

    fn register_hook_usage(&self) -> HookUsage {
        HookUsage::RenderChunk | HookUsage::ResolveFileUrl
    }

    async fn resolve_file_url(
        &self,
        _context: &PluginContext,
        args: &HookResolveFileUrlArgs<'_>,
    ) -> HookResolveFileUrlReturn {
        let Some(prefix) = &self.prefix else {
            return Ok(None);
        };
        Ok(Some(ascii_literal(&join(prefix, args.file_name))?))
    }

    async fn render_chunk(
        &self,
        _context: &PluginContext,
        args: &HookRenderChunkArgs<'_>,
    ) -> HookRenderChunkReturn {
        let chunk = args
            .chunk_table
            .get(args.chunk.originate_from)
            .ok_or_else(|| anyhow::anyhow!("native output chunk has no owning graph record"))?;
        // Rolldown's public output arrays are internal chunks followed by external
        // modules. Capture the authoritative partition, never guess from filenames.
        self.imports
            .lock()
            .map_err(|error| anyhow::anyhow!("{error}"))?
            .insert(
                args.chunk.filename.to_string(),
                InternalImports {
                    static_count: chunk.cross_chunk_imports.len(),
                    dynamic_count: chunk.cross_chunk_dynamic_imports.len(),
                    parallel: BTreeMap::new(),
                },
            );
        if self.prefix.is_none() && !self.parallel {
            return Ok(None);
        }
        if args.chunk.imports.is_empty() && args.chunk.dynamic_imports.is_empty() {
            return Ok(None);
        }
        let allocator = Allocator::default();
        let parsed = Parser::new(&allocator, &args.code, SourceType::mjs()).parse();
        if !parsed.diagnostics.is_empty() {
            return Err(anyhow::anyhow!(
                "could not parse native chunk for publicPath: {:?}",
                parsed.diagnostics
            ));
        }
        let built = SemanticBuilder::new()
            .with_build_nodes(true)
            .build(&parsed.program);
        let external_paths: BTreeSet<_> = chunk
            .direct_imports_from_external_modules
            .iter()
            .map(|(id, _)| *id)
            .chain(chunk.dynamic_imports_from_external_modules.iter().copied())
            .filter_map(|id| args.module_table[id].as_external())
            .map(|external| external.get_import_path(chunk, None))
            .collect();
        let mut edited = MagicString::new(args.code.as_str());
        let mut changed = false;
        let available = if self.parallel {
            parallel::static_closure(args, args.chunk.filename.as_str())?
        } else {
            BTreeSet::new()
        };
        let mut plans = BTreeMap::<String, Vec<String>>::new();
        for node in built.semantic.nodes().iter() {
            let (literal, dynamic) = match node.kind() {
                AstKind::ImportDeclaration(import) => (Some(&import.source), None),
                AstKind::ExportFromDeclaration(export) => (Some(&export.source), None),
                AstKind::ExportAllDeclaration(export) => (Some(&export.source), None),
                AstKind::ImportExpression(import) => match &import.source {
                    Expression::StringLiteral(literal) => (Some(literal.as_ref()), Some(import)),
                    _ => (None, None),
                },
                _ => (None, None),
            };
            let Some(literal) = literal else { continue };
            let Some(path) = literal.value.as_str() else {
                continue;
            };
            if !path.starts_with('.') {
                continue;
            }
            let Some(target) = relative_target(Path::new(args.chunk.filename.as_str()), path)
            else {
                continue;
            };
            // Only rewrite imports proven to be chunks of this exact invocation.
            if !args.chunks.contains_key(target.as_str()) {
                continue;
            }
            if external_paths.contains(path) {
                return Err(anyhow::anyhow!("an external import collides with emitted chunk {target}; change chunkNames or the external module ID before setting publicPath"));
            }
            if let Some(dynamic) = dynamic.filter(|_| self.parallel) {
                if !plans.contains_key(&target) {
                    self.reserve_plan(target.len())?;
                    let mut dependencies = Vec::new();
                    for dependency in parallel::static_closure(args, &target)? {
                        if dependency != target && !available.contains(dependency) {
                            self.reserve_plan(dependency.len())?;
                            dependencies.push(dependency.to_owned());
                        }
                    }
                    plans.insert(target.clone(), dependencies);
                }
                let dependencies = &plans[&target];
                if !dependencies.is_empty() {
                    let original = dynamic.span.source_text(&args.code);
                    let target_call = if let Some(prefix) = &self.prefix {
                        let before = usize::try_from(literal.span.start - dynamic.span.start)?;
                        let after = usize::try_from(literal.span.end - dynamic.span.start)?;
                        format!(
                            "{}{}{}",
                            &original[..before],
                            ascii_literal(&join(prefix, &target))?,
                            &original[after..]
                        )
                    } else {
                        original.to_owned()
                    };
                    edited
                        .update(
                            dynamic.span.start,
                            dynamic.span.end,
                            parallel::expression(
                                args.chunk.filename.as_str(),
                                &target_call,
                                dependencies,
                                self.prefix.as_deref(),
                            )?,
                        )
                        .map_err(anyhow::Error::msg)?;
                    changed = true;
                    continue;
                }
            }
            let Some(prefix) = &self.prefix else { continue };
            edited
                .update(
                    literal.span.start,
                    literal.span.end,
                    ascii_literal(&join(prefix, &target))?,
                )
                .map_err(anyhow::Error::msg)?;
            changed = true;
        }
        self.imports
            .lock()
            .map_err(|error| anyhow::anyhow!("{error}"))?
            .get_mut(args.chunk.filename.as_str())
            .ok_or_else(|| anyhow::anyhow!("parallel imports lost their native output record"))?
            .parallel = plans;
        if !changed {
            return Ok(None);
        }
        let map = if args.options.sourcemap.is_some() {
            HookTransformOutputMap::from(edited.source_map(SourceMapOptions {
                source: args.chunk.filename.as_str().into(),
                include_content: true,
                ..Default::default()
            }))
        } else {
            HookTransformOutputMap::Null
        };
        Ok(Some(HookRenderChunkOutput {
            code: edited.to_string(),
            map,
        }))
    }
}

fn join(prefix: &str, path: &str) -> String {
    format!("{}/{path}", prefix.trim_end_matches('/'))
}

fn relative_target(importer: &Path, import: &str) -> Option<String> {
    let path = importer.parent()?.join(import);
    let mut normalized = PathBuf::new();
    for part in path.components() {
        match part {
            Component::Normal(value) => normalized.push(value),
            Component::CurDir => {}
            Component::ParentDir if normalized.pop() => {}
            _ => return None,
        }
    }
    Some(resources::portable(&normalized))
}

pub(super) fn validate_prefix(prefix: &str) -> Result<(), ClientBuildError> {
    if prefix.is_empty()
        || prefix.contains(['\\', '\0', '?', '#'])
        || prefix
            .bytes()
            .any(|byte| byte.is_ascii_control() || byte == b' ')
    {
        return Err(ClientBuildError::Invalid(
            "publicPath must be a nonempty URL prefix without whitespace, query or fragment".into(),
        ));
    }
    Ok(())
}

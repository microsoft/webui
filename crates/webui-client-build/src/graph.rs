// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

use std::borrow::Cow;
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard};

use rolldown::plugin::{
    HookLoadArgs, HookLoadReturn, HookResolveIdArgs, HookResolveIdOutput, HookResolveIdReturn,
    HookTransformArgs, HookTransformReturn, HookUsage, Plugin, PluginContext,
    PluginContextResolveOptions, SharedLoadPluginContext, SharedTransformPluginContext,
};
use rolldown_common::{ModuleId, ResolvedId};

use crate::{
    loaders, manifest, resolver_path, resources, ClientBuildError, ClientBuildOptions,
    InputMetadata,
};

pub use webui_projection::{Import, Module, ModuleGraph, ScriptSyntax};

pub(super) const MODULE_LIMIT: usize = 16 * 1024 * 1024;
const GRAPH_LIMIT: usize = 64 * 1024 * 1024;
const MODULES_LIMIT: usize = 65_535;

#[derive(Debug, Default)]
pub(super) struct Evidence {
    pub graph: ModuleGraph,
    pub inputs: BTreeMap<String, InputMetadata>,
    pub hashes: BTreeMap<String, String>,
    pub dependencies: BTreeSet<PathBuf>,
}

#[derive(Debug, Default)]
struct State {
    evidence: Evidence,
    loaded_bytes: usize,
    external_ids: BTreeSet<String>,
}

#[derive(Clone, Debug)]
pub(super) struct Capture {
    project: Arc<Path>,
    framework: Option<Arc<Path>>,
    options: Arc<ClientBuildOptions>,
    state: Arc<Mutex<State>>,
}

impl Capture {
    fn is_external(&self, specifier: &str) -> bool {
        let aliased = self
            .options
            .aliases
            .keys()
            .any(|name| alias_matches(name, specifier))
            || (self.framework.is_some() && alias_matches("@microsoft/webui-framework", specifier));
        !aliased
            && self.options.external.iter().any(|external| {
                external == specifier
                    || (!external.starts_with(['.', '/', '\\', '\0'])
                        && specifier
                            .strip_prefix(external)
                            .is_some_and(|suffix| suffix.starts_with('/')))
            })
    }

    fn virtual_id<'a>(&'a self, specifier: &'a str) -> Option<&'a str> {
        if self.options.virtual_modules.contains_key(specifier) {
            return Some(specifier);
        }
        self.options.aliases.iter().find_map(|(name, target)| {
            (name.trim_end_matches('$') == specifier
                && self.options.virtual_modules.contains_key(target))
            .then_some(target.as_str())
        })
    }

    pub(super) fn new(
        project: PathBuf,
        framework: Option<PathBuf>,
        options: ClientBuildOptions,
    ) -> Self {
        Self {
            project: Arc::from(project),
            framework: framework.map(Arc::from),
            options: Arc::new(options),
            state: Arc::default(),
        }
    }

    fn lock(&self) -> Result<MutexGuard<'_, State>, ClientBuildError> {
        self.state
            .lock()
            .map_err(|error| ClientBuildError::Invalid(error.to_string()))
    }

    pub(super) fn finish(&self) -> Result<Evidence, ClientBuildError> {
        Ok(std::mem::take(&mut self.lock()?.evidence))
    }

    fn reserve(&self, bytes: usize) -> Result<(), ClientBuildError> {
        let mut state = self.lock()?;
        if bytes > MODULE_LIMIT || state.loaded_bytes.saturating_add(bytes) > GRAPH_LIMIT {
            return Err(limit_error());
        }
        state.loaded_bytes += bytes;
        Ok(())
    }

    fn record_resolution(
        &self,
        args: &HookResolveIdArgs<'_>,
        resolved: &ResolvedId,
    ) -> Result<(), ClientBuildError> {
        let external = resolved.external.is_external();
        let raw_id = resolved.id.raw_id();
        let package_metadata = resolved
            .package_json
            .as_ref()
            .map(|package| resources::contained(&self.project, package.realpath()))
            .transpose()?;
        let package_name = if !external && !raw_id.starts_with('\0') {
            let path = resources::contained(&self.project, Path::new(raw_id))?;
            match &resolved.package_json {
                Some(package) => package.name().map(str::to_owned),
                None if self
                    .framework
                    .as_ref()
                    .is_some_and(|root| path.starts_with(root)) =>
                {
                    Some("@microsoft/webui-framework".to_owned())
                }
                None => None,
            }
        } else {
            None
        };
        let mut state = self.lock()?;
        state.evidence.dependencies.extend(package_metadata);
        if external && !state.external_ids.contains(resolved.id.as_str()) {
            if state.external_ids.len() + state.evidence.graph.modules.len() >= MODULES_LIMIT {
                return Err(limit_error());
            }
            state.external_ids.insert(resolved.id.to_string());
        }
        if args.is_entry {
            if external {
                return Err(ClientBuildError::Invalid(
                    "entry points cannot be external".into(),
                ));
            }
            state.evidence.graph.entries.insert(resolved.id.to_string());
        }
        if !external {
            module(&mut state, &resolved.id)?.package_name = package_name.clone();
        }
        if let Some(importer) = args.importer {
            let parent = module(&mut state, importer)?;
            let edge = Import {
                specifier: args.specifier.to_owned(),
                resolved: resolved.id.to_string(),
                dynamic: args.kind.is_dynamic(),
                external,
                package_name,
            };
            parent.imports.push(edge);
        }
        Ok(())
    }

    fn record_input(
        &self,
        args: &HookLoadArgs<'_>,
        bytes: &[u8],
        source: Option<String>,
        syntax: Option<ScriptSyntax>,
    ) -> Result<(), ClientBuildError> {
        let opaque = syntax.is_none();
        let identity = resources::input_key(&self.project, args.raw_id)?;
        let mut state = self.lock()?;
        let input = module(&mut state, args.id)?;
        input.source = source;
        input.opaque = opaque;
        input.syntax = syntax;
        state
            .evidence
            .hashes
            .insert(args.id.to_owned(), manifest::hash(bytes));
        state.evidence.inputs.insert(
            args.id.to_owned(),
            InputMetadata {
                bytes: bytes.len(),
                source: identity,
                opaque,
                syntax,
                imports: Vec::new(),
            },
        );
        Ok(())
    }
}

impl Plugin for Capture {
    fn name(&self) -> Cow<'static, str> {
        Cow::Borrowed("webui-native-build-evidence")
    }

    fn register_hook_usage(&self) -> HookUsage {
        HookUsage::ResolveId | HookUsage::Load | HookUsage::Transform
    }

    async fn resolve_id(
        &self,
        context: &PluginContext,
        args: &HookResolveIdArgs<'_>,
    ) -> HookResolveIdReturn {
        let result = async {
            let resolved = if self.is_external(args.specifier) {
                ResolvedId {
                    id: ModuleId::new(args.specifier),
                    external: true.into(),
                    ..Default::default()
                }
            } else if let Some(id) = self.virtual_id(args.specifier) {
                ResolvedId {
                    id: ModuleId::new(id),
                    attributes: args.attributes.clone(),
                    ..Default::default()
                }
                .finalize_attributes(true)
            } else {
                // Virtual modules resolve relative imports from the explicitly chosen project.
                // No independent resolver is introduced.
                let importer = args.importer.filter(|id| !id.starts_with('\0'));
                let specifier = if Path::new(args.specifier).is_absolute() {
                    Cow::Owned(resolver_path::path(Path::new(args.specifier))?)
                } else {
                    Cow::Borrowed(args.specifier)
                };
                context
                    .resolve(
                        &specifier,
                        importer,
                        Some(PluginContextResolveOptions {
                            import_kind: args.kind,
                            is_entry: args.is_entry,
                            skip_self: true,
                            custom: Arc::clone(&args.custom),
                            attributes: args.attributes.clone(),
                        }),
                    )
                    .await??
            };
            self.record_resolution(args, &resolved)?;
            Ok(Some(HookResolveIdOutput::from_resolved_id(resolved)))
        }
        .await;
        result.map_err(diagnostic)
    }

    async fn load(
        &self,
        context: SharedLoadPluginContext,
        args: &HookLoadArgs<'_>,
    ) -> HookLoadReturn {
        let result: HookLoadReturn = (|| {
            let bytes = if let Some(source) = self.options.virtual_modules.get(args.raw_id) {
                self.reserve(source.len())?;
                source.as_bytes().to_vec()
            } else if args.raw_id.starts_with('\0') {
                return Ok(None);
            } else {
                let path = resources::contained(&self.project, Path::new(args.raw_id))?;
                context.add_watch_file(args.raw_id);
                // Our bounded load replaces Rolldown's disk load, so restore its
                // native governing-tsconfig watch registration through the same
                // resolver/cache rather than guessing a config path.
                if let Some(config) = context
                    .options()
                    .transform_options
                    .discover_tsconfig_file(Path::new(&resolver_path::path(&path)?))
                {
                    context.add_watch_file(resources::utf8(&config)?);
                }
                // Reserve metadata size before reading; correct for growth below.
                let (file, size) = resources::open_bounded(&path, MODULE_LIMIT)?;
                self.reserve(size)?;
                let bytes = resources::read_bounded(file, &path, MODULE_LIMIT)?;
                if bytes.len() > size {
                    self.reserve(bytes.len() - size)?;
                }
                bytes
            };
            let loader = loaders::kind(args, &self.options.loaders)?;
            let syntax = loaders::syntax(&loader);
            let source = if matches!(loader, loaders::Kind::File | loaders::Kind::DataUrl) {
                None
            } else {
                Some(
                    std::str::from_utf8(&bytes)
                        .map_err(|error| {
                            ClientBuildError::Invalid(format!(
                                "{} must be UTF-8: {error}",
                                args.raw_id
                            ))
                        })?
                        .to_owned(),
                )
            };
            self.record_input(args, &bytes, source, syntax)?;
            loaders::load(&context, args, bytes, loader)
        })();
        result.map_err(diagnostic)
    }

    async fn transform(
        &self,
        _context: SharedTransformPluginContext,
        args: &HookTransformArgs<'_>,
    ) -> HookTransformReturn {
        let result: HookTransformReturn = (|| {
            let mut state = self.lock()?;
            if !state.evidence.inputs.contains_key(args.id) {
                // Rolldown's own virtual runtime modules have no physical source.
                if !args.raw_id.starts_with('\0') {
                    return Err(ClientBuildError::Invalid(
                        "physical module bypassed bounded capture".into(),
                    )
                    .into());
                }
                if args.code.len() > MODULE_LIMIT
                    || state.loaded_bytes.saturating_add(args.code.len()) > GRAPH_LIMIT
                {
                    return Err(limit_error().into());
                }
                state.loaded_bytes += args.code.len();
                let module = module(&mut state, args.id)?;
                module.source = Some(args.code.to_string());
                module.syntax = Some(ScriptSyntax::Js);
                state
                    .evidence
                    .hashes
                    .insert(args.id.to_owned(), manifest::hash(args.code.as_bytes()));
                state.evidence.inputs.insert(
                    args.id.to_owned(),
                    InputMetadata {
                        bytes: args.code.len(),
                        source: resources::input_key(&self.project, args.raw_id)?,
                        opaque: false,
                        syntax: Some(ScriptSyntax::Js),
                        imports: Vec::new(),
                    },
                );
            }
            Ok(None)
        })();
        result.map_err(diagnostic)
    }
}

fn alias_matches(alias: &str, specifier: &str) -> bool {
    if let Some(exact) = alias.strip_suffix('$') {
        exact == specifier
    } else {
        alias == specifier
            || specifier
                .strip_prefix(alias)
                .is_some_and(|suffix| suffix.starts_with('/'))
    }
}

fn module<'a>(state: &'a mut State, id: &str) -> Result<&'a mut Module, ClientBuildError> {
    if state.evidence.graph.modules.len() + state.external_ids.len() >= MODULES_LIMIT
        && !state.evidence.graph.modules.contains_key(id)
    {
        return Err(limit_error());
    }
    Ok(state
        .evidence
        .graph
        .modules
        .entry(id.to_owned())
        .or_default())
}

#[cold]
fn limit_error() -> ClientBuildError {
    ClientBuildError::Invalid("browser source graph exceeds supported build limits (16 MiB/module, 64 MiB total, 65,535 modules); reduce the input graph".into())
}

#[cold]
#[inline(never)]
fn diagnostic(error: anyhow::Error) -> anyhow::Error {
    if error.is::<rolldown_error::BuildDiagnostic>() {
        error
    } else {
        rolldown_error::BuildDiagnostic::unhandleable_error(error).into()
    }
}

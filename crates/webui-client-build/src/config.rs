// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use rolldown::{BundlerOptions, BundlerTransformOptions, InputItem, ResolveOptions};
use rolldown_common::{
    CodeSplittingMode, ExperimentalOptions, OutputCharset, OutputFormat, Platform,
    PreserveEntrySignatures, SourceMapType, TsConfig,
};

use crate::{
    graph, resolver_path, resources, urls, ClientBuildError, ClientBuildOptions, Format, Loader,
    SourceMap,
};

pub(super) struct Prepared {
    pub project: PathBuf,
    pub out: PathBuf,
    pub framework: Option<PathBuf>,
    pub bundler_options: BundlerOptions,
    pub dependencies: BTreeSet<PathBuf>,
}

pub(super) fn prepare(options: &ClientBuildOptions) -> Result<Prepared, ClientBuildError> {
    validate(options)?;
    let project =
        std::fs::canonicalize(&options.project).map_err(|source| ClientBuildError::Read {
            path: options.project.clone(),
            source,
        })?;
    let out = resources::output_path(&project, &options.out_dir)?;
    let framework = options
        .framework
        .as_ref()
        .map(|path| resources::contained(&project, path))
        .transpose()?;
    let aliases = aliases(options, &project, framework.as_deref())?;
    let mut defines: BTreeMap<_, _> = options.defines.clone();
    defines
        .entry("__WEBUI_DEV__".into())
        .or_insert_with(|| options.development.to_string());
    let tsconfig = options
        .tsconfig
        .as_ref()
        .map(|path| resources::contained(&project, path))
        .transpose()?;
    let mut dependencies = BTreeSet::new();
    dependencies.extend(tsconfig.iter().cloned());
    if let Some(framework) = &framework {
        dependencies.insert(framework.join("package.json"));
    }
    let bundler_options = BundlerOptions {
        cwd: Some(resolver_path::path(&project)?.into()),
        input: Some(entries(options, &project)?),
        dir: Some(resolver_path::path(&out)?),
        platform: Some(Platform::Browser),
        format: Some(match options.format {
            Format::Esm => OutputFormat::Esm,
            Format::Iife => OutputFormat::Iife,
        }),
        code_splitting: Some(CodeSplittingMode::Bool(options.format == Format::Esm)),
        preserve_entry_signatures: (options.format == Format::Iife)
            .then_some(PreserveEntrySignatures::False),
        charset: Some(OutputCharset::Ascii),
        legal_comments: Some(rolldown::LegalComments::Inline),
        entry_filenames: Some(options.entry_names.clone().into()),
        chunk_filenames: Some(options.chunk_names.clone().into()),
        transform: Some(BundlerTransformOptions {
            target: Some(either::Either::Left(options.target.clone())),
            ..Default::default()
        }),
        tsconfig: tsconfig
            .as_deref()
            .map(resolver_path::path)
            .transpose()?
            .map(|path| TsConfig::Manual(path.into())),
        sourcemap: match options.sourcemap {
            SourceMap::None => None,
            SourceMap::File => Some(SourceMapType::File),
            SourceMap::Inline => Some(SourceMapType::Inline),
            SourceMap::Hidden => Some(SourceMapType::Hidden),
        },
        minify: Some(options.minify.into()),
        define: Some(defines.into_iter().collect()),
        resolve: Some(ResolveOptions {
            alias: Some(aliases),
            ..Default::default()
        }),
        experimental: Some(ExperimentalOptions {
            import_attributes: Some(true),
            ..Default::default()
        }),
        ..Default::default()
    };
    Ok(Prepared {
        project,
        out,
        framework,
        bundler_options,
        dependencies,
    })
}

fn aliases(
    options: &ClientBuildOptions,
    project: &Path,
    framework: Option<&Path>,
) -> Result<resources::FrameworkAliases, ClientBuildError> {
    let mut aliases = framework
        .map(resources::framework_exports)
        .transpose()?
        .unwrap_or_default();
    for (name, target) in &options.aliases {
        if aliases
            .iter()
            .any(|(key, _)| key.trim_end_matches('$') == name.trim_end_matches('$'))
        {
            return Err(ClientBuildError::Invalid(
                "aliases cannot override the pinned framework export map".into(),
            ));
        }
        let target = if target.starts_with('.') {
            resolver_path::path(&project.join(target))?
        } else if Path::new(target).is_absolute() {
            resolver_path::path(Path::new(target))?
        } else {
            target.clone()
        };
        aliases.push((name.clone(), vec![Some(target)]));
    }
    Ok(aliases)
}

fn entries(
    options: &ClientBuildOptions,
    project: &Path,
) -> Result<Vec<InputItem>, ClientBuildError> {
    options
        .entries
        .iter()
        .map(|(name, entry)| {
            let id = resources::utf8(entry)?;
            let import = if id.starts_with('\0') {
                if !options.virtual_modules.contains_key(id) {
                    return Err(ClientBuildError::Invalid(format!(
                        "entry {name} has no matching virtual module"
                    )));
                }
                id.to_owned()
            } else {
                resolver_path::path(&resources::contained(project, entry)?)?
            };
            Ok(InputItem {
                name: Some(name.clone()),
                import,
            })
        })
        .collect()
}

fn validate(options: &ClientBuildOptions) -> Result<(), ClientBuildError> {
    if options.project.as_os_str().is_empty()
        || options.entries.is_empty()
        || options.entries.len() > 65_535
        || options.virtual_modules.len() > 65_535
    {
        return Err(ClientBuildError::Invalid(
            "provide a project boundary and 1–65,535 named entries, with at most 65,535 virtual modules".into(),
        ));
    }
    resources::output_path(Path::new(""), &options.out_dir)?;
    for name in options
        .entries
        .keys()
        .chain([&options.entry_names, &options.chunk_names])
    {
        resources::output_path(Path::new(""), Path::new(name))?;
        if name.contains(['\\', '\0', ':']) {
            return Err(ClientBuildError::Invalid(
                "entry names and output templates must use portable relative paths".into(),
            ));
        }
    }
    if let Some(prefix) = &options.public_path {
        urls::validate_prefix(prefix)?;
    }
    for (extension, loader) in &options.loaders {
        if !extension.starts_with('.')
            || extension.len() < 2
            || extension.contains(['/', '\\', '\0'])
        {
            return Err(ClientBuildError::Invalid(
                "loader extensions must start with '.' and contain no path separators".into(),
            ));
        }
        if *loader == Loader::Css {
            return Err(ClientBuildError::Invalid(
                "CSS bundling is unsupported; use a text loader or CSS import attributes".into(),
            ));
        }
    }
    let mut size = 0_usize;
    for (id, source) in &options.virtual_modules {
        size = size.saturating_add(source.len());
        if !id.starts_with('\0')
            || id.len() < 2
            || id[1..].contains('\0')
            || source.len() > graph::MODULE_LIMIT
            || size > 64 * 1024 * 1024
        {
            return Err(ClientBuildError::Invalid("virtual modules require exact NUL-prefixed IDs and bounded sources (16 MiB/module, 64 MiB total)".into()));
        }
    }
    Ok(())
}

// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

use std::borrow::Cow;
use std::path::{Path, PathBuf};

use oxc::ast::AstKind;
use oxc::semantic::SemanticBuilder;
use oxc::transformer::Transformer;
use rolldown::plugin::{
    HookTransformAstArgs, HookTransformAstReturn, HookUsage, Plugin, PluginContext,
};
use rolldown_common::ModuleType;

/// Rolldown deliberately excludes node_modules and virtual IDs from tsconfig
/// discovery, even in manual mode. Apply the same native transform in place to
/// decorated authored sources there, retaining original spans and source maps.
#[derive(Debug)]
pub(super) struct LegacyDecorators {
    pub project: PathBuf,
}

impl Plugin for LegacyDecorators {
    fn name(&self) -> Cow<'static, str> {
        Cow::Borrowed("webui-native-legacy-decorators")
    }

    fn register_hook_usage(&self) -> HookUsage {
        HookUsage::TransformAst
    }

    async fn transform_ast(
        &self,
        context: &PluginContext,
        args: HookTransformAstArgs<'_>,
    ) -> HookTransformAstReturn {
        if !matches!(
            args.module_type,
            ModuleType::Js | ModuleType::Jsx | ModuleType::Ts | ModuleType::Tsx
        ) || !args.ast.source().contains('@')
        {
            return Ok(args.ast);
        }
        let mut warnings = Vec::new();
        let native_options = context
            .options()
            .transform_options
            .options_for_file(Some(Path::new(args.id)), &mut warnings)?;
        reject_warnings(&warnings)?;
        // Preserve Rolldown's ordinary one-pass path wherever it already lowers
        // legacy decorators. Plain JS is otherwise skipped at ES2022.
        if native_options.decorator.legacy && *args.module_type != ModuleType::Js {
            return Ok(args.ast);
        }
        let mut ast = args.ast;
        ast.program.with_mut(|fields| -> anyhow::Result<()> {
            let built = SemanticBuilder::new().with_build_nodes(true)
                .with_check_syntax_error(true).with_enum_eval(true).build(fields.program);
            if !built.semantic.nodes().iter().any(|node| matches!(node.kind(), AstKind::Decorator(_))) {
                return Ok(());
            }
            if !built.diagnostics.is_empty() {
                return Err(anyhow::anyhow!("invalid decorated browser source {}: {:?}", args.id, built.diagnostics));
            }
            let scoped_path = if args.id.starts_with('\0')
                || Path::new(args.id).components().any(|part| part.as_os_str() == "node_modules")
            {
                // A resolver scope, not a physical input or a generated file.
                self.project.join("__webui_decorator_scope__.ts")
            } else {
                PathBuf::from(args.id)
            };
            let resolver = context.resolver().clone_default_resolver();
            let tsconfig = resolver.find_tsconfig(&scoped_path)?;
            if tsconfig.as_ref().is_some_and(|config| config.compiler_options.experimental_decorators == Some(false)) {
                return Err(anyhow::anyhow!("{} uses decorators but its tsconfig disables legacy decorators; set experimentalDecorators: true for ES2022 browser output", args.id));
            }
            let fallback = context.options().transform_options.options_for_file(Some(&scoped_path), &mut warnings)?;
            reject_warnings(&warnings)?;
            let mut options = (*fallback).clone();
            options.decorator.legacy = true;
            if tsconfig.is_none() {
                options.assumptions.set_public_class_fields = true;
                options.typescript.remove_class_fields_without_initializer = true;
            }
            let scoping = built.semantic.into_scoping();
            let result = Transformer::new(fields.allocator, Path::new(args.id), &options)
                .build_with_scoping(scoping, fields.program);
            if !result.diagnostics.is_empty() {
                return Err(anyhow::anyhow!("could not lower legacy decorators in {}: {:?}", args.id, result.diagnostics));
            }
            Ok(())
        })?;
        Ok(ast)
    }
}

fn reject_warnings(warnings: &[rolldown_error::BuildDiagnostic]) -> anyhow::Result<()> {
    if warnings.is_empty() {
        return Ok(());
    }
    Err(anyhow::anyhow!(
        "{}",
        warnings
            .iter()
            .map(|warning| warning.to_diagnostic().convert_to_string(false))
            .collect::<Vec<_>>()
            .join("\n")
    ))
}

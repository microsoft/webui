// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

mod classes;
use classes::{class_binding, function_binding};

use std::collections::HashMap;

use oxc::allocator::Allocator;
use oxc::ast::{ast::*, AstKind};
use oxc::parser::Parser;
use oxc::semantic::{Semantic, SemanticBuilder};
use oxc::span::SourceType;
use oxc::syntax::{node::NodeId, symbol::SymbolId};

use crate::{diagnostic, ProjectionDiagnostic};

#[derive(Debug)]
pub(super) enum Value {
    Symbol(SymbolId),
    Global,
    Member(NodeId, String),
    Call {
        callee: NodeId,
        arguments: Vec<NodeId>,
        valid: bool,
    },
    Class,
    Alias(NodeId),
    Other,
}

#[derive(Debug)]
pub(super) enum Binding {
    Import {
        specifier: String,
        name: Option<String>,
    },
    Value(NodeId),
    Factory(Factory),
    Mutable,
    Other,
}

#[derive(Debug)]
pub(super) enum Export {
    Symbol(SymbolId),
    Value(NodeId),
    Forward {
        specifier: String,
        name: Option<String>,
    },
}

#[derive(Debug)]
pub(super) struct Factory {
    pub parameters: Vec<SymbolId>,
    pub class: NodeId,
}

#[derive(Debug)]
pub(super) struct Property {
    pub name: Option<String>,
    pub decorators: Vec<NodeId>,
}

#[derive(Debug)]
pub(super) struct Class {
    pub base: Option<NodeId>,
    pub properties: Vec<Property>,
    pub valid_factory: bool,
}

#[derive(Clone, Debug)]
pub(super) struct Definition {
    pub class: NodeId,
    pub tag: Option<String>,
    pub arguments_valid: bool,
}

#[derive(Debug, Default)]
pub(super) struct Analysis {
    pub bindings: HashMap<SymbolId, Binding>,
    pub exports: HashMap<String, Export>,
    pub stars: Vec<String>,
    pub values: HashMap<NodeId, Value>,
    pub classes: HashMap<NodeId, Class>,
    pub definitions: Vec<Definition>,
}

pub(super) fn supported(module: &str) -> bool {
    matches!(
        std::path::Path::new(module)
            .extension()
            .and_then(|extension| extension.to_str()),
        Some("ts" | "tsx" | "mts" | "cts" | "js" | "jsx" | "mjs" | "cjs")
    )
}

pub(super) fn parse(
    module: &str,
    source: &str,
    syntax: Option<crate::ScriptSyntax>,
) -> Result<Analysis, ProjectionDiagnostic> {
    let allocator = Allocator::default();
    let inferred = SourceType::from_path(module).unwrap_or_default();
    let source_type = syntax.map_or(inferred, |syntax| {
        inferred
            .with_typescript(matches!(
                syntax,
                crate::ScriptSyntax::Ts | crate::ScriptSyntax::Tsx
            ))
            .with_javascript(matches!(
                syntax,
                crate::ScriptSyntax::Js | crate::ScriptSyntax::Jsx
            ))
            .with_jsx(matches!(
                syntax,
                crate::ScriptSyntax::Jsx | crate::ScriptSyntax::Tsx
            ))
    });
    let parsed = Parser::new(&allocator, source, source_type).parse();
    if !parsed.diagnostics.is_empty() {
        return Err(diagnostic(
            module,
            "PROJ-C001",
            format!(
                "Fix invalid TypeScript/JavaScript syntax: {:?}",
                parsed.diagnostics
            ),
        ));
    }
    let built = SemanticBuilder::new()
        .with_build_nodes(true)
        .build(&parsed.program);
    if !built.diagnostics.is_empty() {
        return Err(diagnostic(
            module,
            "PROJ-C001",
            format!(
                "Fix invalid TypeScript/JavaScript bindings: {:?}",
                built.diagnostics
            ),
        ));
    }
    let semantic = built.semantic;
    let mut result = Analysis::default();
    for node in semantic.nodes().iter() {
        let value = value(node.kind(), &semantic);
        if !matches!(value, Value::Other) {
            result.values.insert(node.id(), value);
        }
        match node.kind() {
            AstKind::ImportDeclaration(import) => {
                imports(import, &mut result).ok_or_else(|| {
                    diagnostic(
                        module,
                        "PROJ-C001",
                        "Module specifiers must be valid UTF-8 strings.",
                    )
                })?
            }
            AstKind::VariableDeclarator(variable) => {
                variable_binding(variable, &semantic, &mut result)
            }
            AstKind::Class(class) => class_binding(class, &semantic, &mut result),
            AstKind::Function(function) => function_binding(function, &semantic, &mut result),
            AstKind::CallExpression(call) => definition(call, &semantic, &mut result),
            AstKind::ExportDeclaration(export) => {
                declaration_exports(&export.declaration, &mut result)
            }
            AstKind::ExportNamedDeclaration(export) => {
                for specifier in &export.specifiers {
                    if let ModuleExportName::IdentifierReference(local) = &specifier.local {
                        if let Some(symbol) = reference_symbol(local, &semantic) {
                            result.exports.insert(
                                specifier.exported.name().to_string(),
                                Export::Symbol(symbol),
                            );
                        }
                    }
                }
            }
            AstKind::ExportFromDeclaration(export) => {
                for specifier in &export.specifiers {
                    result.exports.insert(
                        specifier.exported.name().to_string(),
                        Export::Forward {
                            specifier: export
                                .source
                                .value
                                .as_str()
                                .ok_or_else(|| {
                                    diagnostic(
                                        module,
                                        "PROJ-C001",
                                        "Module specifiers must be valid UTF-8 strings.",
                                    )
                                })?
                                .to_owned(),
                            name: Some(specifier.local.name().to_string()),
                        },
                    );
                }
            }
            AstKind::ExportAllDeclaration(export) => {
                let specifier = export
                    .source
                    .value
                    .as_str()
                    .ok_or_else(|| {
                        diagnostic(
                            module,
                            "PROJ-C001",
                            "Module specifiers must be valid UTF-8 strings.",
                        )
                    })?
                    .to_owned();
                if let Some(name) = &export.exported {
                    result.exports.insert(
                        name.name().to_string(),
                        Export::Forward {
                            specifier,
                            name: None,
                        },
                    );
                } else {
                    result.stars.push(specifier);
                }
            }
            AstKind::ExportDefaultDeclaration(export) => {
                let id = match &export.declaration {
                    ExportDefaultDeclarationKind::ClassDeclaration(class) => {
                        Some(class.node_id.get())
                    }
                    ExportDefaultDeclarationKind::FunctionDeclaration(function) => {
                        Some(function.node_id.get())
                    }
                    other => other.as_expression().map(Expression::node_id),
                };
                if let Some(id) = id {
                    result.exports.insert("default".into(), Export::Value(id));
                }
            }
            _ => {}
        }
    }
    Ok(result)
}

fn reference_symbol(
    reference: &IdentifierReference<'_>,
    semantic: &Semantic<'_>,
) -> Option<SymbolId> {
    reference
        .reference_id
        .get()
        .and_then(|id| semantic.scoping().get_reference(id).symbol_id())
}

fn value(kind: AstKind<'_>, semantic: &Semantic<'_>) -> Value {
    match kind {
        AstKind::IdentifierReference(reference) => {
            reference_symbol(reference, semantic).map_or(Value::Global, Value::Symbol)
        }
        AstKind::StaticMemberExpression(member) => {
            Value::Member(member.object.node_id(), member.property.name.to_string())
        }
        AstKind::CallExpression(call) => Value::Call {
            callee: call.callee.node_id(),
            arguments: call.arguments.iter().map(Argument::node_id).collect(),
            valid: !call.optional
                && !call
                    .arguments
                    .iter()
                    .any(|arg| matches!(arg, Argument::SpreadElement(_))),
        },
        AstKind::Class(_) => Value::Class,
        AstKind::ParenthesizedExpression(node) => Value::Alias(node.expression.node_id()),
        AstKind::TSAsExpression(node) => Value::Alias(node.expression.node_id()),
        AstKind::TSSatisfiesExpression(node) => Value::Alias(node.expression.node_id()),
        AstKind::TSNonNullExpression(node) => Value::Alias(node.expression.node_id()),
        AstKind::TSInstantiationExpression(node) => Value::Alias(node.expression.node_id()),
        AstKind::TSTypeAssertion(node) => Value::Alias(node.expression.node_id()),
        _ => Value::Other,
    }
}

fn imports(import: &ImportDeclaration<'_>, result: &mut Analysis) -> Option<()> {
    let source = import.source.value.as_str()?;
    for specifier in import.specifiers.iter().flatten() {
        let (local, name) = match specifier {
            ImportDeclarationSpecifier::ImportSpecifier(specifier) => (
                &specifier.local,
                Some(specifier.imported.name().to_string()),
            ),
            ImportDeclarationSpecifier::ImportDefaultSpecifier(specifier) => {
                (&specifier.local, Some("default".into()))
            }
            ImportDeclarationSpecifier::ImportNamespaceSpecifier(specifier) => {
                (&specifier.local, None)
            }
        };
        if let Some(symbol) = local.symbol_id.get() {
            result.bindings.insert(
                symbol,
                Binding::Import {
                    specifier: source.to_owned(),
                    name,
                },
            );
        }
    }
    Some(())
}

fn variable_binding(
    variable: &VariableDeclarator<'_>,
    semantic: &Semantic<'_>,
    result: &mut Analysis,
) {
    let Some(symbol) = variable
        .id
        .get_binding_identifier()
        .and_then(|id| id.symbol_id.get())
    else {
        return;
    };
    let binding = if semantic.scoping().symbol_is_mutated(symbol)
        || !semantic.scoping().symbol_flags(symbol).is_const_variable()
    {
        Binding::Mutable
    } else {
        variable
            .init
            .as_ref()
            .map_or(Binding::Other, |expr| Binding::Value(expr.node_id()))
    };
    result.bindings.insert(symbol, binding);
}

fn definition(call: &CallExpression<'_>, semantic: &Semantic<'_>, result: &mut Analysis) {
    let Expression::StaticMemberExpression(member) = &call.callee else {
        return;
    };
    if member.property.name != "define" {
        return;
    }
    let custom = match &member.object {
        Expression::Identifier(id) if id.name == "customElements" => {
            if reference_symbol(id, semantic).is_some() {
                return;
            }
            true
        }
        _ => false,
    };
    let class = if custom {
        let Some(argument) = call.arguments.get(1) else {
            return;
        };
        argument.node_id()
    } else {
        member.object.node_id()
    };
    let tag = match call.arguments.first() {
        Some(Argument::StringLiteral(tag)) => tag.value.as_str().map(str::to_owned),
        _ => None,
    };
    result.definitions.push(Definition {
        class,
        tag,
        arguments_valid: call.arguments.len() == if custom { 2 } else { 1 },
    });
}

fn declaration_exports(declaration: &Declaration<'_>, result: &mut Analysis) {
    let mut identifiers = Vec::new();
    match declaration {
        Declaration::ClassDeclaration(class) => identifiers.extend(class.id.as_ref()),
        Declaration::FunctionDeclaration(function) => identifiers.extend(function.id.as_ref()),
        Declaration::VariableDeclaration(variable) => {
            identifiers.extend(
                variable
                    .declarations
                    .iter()
                    .filter_map(|decl| decl.id.get_binding_identifier()),
            );
        }
        _ => {}
    }
    for identifier in identifiers {
        if let Some(symbol) = identifier.symbol_id.get() {
            result
                .exports
                .insert(identifier.name.to_string(), Export::Symbol(symbol));
        }
    }
}

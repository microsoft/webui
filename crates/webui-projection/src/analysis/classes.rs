// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

use super::*;

pub(super) fn class_binding(
    class: &oxc::ast::ast::Class<'_>,
    semantic: &Semantic<'_>,
    result: &mut Analysis,
) {
    let mut properties = Vec::new();
    let mut method_decorators = Vec::new();
    let mut valid_factory = class.decorators.is_empty();
    let mut source_identity_preserved = valid_factory;
    for element in &class.body.body {
        match element {
            ClassElement::PropertyDefinition(property) => {
                valid_factory &= !property.r#static;
                if property.r#static {
                    source_identity_preserved &=
                        ordinary_static_member(&property.key) && property.decorators.is_empty();
                }
                if property.decorators.is_empty() {
                    continue;
                }
                let name = match &property.key {
                    PropertyKey::StaticIdentifier(name) if !property.computed => {
                        Some(name.name.to_string())
                    }
                    _ => None,
                };
                properties.push(Property {
                    name,
                    decorators: property
                        .decorators
                        .iter()
                        .map(|decorator| decorator.expression.node_id())
                        .collect(),
                });
            }
            ClassElement::MethodDefinition(method) => {
                valid_factory &= !method.r#static;
                source_identity_preserved &=
                    !method.r#static || ordinary_static_member(&method.key);
                source_identity_preserved &= method
                    .value
                    .params
                    .items
                    .iter()
                    .all(|param| param.decorators.is_empty());
                if method.kind == MethodDefinitionKind::Method
                    && !method.r#static
                    && !method.computed
                {
                    method_decorators.extend(
                        method
                            .decorators
                            .iter()
                            .map(|decorator| decorator.expression.node_id()),
                    );
                } else {
                    source_identity_preserved &= method.decorators.is_empty();
                }
            }
            ClassElement::AccessorProperty(accessor) => {
                source_identity_preserved &= accessor.decorators.is_empty()
                    && (!accessor.r#static || ordinary_static_member(&accessor.key));
            }
            ClassElement::StaticBlock(_) => {
                valid_factory = false;
                source_identity_preserved = false;
            }
            _ => {}
        }
    }
    let id = class.node_id.get();
    result.classes.insert(
        id,
        Class {
            base: class
                .heritage
                .as_ref()
                .map(|heritage| heritage.expression.node_id()),
            properties,
            method_decorators,
            valid_factory,
            source_identity_preserved,
        },
    );
    if let Some(symbol) = class.id.as_ref().and_then(|id| id.symbol_id.get()) {
        let binding = if semantic.scoping().symbol_is_mutated(symbol) {
            Binding::Mutable
        } else {
            Binding::Value(id)
        };
        result.bindings.insert(symbol, binding);
    }
}

fn ordinary_static_member(key: &PropertyKey<'_>) -> bool {
    matches!(key, PropertyKey::StaticIdentifier(name) if name.name != "define")
}

pub(super) fn function_binding(
    function: &Function<'_>,
    semantic: &Semantic<'_>,
    result: &mut Analysis,
) {
    let Some(symbol) = function.id.as_ref().and_then(|id| id.symbol_id.get()) else {
        return;
    };
    let id = function.node_id.get();
    let binding = if semantic.scoping().symbol_is_mutated(symbol) {
        Binding::Mutable
    } else if matches!(result.values.get(&id), Some(Value::NeutralDecorator(_))) {
        Binding::Value(id)
    } else {
        factory(function).map_or(Binding::Other, Binding::Factory)
    };
    result.bindings.insert(symbol, binding);
}

fn factory(function: &Function<'_>) -> Option<Factory> {
    if function.r#async || function.generator || function.params.rest.is_some() {
        return None;
    }
    let mut parameters = Vec::with_capacity(function.params.items.len());
    for parameter in &function.params.items {
        if parameter.initializer.is_some() {
            return None;
        }
        parameters.push(
            parameter
                .pattern
                .get_binding_identifier()?
                .symbol_id
                .get()?,
        );
    }
    let statements = &function.body.as_ref()?.statements;
    let class = match statements.as_slice() {
        [Statement::ReturnStatement(returned)] => match returned.argument.as_ref()? {
            Expression::ClassExpression(class) => class.node_id.get(),
            _ => return None,
        },
        [Statement::ClassDeclaration(class), Statement::ReturnStatement(returned)] => {
            let Expression::Identifier(reference) = returned.argument.as_ref()? else {
                return None;
            };
            if class.id.as_ref()?.name != reference.name {
                return None;
            }
            class.node_id.get()
        }
        _ => return None,
    };
    Some(Factory { parameters, class })
}

// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

use super::*;

pub(super) fn class_binding(
    class: &oxc::ast::ast::Class<'_>,
    semantic: &Semantic<'_>,
    result: &mut Analysis,
) {
    let mut properties = Vec::new();
    let mut valid_factory = class.decorators.is_empty();
    for element in &class.body.body {
        match element {
            ClassElement::PropertyDefinition(property) => {
                valid_factory &= !property.r#static;
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
            }
            ClassElement::StaticBlock(_) => valid_factory = false,
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
            valid_factory,
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

pub(super) fn function_binding(
    function: &Function<'_>,
    semantic: &Semantic<'_>,
    result: &mut Analysis,
) {
    let Some(symbol) = function.id.as_ref().and_then(|id| id.symbol_id.get()) else {
        return;
    };
    let binding = if semantic.scoping().symbol_is_mutated(symbol) {
        Binding::Mutable
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

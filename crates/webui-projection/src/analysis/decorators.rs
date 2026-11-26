// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

use super::*;
use oxc::syntax::scope::ScopeId;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct Effect {
    pub(crate) factory: bool,
    pub(crate) method_only: bool,
}

pub(super) struct Proof<'s, 'a> {
    semantic: &'s Semantic<'a>,
    private_weak_maps: HashMap<SymbolId, bool>,
}

impl<'s, 'a> Proof<'s, 'a> {
    pub(super) fn new(semantic: &'s Semantic<'a>) -> Self {
        Self {
            semantic,
            private_weak_maps: HashMap::new(),
        }
    }

    pub(super) fn classify(&mut self, node: AstKind<'a>) -> Option<Effect> {
        let body = match node {
            AstKind::Function(function) => Body::function(function)?,
            AstKind::ArrowFunctionExpression(arrow) => Body::arrow(arrow)?,
            _ => return None,
        };
        if let Some(decorator) = body
            .returned_closure()
            .filter(|_| body.plain_parameters())
            .and_then(Body::expression)
        {
            self.member_effect(decorator, body.scope)
                .map(|method_only| Effect {
                    factory: true,
                    method_only,
                })
        } else {
            self.member_effect(body, None).map(|method_only| Effect {
                factory: false,
                method_only,
            })
        }
    }

    fn member_effect(&mut self, body: Body<'a>, captures: Option<ScopeId>) -> Option<bool> {
        let symbols = body.member_parameters()?;
        let mut method_only = false;
        match body.code {
            Code::Expression(expression) => {
                method_only = self.reads(expression, symbols[2]);
                if !method_only {
                    return None;
                }
            }
            Code::Statements(statements) => {
                for statement in statements {
                    match statement {
                        Statement::EmptyStatement(_) => {}
                        Statement::ReturnStatement(returned) => {
                            if let Some(expression) = &returned.argument {
                                if !self.reads(expression, symbols[2]) {
                                    return None;
                                }
                                method_only = true;
                            }
                        }
                        Statement::ExpressionStatement(statement)
                            if self.record_member(&statement.expression, symbols, captures) => {}
                        _ => return None,
                    }
                }
            }
        }
        Some(method_only)
    }

    fn reads(&self, expression: &Expression<'_>, symbol: Option<SymbolId>) -> bool {
        symbol.is_some_and(|symbol| {
            matches!(expression.get_inner_expression(), Expression::Identifier(reference)
                if reference_symbol(reference, self.semantic) == Some(symbol))
        })
    }

    fn record_member(
        &mut self,
        expression: &Expression<'a>,
        symbols: [Option<SymbolId>; 3],
        captures: Option<ScopeId>,
    ) -> bool {
        let Expression::CallExpression(call) = expression.get_inner_expression() else {
            return false;
        };
        let Some(reference) = weak_map_receiver(call) else {
            return false;
        };
        let [key, value] = call.arguments.as_slice() else {
            return false;
        };
        if !key
            .as_expression()
            .is_some_and(|key| self.reads(key, symbols[0]))
            || !self.metadata_value(value, symbols[1], captures)
        {
            return false;
        }
        let Some(symbol) = reference_symbol(reference, self.semantic) else {
            return false;
        };
        *self
            .private_weak_maps
            .entry(symbol)
            .or_insert_with(|| private_weak_map(self.semantic, symbol))
    }

    fn metadata_value(
        &self,
        value: &Argument<'_>,
        member_name: Option<SymbolId>,
        captures: Option<ScopeId>,
    ) -> bool {
        if literal_argument(value) {
            return true;
        }
        let Some(Expression::Identifier(reference)) =
            value.as_expression().map(Expression::get_inner_expression)
        else {
            return false;
        };
        let Some(symbol) = reference_symbol(reference, self.semantic) else {
            return false;
        };
        let scoping = self.semantic.scoping();
        member_name == Some(symbol)
            || (captures == Some(scoping.symbol_scope_id(symbol))
                && matches!(
                    self.semantic
                        .nodes()
                        .kind(scoping.symbol_declaration(symbol)),
                    AstKind::FormalParameter(_)
                ))
    }
}

struct Body<'a> {
    params: &'a FormalParameters<'a>,
    code: Code<'a>,
    scope: Option<ScopeId>,
}

enum Code<'a> {
    Expression(&'a Expression<'a>),
    Statements(&'a [Statement<'a>]),
}

impl<'a> Body<'a> {
    fn expression(expression: &'a Expression<'a>) -> Option<Self> {
        match expression.get_inner_expression() {
            Expression::FunctionExpression(function) => Self::function(function),
            Expression::ArrowFunctionExpression(arrow) => Self::arrow(arrow),
            _ => None,
        }
    }

    fn function(function: &'a Function<'a>) -> Option<Self> {
        (!function.r#async && !function.generator).then_some(Self {
            params: &function.params,
            code: Code::Statements(&function.body.as_ref()?.statements),
            scope: function.scope_id.get(),
        })
    }

    fn arrow(arrow: &'a ArrowFunctionExpression<'a>) -> Option<Self> {
        if arrow.r#async {
            return None;
        }
        Some(Self {
            params: &arrow.params,
            scope: arrow.scope_id.get(),
            code: if let Some(expression) = arrow.body.as_expression() {
                Code::Expression(expression)
            } else {
                Code::Statements(&arrow.body.as_function_body()?.statements)
            },
        })
    }

    fn returned_closure(&self) -> Option<&'a Expression<'a>> {
        match self.code {
            Code::Expression(expression) => Some(expression),
            Code::Statements([Statement::ReturnStatement(returned)]) => returned.argument.as_ref(),
            Code::Statements(_) => None,
        }
    }

    fn plain_parameters(&self) -> bool {
        self.params.rest.is_none()
            && self.params.items.iter().all(|parameter| {
                parameter.initializer.is_none()
                    && parameter.decorators.is_empty()
                    && parameter.pattern.get_binding_identifier().is_some()
            })
    }

    fn member_parameters(&self) -> Option<[Option<SymbolId>; 3]> {
        if self.params.items.len() > 3 || !self.plain_parameters() {
            return None;
        }
        let mut symbols = [None; 3];
        for (slot, parameter) in symbols.iter_mut().zip(&self.params.items) {
            *slot = Some(
                parameter
                    .pattern
                    .get_binding_identifier()?
                    .symbol_id
                    .get()?,
            );
        }
        Some(symbols)
    }
}

pub(super) fn literal_argument(argument: &Argument<'_>) -> bool {
    matches!(
        argument
            .as_expression()
            .map(Expression::get_inner_expression),
        Some(
            Expression::BooleanLiteral(_)
                | Expression::NullLiteral(_)
                | Expression::NumericLiteral(_)
                | Expression::BigIntLiteral(_)
                | Expression::StringLiteral(_)
        )
    )
}

fn weak_map_receiver<'a>(call: &'a CallExpression<'a>) -> Option<&'a IdentifierReference<'a>> {
    let Expression::StaticMemberExpression(member) = &call.callee else {
        return None;
    };
    let Expression::Identifier(reference) = &member.object else {
        return None;
    };
    (!call.optional && !member.optional && member.property.name == "set").then_some(reference)
}

fn private_weak_map(semantic: &Semantic<'_>, symbol: SymbolId) -> bool {
    let scopes = semantic.scoping();
    if !scopes.symbol_flags(symbol).is_const_variable()
        || scopes.symbol_is_mutated(symbol)
        || scopes.symbol_scope_id(symbol) != scopes.root_scope_id()
    {
        return false;
    }
    let nodes = semantic.nodes();
    let declaration = scopes.symbol_declaration(symbol);
    let AstKind::VariableDeclarator(variable) = nodes.kind(declaration) else {
        return false;
    };
    if !matches!(
        nodes.parent_kind(nodes.parent_id(declaration)),
        AstKind::Program(_)
    ) {
        return false;
    }
    let Some(Expression::NewExpression(init)) = variable.init.as_ref() else {
        return false;
    };
    if !matches!(&init.callee, Expression::Identifier(reference)
        if reference.name == "WeakMap" && reference_symbol(reference, semantic).is_none())
        || !init.arguments.is_empty()
    {
        return false;
    }
    // Reject aliases, exports, writes, and any other way the map could escape
    // or have its methods replaced. Native WeakMap operations do not call user code.
    scopes.get_resolved_references(symbol).all(|reference| {
        let id = reference.node_id();
        let member_id = nodes.parent_id(id);
        let AstKind::StaticMemberExpression(member) = nodes.kind(member_id) else {
            return false;
        };
        matches!(
            member.property.name.as_str(),
            "set" | "get" | "has" | "delete"
        ) && member.object.node_id() == id
            && matches!(nodes.parent_kind(member_id), AstKind::CallExpression(call)
                if call.callee.node_id() == member_id)
    })
}

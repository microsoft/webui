// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

use std::{borrow::Cow, ops::Range};

use serde_json::Value;
use webui_protocol::{condition_expr::Expr, ComparisonOperator, ConditionExpr, LogicalOperator};
use webui_state::find_value_by_dotted_path_ref;

use crate::{
    compare_values, comparison_operator, evaluation_error, is_literal, logical_operator,
    missing_value, parse_literal,
    traversal::{strip_nots, validate, MAX_OPERATORS},
    truthy, ExpressionError, Result,
};

#[cfg(test)]
#[path = "prepared_storage_tests.rs"]
mod storage_tests;

// Destinations 0 and 1 return false and true. All other destinations index a
// test at destination - 2. Each test selects its next destination directly;
// there is no evaluation stack and no runtime NOT/compound traversal.
const FALSE: usize = 0;
const TRUE: usize = 1;

/// An immutable condition prepared once for repeated evaluation.
///
/// Preparation owns paths and parsed literals, never the wire tree or state.
/// Global operator errors are cached; other errors remain lazy, preserving
/// short-circuit behavior and left-to-right resolver calls.
#[derive(Debug)]
pub struct PreparedCondition {
    kind: PreparedKind,
}

#[derive(Debug)]
enum PreparedKind {
    Constant(bool),
    Identifier { path: Box<str>, negated: bool },
    Tape(Tape),
}

#[derive(Debug)]
struct Tape {
    steps: Box<[Step]>,
    paths: Box<str>,
    entry: usize,
}

#[derive(Debug)]
struct Step {
    test: Test,
    next: [usize; 2],
}

#[derive(Debug)]
enum Test {
    Identifier(Range<usize>),
    Predicate {
        left: Range<usize>,
        right: Operand,
        operator: Result<ComparisonOperator>,
    },
    Error(ExpressionError),
}

#[derive(Debug)]
enum Operand {
    Path(Range<usize>),
    Literal(Result<Value>),
}

impl PreparedCondition {
    /// Prepare a wire condition without evaluating it or resolving any state.
    ///
    /// This is infallible: malformed input is retained as a lazy evaluation
    /// error, not reported prematurely from a potentially skipped subtree.
    #[must_use]
    pub fn new(condition: &ConditionExpr) -> Self {
        let (condition, negated) = strip_nots(condition);
        if let Some(Expr::Identifier(id)) = &condition.expr {
            let kind = match id.value.as_str() {
                "true" => PreparedKind::Constant(!negated),
                "false" => PreparedKind::Constant(negated),
                path => PreparedKind::Identifier {
                    path: path.into(),
                    negated,
                },
            };
            return Self { kind };
        }
        let mut compiler = Compiler {
            steps: Vec::new(),
            paths: String::new(),
        };
        let next = if negated {
            [TRUE, FALSE]
        } else {
            [FALSE, TRUE]
        };
        let entry = if matches!(condition.expr, Some(Expr::Compound(_))) {
            match validate(condition) {
                Ok(()) => compiler.compile(condition, next),
                Err(error) => compiler.push(Test::Error(error), next),
            }
        } else {
            compiler.compile(condition, next)
        };
        Self {
            kind: PreparedKind::Tape(Tape {
                steps: compiler.steps.into_boxed_slice(),
                paths: compiler.paths.into_boxed_str(),
                entry,
            }),
        }
    }

    /// Evaluate against JSON state, borrowing existing values during lookups.
    #[inline]
    pub fn evaluate(&self, state: &Value) -> Result<bool> {
        self.evaluate_with_resolver(|path| find_value_by_dotted_path_ref(path, state))
    }

    /// Evaluate with a borrowed/owned resolver, without allocating traversal
    /// storage, revalidating the tree, or reparsing right-hand literals.
    ///
    /// Missing identifiers are false; missing predicate operands are errors.
    /// The resolver may allocate synthetic values (for example `.length`).
    #[inline]
    pub fn evaluate_with_resolver<'a, F>(&self, resolver: F) -> Result<bool>
    where
        F: Fn(&str) -> Option<Cow<'a, Value>>,
    {
        match &self.kind {
            PreparedKind::Constant(value) => Ok(*value),
            PreparedKind::Identifier { path, negated } => {
                Ok(resolver(path).is_some_and(|value| truthy(value.as_ref())) ^ negated)
            }
            PreparedKind::Tape(tape) => tape.evaluate(&resolver),
        }
    }
}

impl Tape {
    fn evaluate<'a, F>(&self, resolver: &F) -> Result<bool>
    where
        F: Fn(&str) -> Option<Cow<'a, Value>>,
    {
        let mut destination = self.entry;
        while destination >= 2 {
            let step = &self.steps[destination - 2];
            let result = match &step.test {
                Test::Identifier(path) => {
                    resolver(&self.paths[path.clone()]).is_some_and(|value| truthy(value.as_ref()))
                }
                Test::Predicate {
                    left,
                    right,
                    operator,
                } => {
                    let path = &self.paths[left.clone()];
                    let left = resolver(path).ok_or_else(|| missing_value(path))?;
                    let right = match right {
                        Operand::Path(path) => {
                            let path = &self.paths[path.clone()];
                            resolver(path).ok_or_else(|| missing_value(path))?
                        }
                        Operand::Literal(value) => {
                            Cow::Borrowed(value.as_ref().map_err(clone_error)?)
                        }
                    };
                    let operator = operator.as_ref().map_err(clone_error)?;
                    compare_values(left.as_ref(), operator, right.as_ref())?
                }
                Test::Error(error) => return Err(clone_error(error)),
            };
            destination = step.next[usize::from(result)];
        }
        Ok(destination == TRUE)
    }
}

struct Compiler {
    steps: Vec<Step>,
    paths: String,
}

impl Compiler {
    fn path(&mut self, path: &str) -> Range<usize> {
        let start = self.paths.len();
        self.paths.push_str(path);
        start..self.paths.len()
    }

    fn push(&mut self, test: Test, next: [usize; 2]) -> usize {
        self.steps.push(Step { test, next });
        self.steps.len() + 1
    }

    // Compile right before left so short-circuit destinations already exist.
    // This changes only preparation order, never resolver/evaluation order.
    fn compile(&mut self, mut condition: &ConditionExpr, mut next: [usize; 2]) -> usize {
        let mut pending = Vec::new();
        loop {
            let (inner, negated) = strip_nots(condition);
            condition = inner;
            if negated {
                next.swap(0, 1);
            }
            let entry = match &condition.expr {
                Some(Expr::Compound(compound)) => {
                    match (compound.left.as_deref(), compound.right.as_deref()) {
                        (None, _) => self.push(
                            Test::Error(evaluation_error("Compound missing left expression")),
                            next,
                        ),
                        (_, None) => self.push(
                            Test::Error(evaluation_error("Compound missing right expression")),
                            next,
                        ),
                        (Some(left), Some(right)) => {
                            match logical_operator(compound.op) {
                                Ok(operator) if operator != LogicalOperator::Unspecified => {
                                    if pending.is_empty() {
                                        pending.reserve(MAX_OPERATORS);
                                    }
                                    pending.push((left, operator == LogicalOperator::And, next));
                                    condition = right;
                                }
                                operator => {
                                    let error = match operator {
                                        Err(error) => error,
                                        Ok(_) => evaluation_error("Unspecified logical operator"),
                                    };
                                    let entry = self.push(Test::Error(error), next);
                                    condition = left;
                                    next = [entry; 2];
                                }
                            }
                            continue;
                        }
                    }
                }
                Some(Expr::Identifier(id)) if id.value == "true" => next[1],
                Some(Expr::Identifier(id)) if id.value == "false" => next[0],
                Some(Expr::Identifier(id)) => {
                    let path = self.path(&id.value);
                    self.push(Test::Identifier(path), next)
                }
                Some(Expr::Predicate(predicate)) => {
                    let left = self.path(&predicate.left);
                    let right = if is_literal(&predicate.right) {
                        Operand::Literal(parse_literal(&predicate.right))
                    } else {
                        Operand::Path(self.path(&predicate.right))
                    };
                    self.push(
                        Test::Predicate {
                            left,
                            right,
                            operator: comparison_operator(predicate.operator),
                        },
                        next,
                    )
                }
                Some(Expr::Not(_)) => self.push(
                    Test::Error(evaluation_error("Not condition missing inner expression")),
                    next,
                ),
                None => self.push(
                    Test::Error(evaluation_error("Empty condition expression")),
                    next,
                ),
            };
            let Some((left, is_and, parent_next)) = pending.pop() else {
                return entry;
            };
            condition = left;
            next = if is_and {
                [parent_next[0], entry]
            } else {
                [entry, parent_next[1]]
            };
        }
    }
}

#[cold]
#[inline(never)]
fn clone_error(error: &ExpressionError) -> ExpressionError {
    error.clone()
}

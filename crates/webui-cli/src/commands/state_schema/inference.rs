// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

use anyhow::{Context, Result};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};
use std::rc::Rc;
use webui_protocol::condition_expr;
use webui_protocol::web_ui_fragment;
use webui_protocol::{
    ComparisonOperator, ConditionExpr, WebUIFragment, WebUIFragmentAttribute, WebUIProtocol,
};

use super::model::{add_array_path, add_resolved_path, node_to_schema, InferredKind, Node};
use super::scope::{array_item_path, has_component_scope, resolve_path, BindingOrigin, Scope};

struct WalkFrame<'a> {
    fragment_id: &'a str,
    fragments: &'a [WebUIFragment],
    next_index: usize,
    scope: Rc<Scope>,
    component_attrs: BTreeMap<String, BindingOrigin>,
    collecting_component_attrs: bool,
    binding_requirement: Requirement,
}

#[derive(Default)]
struct WalkState<'a> {
    stack: Vec<WalkFrame<'a>>,
    active_fragments: BTreeSet<&'a str>,
}

enum NextFrame<'a> {
    None,
    Push {
        fragment_id: &'a str,
        scope: Rc<Scope>,
        binding_requirement: Requirement,
    },
}

#[derive(Clone, Copy)]
enum Requirement {
    Optional,
    Scoped,
    Required,
}

pub(super) struct SchemaInference<'a> {
    protocol: &'a WebUIProtocol,
    root: Node,
}

impl<'a> SchemaInference<'a> {
    pub(super) fn new(protocol: &'a WebUIProtocol) -> Self {
        Self {
            protocol,
            root: Node::root(),
        }
    }

    pub(super) fn infer_entry(&mut self, fragment_id: &'a str) -> Result<()> {
        self.infer_fragment(fragment_id, Rc::new(Scope::Root))
    }

    pub(super) fn infer_root_component(&mut self, fragment_id: &'a str) -> Result<()> {
        self.infer_fragment(
            fragment_id,
            Rc::new(Scope::Component {
                attrs: BTreeMap::new(),
                require_global_fallback: true,
            }),
        )
    }

    fn infer_fragment(&mut self, fragment_id: &'a str, scope: Rc<Scope>) -> Result<()> {
        let mut walk = WalkState::default();
        self.push_frame(fragment_id, scope, Requirement::Required, &mut walk)?;

        while !walk.stack.is_empty() {
            let should_pop = walk
                .stack
                .last()
                .is_some_and(|frame| frame.next_index >= frame.fragments.len());
            if should_pop {
                if let Some(frame) = walk.stack.pop() {
                    walk.active_fragments.remove(frame.fragment_id);
                }
                continue;
            }

            let action = {
                let Some(frame) = walk.stack.last_mut() else {
                    break;
                };
                let fragment = &frame.fragments[frame.next_index];
                frame.next_index += 1;
                self.process_fragment(fragment, frame)
            };

            if let NextFrame::Push {
                fragment_id,
                scope,
                binding_requirement,
            } = action
            {
                self.push_frame(fragment_id, scope, binding_requirement, &mut walk)?;
            }
        }
        Ok(())
    }

    pub(super) fn into_schema(self) -> Value {
        node_to_schema(&self.root)
    }

    fn push_frame(
        &self,
        fragment_id: &'a str,
        scope: Rc<Scope>,
        binding_requirement: Requirement,
        walk: &mut WalkState<'a>,
    ) -> Result<()> {
        if walk.active_fragments.contains(fragment_id) {
            return Ok(());
        }
        let fragment_list = self
            .protocol
            .fragments
            .get(fragment_id)
            .with_context(|| format!("Protocol fragment '{fragment_id}' was not found"))?;
        walk.active_fragments.insert(fragment_id);
        walk.stack.push(WalkFrame {
            fragment_id,
            fragments: &fragment_list.fragments,
            next_index: 0,
            scope,
            component_attrs: BTreeMap::new(),
            collecting_component_attrs: false,
            binding_requirement,
        });
        Ok(())
    }

    fn process_fragment(
        &mut self,
        fragment: &'a WebUIFragment,
        frame: &mut WalkFrame<'a>,
    ) -> NextFrame<'a> {
        match fragment.fragment.as_ref() {
            Some(web_ui_fragment::Fragment::Signal(signal)) => {
                if !is_runtime_signal(&signal.value, signal.raw) {
                    add_path(
                        &mut self.root,
                        &frame.scope,
                        &signal.value,
                        if signal.raw {
                            InferredKind::String
                        } else {
                            InferredKind::Scalar
                        },
                        frame.binding_requirement,
                    );
                }
                NextFrame::None
            }
            Some(web_ui_fragment::Fragment::ForLoop(for_loop)) => {
                let origin = resolve_path(&frame.scope, &for_loop.collection);
                let item_origin = match origin {
                    BindingOrigin::RootPath { path, required } => {
                        add_array_path(&mut self.root, &path, required);
                        BindingOrigin::RootPath {
                            path: array_item_path(path),
                            required,
                        }
                    }
                    BindingOrigin::LocalOnly => BindingOrigin::LocalOnly,
                };
                NextFrame::Push {
                    fragment_id: &for_loop.fragment_id,
                    scope: Rc::new(Scope::Loop {
                        item: for_loop.item.clone(),
                        origin: item_origin,
                        parent: Rc::clone(&frame.scope),
                    }),
                    binding_requirement: Requirement::Required,
                }
            }
            Some(web_ui_fragment::Fragment::IfCond(if_cond)) => {
                if let Some(condition) = &if_cond.condition {
                    add_condition_paths(&mut self.root, &frame.scope, condition);
                }
                NextFrame::Push {
                    fragment_id: &if_cond.fragment_id,
                    scope: Rc::clone(&frame.scope),
                    binding_requirement: Requirement::Required,
                }
            }
            Some(web_ui_fragment::Fragment::Attribute(attribute)) => {
                self.process_attribute(attribute, frame)
            }
            Some(web_ui_fragment::Fragment::Component(component)) => {
                let attrs = if frame.collecting_component_attrs {
                    frame.collecting_component_attrs = false;
                    std::mem::take(&mut frame.component_attrs)
                } else {
                    BTreeMap::new()
                };
                let require_global_fallback = !has_component_scope(&frame.scope);
                NextFrame::Push {
                    fragment_id: &component.fragment_id,
                    scope: Rc::new(Scope::Component {
                        attrs,
                        require_global_fallback,
                    }),
                    binding_requirement: Requirement::Required,
                }
            }
            Some(web_ui_fragment::Fragment::Raw(_))
            | Some(web_ui_fragment::Fragment::Plugin(_))
            | Some(web_ui_fragment::Fragment::Route(_))
            | Some(web_ui_fragment::Fragment::Outlet(_))
            | None => NextFrame::None,
        }
    }

    fn process_attribute(
        &mut self,
        attribute: &'a WebUIFragmentAttribute,
        frame: &mut WalkFrame<'a>,
    ) -> NextFrame<'a> {
        if attribute.attr_start {
            frame.component_attrs.clear();
            frame.collecting_component_attrs = true;
        }

        if let Some(condition) = &attribute.condition_tree {
            add_condition_paths(&mut self.root, &frame.scope, condition);
        } else if !attribute.template.is_empty() {
            // The template fragment is traversed below with the parent scope.
        } else if !attribute.raw_value && !attribute.value.is_empty() {
            add_path(
                &mut self.root,
                &frame.scope,
                &attribute.value,
                if attribute.complex {
                    InferredKind::Any
                } else {
                    InferredKind::Scalar
                },
                Requirement::Scoped,
            );
        }

        if frame.collecting_component_attrs && !attribute.attr_skip {
            let name = component_attr_name(&attribute.name);
            let origin = if attribute.condition_tree.is_some()
                || !attribute.template.is_empty()
                || attribute.raw_value
                || attribute.value.is_empty()
                || !attribute.complex
            {
                BindingOrigin::LocalOnly
            } else {
                resolve_path(&frame.scope, &attribute.value)
            };
            frame.component_attrs.insert(name, origin);
        }

        if attribute.template.is_empty() {
            NextFrame::None
        } else {
            NextFrame::Push {
                fragment_id: &attribute.template,
                scope: Rc::clone(&frame.scope),
                binding_requirement: Requirement::Scoped,
            }
        }
    }
}

fn component_attr_name(name: &str) -> String {
    let stripped = name.strip_prefix(':').unwrap_or(name);
    webui_protocol::attrs::attribute_to_camel(stripped)
}

fn add_condition_paths(root: &mut Node, scope: &Rc<Scope>, condition: &ConditionExpr) {
    let mut pending = vec![condition];
    while let Some(current) = pending.pop() {
        match current.expr.as_ref() {
            Some(condition_expr::Expr::Identifier(identifier)) => {
                add_path(
                    root,
                    scope,
                    &identifier.value,
                    InferredKind::Any,
                    Requirement::Optional,
                );
            }
            Some(condition_expr::Expr::Predicate(predicate)) => {
                add_predicate_paths(root, scope, predicate);
            }
            Some(condition_expr::Expr::Not(not)) => {
                if let Some(inner) = &not.condition {
                    pending.push(inner);
                }
            }
            Some(condition_expr::Expr::Compound(compound)) => {
                if let Some(right) = &compound.right {
                    pending.push(right);
                }
                if let Some(left) = &compound.left {
                    pending.push(left);
                }
            }
            None => {}
        }
    }
}

fn add_predicate_paths(root: &mut Node, scope: &Rc<Scope>, predicate: &webui_protocol::Predicate) {
    let operator = ComparisonOperator::try_from(predicate.operator).ok();
    let right_literal = literal_kind(&predicate.right);

    match operator {
        Some(ComparisonOperator::GreaterThan)
        | Some(ComparisonOperator::LessThan)
        | Some(ComparisonOperator::GreaterThanOrEqual)
        | Some(ComparisonOperator::LessThanOrEqual) => {
            add_path(
                root,
                scope,
                &predicate.left,
                InferredKind::Number,
                Requirement::Optional,
            );
            if right_literal.is_none() {
                add_path(
                    root,
                    scope,
                    &predicate.right,
                    InferredKind::Number,
                    Requirement::Optional,
                );
            }
        }
        _ => match right_literal {
            Some(kind) => add_path(root, scope, &predicate.left, kind, Requirement::Optional),
            None => {
                add_path(
                    root,
                    scope,
                    &predicate.left,
                    InferredKind::Scalar,
                    Requirement::Optional,
                );
                add_path(
                    root,
                    scope,
                    &predicate.right,
                    InferredKind::Scalar,
                    Requirement::Optional,
                );
            }
        },
    }
}

fn literal_kind(value: &str) -> Option<InferredKind> {
    if value == "true" || value == "false" {
        return Some(InferredKind::Boolean);
    }
    if value.parse::<f64>().is_ok() {
        return Some(InferredKind::Number);
    }
    if (value.starts_with('"') && value.ends_with('"'))
        || (value.starts_with('\'') && value.ends_with('\''))
    {
        return Some(InferredKind::String);
    }
    None
}

fn add_path(
    root: &mut Node,
    scope: &Rc<Scope>,
    path: &str,
    kind: InferredKind,
    requirement: Requirement,
) {
    if path.is_empty() {
        return;
    }
    if let BindingOrigin::RootPath {
        path: resolved,
        required: scope_required,
    } = resolve_path(scope, path)
    {
        let required = match requirement {
            Requirement::Optional => false,
            Requirement::Scoped => scope_required,
            Requirement::Required => true,
        };
        add_resolved_path(root, &resolved, kind, required);
    }
}

fn is_runtime_signal(value: &str, raw: bool) -> bool {
    raw && matches!(value, "head_end" | "body_start" | "body_end")
}

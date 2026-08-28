// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

//! Version-3 checkpoint, update, span-completion, and terminal serialization.

use super::error::component_style_payload_resources_missing_error;
use super::inventory::{
    commit_checkpoint_inventory, expand_static_checkpoint_reachability,
    mark_streaming_style_resource_sent, mark_streaming_template_sent,
    replace_checkpoint_reachability,
};
use super::state::StateUpdatePlan;
use super::{flush_streaming_transport, streaming_state, MarkerBuffer};
use crate::plugin::WebUiTemplatePayload;
use crate::{
    collect_hydration_key_ids_into, write_selected_state, write_webui_bootstrap, HandlerError,
    HydrationKeySelection, Result, StateSelection, WebUIHandler, WebUIProcessContext,
    WebUiBootstrap, WebUiBootstrapState,
};

pub(super) const RECORD_KIND_FINAL_CHECKPOINT: usize = 0;
pub(super) const RECORD_KIND_UPDATABLE_CHECKPOINT: usize = 1;
pub(super) const RECORD_KIND_STATE_UPDATE: usize = 2;
pub(super) const RECORD_KIND_SPAN_COMPLETION: usize = 3;
pub(super) const RECORD_KIND_TERMINAL: usize = 4;
const STREAMING_PROTOCOL_VERSION: usize = 3;

/// The range-bearing record currently being committed.
pub(super) enum RangeRecord {
    Boundary {
        instance_id: u32,
        declaration_id: u32,
        enclosing_span_instance_id: Option<u32>,
        updatable: bool,
    },
    Span {
        instance_id: u32,
    },
}

impl RangeRecord {
    fn target(&self) -> u32 {
        match self {
            Self::Boundary { instance_id, .. } | Self::Span { instance_id } => *instance_id,
        }
    }

    fn kind(&self) -> usize {
        match self {
            Self::Boundary {
                updatable: true, ..
            } => RECORD_KIND_UPDATABLE_CHECKPOINT,
            Self::Boundary {
                updatable: false, ..
            } => RECORD_KIND_FINAL_CHECKPOINT,
            Self::Span { .. } => RECORD_KIND_SPAN_COMPLETION,
        }
    }
}

#[derive(Clone, Copy)]
enum StateDelta {
    Empty,
    Keys,
    FullExceptLast,
}

/// Build `current - base` while proving that the sorted base is a subset.
fn collect_state_key_delta(base: &[u32], current: &[u32], delta: &mut Vec<u32>) -> bool {
    let mut base_index = 0usize;
    let mut current_index = 0usize;
    while current_index < current.len() {
        if base_index == base.len() {
            delta.extend_from_slice(&current[current_index..]);
            return true;
        }
        match current[current_index].cmp(&base[base_index]) {
            std::cmp::Ordering::Less => {
                delta.push(current[current_index]);
                current_index += 1;
            }
            std::cmp::Ordering::Equal => {
                current_index += 1;
                base_index += 1;
            }
            std::cmp::Ordering::Greater => {
                delta.clear();
                return false;
            }
        }
    }
    if base_index == base.len() {
        true
    } else {
        delta.clear();
        false
    }
}

impl WebUIHandler {
    pub(super) fn emit_streaming_range_record(
        &self,
        record: RangeRecord,
        context: &mut WebUIProcessContext,
    ) -> Result<()> {
        let first_checkpoint = context
            .streaming
            .as_ref()
            .is_some_and(|streaming| !streaming.bootstrap_sent);
        let component_index = context.component_index;
        let needs_route_walk = {
            let streaming = streaming_state(context)?;
            commit_checkpoint_inventory(streaming)?;
            !expand_static_checkpoint_reachability(streaming)?
        };
        if needs_route_walk {
            let protocol = context.protocol;
            let request_path = context.request_path;
            let route_index = context.route_index;
            let reachable = {
                let streaming = streaming_state(context)?;
                crate::route_handler::collect_reachable_components_from_roots(
                    protocol,
                    &streaming.checkpoint_walk_roots,
                    streaming.component_reachability,
                    request_path,
                    route_index,
                )
            };
            replace_checkpoint_reachability(streaming_state(context)?, component_index, &reachable);
        }

        // Names are derived from the tag indices on demand: materializing them
        // into a second buffer would allocate once per record for data that is
        // only ever streamed through twice.
        let checkpoint_tags = {
            let streaming = streaming_state(context)?;
            let reachability = streaming.component_reachability;
            let tags = std::mem::take(&mut streaming.checkpoint_tags);
            if tags.iter().any(|&index| reachability.name(index).is_none()) {
                return Err(missing_reachability_name_error());
            }
            tags
        };
        let mut new_template_tags = {
            let streaming = streaming_state(context)?;
            std::mem::take(&mut streaming.template_tag_scratch)
        };
        new_template_tags.clear();
        {
            let streaming = streaming_state(context)?;
            let reachability = streaming.component_reachability;
            for &index in &checkpoint_tags {
                let Some(name) = reachability.name(index) else {
                    return Err(missing_reachability_name_error());
                };
                if mark_streaming_template_sent(streaming, index)? {
                    new_template_tags.push(name);
                }
            }
        }

        // Rendered components emitted module importmaps inline before their
        // declarative shadow roots. Emit the same metadata here only for
        // reachable-but-unrendered descendants.
        if context.css_strategy == webui_protocol::CssStrategy::Module {
            for &name in &new_template_tags {
                if !context.rendered_components.contains(name) {
                    if let Some(css) = context
                        .protocol
                        .components
                        .get(name)
                        .map(|component| component.css.as_str())
                        .filter(|css| !css.is_empty())
                    {
                        self.emit_css_module_importmap(name, css, context)?;
                    }
                }
            }
        }

        let template_payloads = context.plugin.as_ref().and_then(|plugin| {
            plugin.collect_template_payloads_slice(context.protocol, &new_template_tags)
        });
        let (mut state_key_ids, checkpoint_reachability) = {
            let streaming = streaming_state(context)?;
            (
                std::mem::take(&mut streaming.state_key_ids),
                streaming.component_reachability,
            )
        };
        let requires_full_state = collect_hydration_key_ids_into(
            context.protocol,
            checkpoint_reachability,
            checkpoint_tags.iter().copied(),
            &mut state_key_ids,
        );
        let (
            record_sequence,
            state_revision,
            last_state_record_sequence,
            last_state_revision,
            last_state_full,
            mut last_state_key_ids,
            mut state_delta_key_ids,
        ) = {
            let streaming = streaming_state(context)?;
            (
                streaming.next_record_sequence,
                streaming.state_revision,
                streaming.last_state_record_sequence,
                streaming.last_state_revision,
                streaming.last_state_full,
                std::mem::take(&mut streaming.last_state_key_ids),
                std::mem::take(&mut streaming.state_delta_key_ids),
            )
        };
        state_delta_key_ids.clear();
        let can_reference = context
            .state
            .as_object()
            .is_some_and(|state| !state.is_empty())
            && last_state_record_sequence.is_some()
            && last_state_revision == state_revision;
        let can_reference = can_reference && (last_state_full || !last_state_key_ids.is_empty());
        let state_ref = last_state_record_sequence.and_then(|sequence| {
            if !can_reference {
                return None;
            }
            if requires_full_state {
                return Some((
                    sequence,
                    if last_state_full {
                        StateDelta::Empty
                    } else {
                        StateDelta::FullExceptLast
                    },
                ));
            }
            if last_state_full
                || !collect_state_key_delta(
                    &last_state_key_ids,
                    &state_key_ids,
                    &mut state_delta_key_ids,
                )
            {
                return None;
            }
            Some((
                sequence,
                if state_delta_key_ids.is_empty() {
                    StateDelta::Empty
                } else {
                    StateDelta::Keys
                },
            ))
        });
        let chain = if first_checkpoint {
            crate::WebUIHandler::ensure_request_route_chain(context);
            match context.route_chain.as_deref() {
                Some(chain) => chain
                    .iter()
                    .map(crate::route_handler::RouteChainEntry::to_json)
                    .collect::<Vec<_>>(),
                None => Vec::new(),
            }
        } else {
            Vec::new()
        };
        let (mut css_hrefs, mut style_specs) = {
            let streaming = streaming_state(context)?;
            (
                std::mem::take(&mut streaming.css_href_scratch),
                std::mem::take(&mut streaming.style_spec_scratch),
            )
        };
        css_hrefs.clear();
        style_specs.clear();
        let css_strategy = context.css_strategy;
        let is_link = css_strategy == webui_protocol::CssStrategy::Link;
        let is_module = css_strategy == webui_protocol::CssStrategy::Module;
        for &name in &new_template_tags {
            if let Some(component) = context.protocol.components.get(name) {
                if is_link && !component.css_href.is_empty() {
                    css_hrefs.push(component.css_href.as_str());
                }
                if is_module && !component.css.is_empty() {
                    style_specs.push(name);
                }
            }
        }
        let mut deferred_css_modules = Vec::new();
        for &name in &new_template_tags {
            if context.rendered_components.contains(name) {
                continue;
            }
            if let Some(css) = context
                .protocol
                .components
                .get(name)
                .map(|component| component.css.as_str())
                .filter(|css| !css.is_empty())
            {
                deferred_css_modules.push((name, css));
            }
        }

        let empty_payloads: [WebUiTemplatePayload<'_>; 0] = [];
        let payloads = template_payloads.as_deref().unwrap_or(&empty_payloads);
        let entry_style_root = first_checkpoint.then_some(context.entry_id);
        let style_roots = entry_style_root
            .into_iter()
            .chain(new_template_tags.iter().copied());
        let component_styles = {
            let style_inventory = context.streaming.as_ref().map_or(&[][..], |streaming| {
                streaming.style_resource_inventory.as_slice()
            });
            crate::route_handler::collect_component_style_delta(
                context.protocol,
                style_roots,
                style_inventory,
                context.style_resource_index,
            )?
        };
        let resources = component_styles
            .get("resources")
            .and_then(serde_json::Value::as_object)
            .ok_or_else(component_style_payload_resources_missing_error)?;
        for resource in resources.keys() {
            let Some(&index) = context.style_resource_index.get(resource) else {
                continue;
            };
            mark_streaming_style_resource_sent(streaming_state(context)?, index)?;
        }
        write_record_open(context, record.kind(), record.target())?;
        let (declaration_id, enclosing_span_instance_id) = match record {
            RangeRecord::Boundary {
                declaration_id,
                enclosing_span_instance_id,
                ..
            } => (Some(declaration_id), enclosing_span_instance_id),
            RangeRecord::Span { .. } => (None, None),
        };
        let inventory = context
            .streaming
            .as_ref()
            .map_or("", |streaming| streaming.inventory_hex.as_str());
        let bootstrap_state = match state_ref {
            Some((record_sequence, delta)) => {
                let delta = match delta {
                    StateDelta::Empty => None,
                    StateDelta::Keys => Some(StateSelection::KeyIds(HydrationKeySelection {
                        ids: &state_delta_key_ids,
                        index: checkpoint_reachability,
                    })),
                    StateDelta::FullExceptLast => {
                        Some(StateSelection::FullExceptKeyIds(HydrationKeySelection {
                            ids: &last_state_key_ids,
                            index: checkpoint_reachability,
                        }))
                    }
                };
                WebUiBootstrapState::Reference {
                    record_sequence,
                    value: context.state,
                    delta,
                }
            }
            _ => WebUiBootstrapState::Complete {
                value: context.state,
                selection: if requires_full_state {
                    StateSelection::Full
                } else {
                    StateSelection::KeyIds(HydrationKeySelection {
                        ids: &state_key_ids,
                        index: checkpoint_reachability,
                    })
                },
            },
        };
        write_webui_bootstrap(
            context.writer,
            &mut context.json_scratch,
            WebUiBootstrap {
                declaration_id,
                enclosing_span_instance_id,
                state: bootstrap_state,
                chain: &chain,
                inventory,
                nonce: context.nonce,
                css_hrefs: &css_hrefs,
                style_specs: &style_specs,
                component_styles: &component_styles,
                templates: payloads,
            },
        )?;
        context.writer.write("]</script>")?;

        if let Some(importmap) =
            crate::css_module::build_importmap_tag_batch(&deferred_css_modules, context.nonce)
        {
            context.writer.write(&importmap)?;
        }
        if template_payloads.is_none() {
            if let Some(plugin) = context.plugin.as_ref() {
                plugin.emit_templates_slice(
                    context.protocol,
                    &new_template_tags,
                    context.nonce,
                    context.writer,
                )?;
            }
        }
        if let Some(plugin) = context.plugin.as_ref() {
            plugin.emit_bootstrap_extension_payloads(payloads, context.nonce, context.writer)?;
        }
        context.writer.write("<webui-hydrate></webui-hydrate>")?;
        flush_streaming_transport(context)?;

        let target = usize::try_from(record.target())
            .map_err(|_| invalid_record_target_error(record.target()))?;
        if let RangeRecord::Boundary {
            updatable: true, ..
        } = record
        {
            let streaming = streaming_state(context)?;
            if streaming.update_plans.len() <= target {
                streaming.update_plans.resize_with(target + 1, || None);
            }

            // Reuse the slot's existing key buffer so a boundary that commits
            // updatable more than once in a response does not re-allocate.
            let mut plan = streaming.update_plans[target]
                .take()
                .unwrap_or(StateUpdatePlan {
                    requires_full_state,
                    key_ids: Vec::new(),
                });
            plan.requires_full_state = requires_full_state;
            plan.key_ids.clear();
            if !requires_full_state {
                plan.key_ids.extend_from_slice(&state_key_ids);
            }
            streaming.update_plans[target] = Some(plan);
        }
        if requires_full_state {
            last_state_key_ids.clear();
        } else {
            last_state_key_ids.clear();
            last_state_key_ids.extend_from_slice(&state_key_ids);
        }
        state_delta_key_ids.clear();
        {
            let streaming = streaming_state(context)?;
            streaming.last_state_record_sequence = Some(record_sequence);
            streaming.last_state_revision = state_revision;
            streaming.last_state_full = requires_full_state;
            streaming.last_state_key_ids = last_state_key_ids;
            streaming.state_delta_key_ids = state_delta_key_ids;
        }
        finish_capture(
            context,
            CapturedBuffers {
                checkpoint_tags,
                template_tags: new_template_tags,
                state_key_ids,
                css_hrefs,
                style_specs,
            },
        );
        streaming_state(context)?.bootstrap_sent = true;
        Ok(())
    }

    pub(super) fn emit_streaming_terminal(
        &self,
        record_sequence: usize,
        context: &mut WebUIProcessContext,
    ) -> Result<()> {
        write_script_open(context)?;
        write_record_header(context.writer, record_sequence, RECORD_KIND_TERMINAL, 0)?;
        context
            .writer
            .write("{}]</script><webui-hydrate></webui-hydrate>")?;
        flush_streaming_transport(context)
    }

    pub(super) fn emit_streaming_state_update(
        &self,
        record_sequence: usize,
        boundary_id: usize,
        context: &mut WebUIProcessContext,
    ) -> Result<()> {
        if !context.state.is_object() {
            return Err(super::error::state_update_type_error());
        }
        // The plan is moved out for the duration of the write so the record can
        // borrow the writer mutably, then handed straight back: an update never
        // rebuilds or reallocates the projection it committed with.
        let Some(plan) = context
            .streaming
            .as_mut()
            .and_then(|streaming| streaming.update_plans.get_mut(boundary_id))
            .and_then(Option::take)
        else {
            return Err(super::error::boundary_not_updatable_error(boundary_id));
        };

        write_script_open(context)?;
        write_record_header(
            context.writer,
            record_sequence,
            RECORD_KIND_STATE_UPDATE,
            boundary_id,
        )?;
        let result = match context.streaming.as_ref() {
            Some(streaming) if !plan.requires_full_state => write_selected_state(
                context.writer,
                &mut context.json_scratch,
                context.state,
                &StateSelection::KeyIds(HydrationKeySelection {
                    ids: &plan.key_ids,
                    index: streaming.component_reachability,
                }),
            ),
            _ => write_selected_state(
                context.writer,
                &mut context.json_scratch,
                context.state,
                &StateSelection::Full,
            ),
        };
        // Restore the plan before propagating a write failure so a poisoned
        // response still owns its buffers instead of leaking their capacity.
        if let Some(slot) = context
            .streaming
            .as_mut()
            .and_then(|streaming| streaming.update_plans.get_mut(boundary_id))
        {
            *slot = Some(plan);
        }
        result?;
        context
            .writer
            .write("]</script><webui-hydrate></webui-hydrate>")?;
        flush_streaming_transport(context)
    }
}

fn write_record_open(
    context: &mut WebUIProcessContext<'_, '_, '_>,
    kind: usize,
    target: u32,
) -> Result<()> {
    let record_sequence = streaming_state(context)?.next_record_sequence;
    let target = usize::try_from(target).map_err(|_| invalid_record_target_error(target))?;
    write_script_open(context)?;
    write_record_header(context.writer, record_sequence, kind, target)
}

/// Emit `>[<version>,<sequence>,<kind>,<target>,` in one writer call.
fn write_record_header(
    writer: &mut dyn crate::ResponseWriter,
    record_sequence: usize,
    kind: usize,
    target: usize,
) -> Result<()> {
    let mut buffer = MarkerBuffer::new();
    buffer.push_str(">[")?;
    buffer.push_usize(STREAMING_PROTOCOL_VERSION)?;
    buffer.push_str(",")?;
    buffer.push_usize(record_sequence)?;
    buffer.push_str(",")?;
    buffer.push_usize(kind)?;
    buffer.push_str(",")?;
    buffer.push_usize(target)?;
    buffer.push_str(",")?;
    buffer.flush_to(writer)
}

fn write_script_open(context: &mut WebUIProcessContext<'_, '_, '_>) -> Result<()> {
    context
        .writer
        .write("<script type=\"application/json\" data-webui-boundary")?;
    if let Some(nonce) = context.nonce {
        context.writer.write(" nonce=\"")?;
        context
            .writer
            .write(&crate::html_encode::encode_safe(nonce))?;
        context.writer.write("\"")?;
    }
    Ok(())
}

/// Scratch buffers borrowed for one record and returned for reuse.
struct CapturedBuffers<'a> {
    checkpoint_tags: Vec<u32>,
    template_tags: Vec<&'a str>,
    state_key_ids: Vec<u32>,
    css_hrefs: Vec<&'a str>,
    style_specs: Vec<&'a str>,
}

fn finish_capture<'a>(
    context: &mut WebUIProcessContext<'a, '_, '_>,
    mut buffers: CapturedBuffers<'a>,
) {
    let Some(streaming) = context.streaming.as_mut() else {
        return;
    };
    buffers.checkpoint_tags.clear();
    streaming.checkpoint_tags = buffers.checkpoint_tags;
    streaming.checkpoint_walk_roots.clear();
    buffers.template_tags.clear();
    streaming.template_tag_scratch = buffers.template_tags;
    buffers.state_key_ids.clear();
    streaming.state_key_ids = buffers.state_key_ids;
    buffers.css_hrefs.clear();
    streaming.css_href_scratch = buffers.css_hrefs;
    buffers.style_specs.clear();
    streaming.style_spec_scratch = buffers.style_specs;
    streaming.checkpoint_seen.fill(0);
    streaming.checkpoint_needs_expansion = false;
}

#[cold]
#[inline(never)]
fn missing_reachability_name_error() -> HandlerError {
    HandlerError::Invariant("component reachability name is missing".to_string())
}

#[cold]
#[inline(never)]
fn invalid_record_target_error(target: u32) -> HandlerError {
    HandlerError::Invariant(format!(
        "streaming record target {target} does not fit usize"
    ))
}

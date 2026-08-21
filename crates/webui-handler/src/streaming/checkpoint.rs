// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

//! Boundary envelope emission — the wire format half of a commit.
//!
//! A checkpoint carries only what changed since the previous one: the newly
//! rendered component delta, first-delivery template metadata, and the state
//! keys this surface actually reads.

use super::error::component_style_payload_resources_missing_error;
use super::inventory::{
    commit_checkpoint_inventory, expand_static_checkpoint_reachability,
    mark_streaming_style_resource_sent, mark_streaming_template_sent,
    replace_checkpoint_reachability,
};
use super::{flush_streaming_transport, streaming_state};
use crate::plugin::WebUiTemplatePayload;
use crate::{
    collect_hydration_state_into, write_selected_state, write_usize, write_webui_bootstrap, Result,
    StateSelection, WebUIHandler, WebUIProcessContext, WebUiBootstrap,
};

use super::state::StateUpdatePlan;

pub(super) const RECORD_KIND_FINAL_CHECKPOINT: usize = 0;
pub(super) const RECORD_KIND_UPDATABLE_CHECKPOINT: usize = 1;
pub(super) const RECORD_KIND_STATE_UPDATE: usize = 2;
pub(super) const RECORD_KIND_TERMINAL: usize = 3;

impl WebUIHandler {
    pub(super) fn emit_streaming_checkpoint(
        &self,
        record_sequence: usize,
        boundary_id: usize,
        updatable: bool,
        context: &mut WebUIProcessContext,
    ) -> Result<()> {
        let first_checkpoint = context
            .streaming
            .as_ref()
            .is_some_and(|streaming| !streaming.bootstrap_sent);

        // Commit the exact rendered roots before expanding the local metadata
        // surface. The DOM inventory must never claim hidden descendants.
        let component_index = context.component_index;
        let needs_route_walk = {
            let streaming = streaming_state(context)?;
            commit_checkpoint_inventory(streaming)?;
            !expand_static_checkpoint_reachability(streaming)?
        };

        if needs_route_walk {
            // Only a component surface containing authored routes needs request
            // matching. Route-free surfaces use the startup-built integer graph.
            // The protocol references are copied out first so they do not borrow
            // `context` across the mutable streaming-state borrow below.
            let protocol = context.protocol;
            let request_path = context.request_path;
            let route_index = context.route_index;
            let reachable = {
                let streaming = streaming_state(context)?;
                let reachability = streaming.component_reachability;
                crate::route_handler::collect_reachable_components_from_roots(
                    protocol,
                    &streaming.checkpoint_walk_roots,
                    reachability,
                    request_path,
                    route_index,
                )
            };
            replace_checkpoint_reachability(streaming_state(context)?, component_index, &reachable);
        }

        // Move the reusable buffers out so plugin/state code can borrow them
        // without retaining a borrow of the render context. Names are resolved
        // once here; every downstream consumer reads them instead of re-hashing.
        let (checkpoint_tags, checkpoint_names) = {
            let streaming = streaming_state(context)?;
            let reachability = streaming.component_reachability;
            let tags = std::mem::take(&mut streaming.checkpoint_tags);
            let mut names = std::mem::take(&mut streaming.checkpoint_name_scratch);
            names.clear();
            names.reserve(tags.len());
            for &index in &tags {
                let Some(name) = reachability.name(index) else {
                    return Err(crate::HandlerError::Invariant(
                        "component reachability name is missing".to_string(),
                    ));
                };
                names.push(name);
            }
            (tags, names)
        };
        let mut new_template_tags = {
            let streaming = streaming_state(context)?;
            std::mem::take(&mut streaming.template_tag_scratch)
        };
        new_template_tags.clear();
        {
            let streaming = streaming_state(context)?;
            for (&index, &name) in checkpoint_tags.iter().zip(checkpoint_names.iter()) {
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

        // Hydration state covers this root surface even when a descendant is not
        // initially rendered, so a client-side condition can create it without
        // consulting a page-global state object.
        let mut state_key_scratch = {
            let streaming = streaming_state(context)?;
            std::mem::take(&mut streaming.state_key_scratch)
        };
        let requires_full_state = collect_hydration_state_into(
            context.protocol,
            checkpoint_names.iter().copied(),
            &mut state_key_scratch,
        );
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
        context
            .writer
            .write("<script type=\"application/json\" data-webui-boundary")?;
        if let Some(nonce) = context.nonce {
            context.writer.write(" nonce=\"")?;
            context.writer.write(nonce)?;
            context.writer.write("\"")?;
        }
        context.writer.write(">[2,")?;
        write_usize(context.writer, record_sequence)?;
        context.writer.write(",")?;
        write_usize(
            context.writer,
            if updatable {
                RECORD_KIND_UPDATABLE_CHECKPOINT
            } else {
                RECORD_KIND_FINAL_CHECKPOINT
            },
        )?;
        context.writer.write(",")?;
        write_usize(context.writer, boundary_id)?;
        context.writer.write(",")?;
        let inventory = context
            .streaming
            .as_ref()
            .map_or("", |streaming| streaming.inventory_hex.as_str());
        {
            let state_selection = if requires_full_state {
                StateSelection::Full
            } else {
                StateSelection::BorrowedKeys(&state_key_scratch)
            };
            write_webui_bootstrap(
                context.writer,
                &mut context.json_scratch,
                WebUiBootstrap {
                    state: context.state,
                    state_selection,
                    chain: &chain,
                    inventory,
                    nonce: context.nonce,
                    css_hrefs: &css_hrefs,
                    style_specs: &style_specs,
                    component_styles: &component_styles,
                    templates: payloads,
                },
            )?;
        }
        context.writer.write("]</script>")?;

        if let Some(plugin) = context.plugin.as_ref() {
            plugin.emit_bootstrap_extension_payloads(payloads, context.nonce, context.writer)?;
        }
        context.writer.write("<webui-hydrate></webui-hydrate>")?;
        flush_streaming_transport(context)?;
        if let Some(streaming) = context.streaming.as_mut() {
            streaming.bootstrap_sent = true;
            if updatable {
                let target = boundary_id;
                if streaming.update_plans.len() <= target {
                    streaming.update_plans.resize_with(target + 1, || None);
                }
                streaming.update_plans[target] = Some(StateUpdatePlan {
                    requires_full_state,
                    keys: if requires_full_state {
                        Vec::new()
                    } else {
                        state_key_scratch.iter().copied().map(Box::from).collect()
                    },
                });
            }
            state_key_scratch.clear();
            streaming.state_key_scratch = state_key_scratch;
            // Reset the exact-capture buffers for the next checkpoint, retaining
            // their capacity.
            let mut checkpoint_tags = checkpoint_tags;
            checkpoint_tags.clear();
            streaming.checkpoint_tags = checkpoint_tags;
            let mut checkpoint_names = checkpoint_names;
            checkpoint_names.clear();
            streaming.checkpoint_name_scratch = checkpoint_names;
            streaming.checkpoint_walk_roots.clear();
            new_template_tags.clear();
            streaming.template_tag_scratch = new_template_tags;
            css_hrefs.clear();
            streaming.css_href_scratch = css_hrefs;
            style_specs.clear();
            streaming.style_spec_scratch = style_specs;
            streaming.checkpoint_seen.fill(0);
            streaming.checkpoint_needs_expansion = false;
        }
        Ok(())
    }

    pub(super) fn emit_streaming_terminal(
        &self,
        record_sequence: usize,
        context: &mut WebUIProcessContext,
    ) -> Result<()> {
        context
            .writer
            .write("<script type=\"application/json\" data-webui-boundary")?;
        if let Some(nonce) = context.nonce {
            context.writer.write(" nonce=\"")?;
            context.writer.write(nonce)?;
            context.writer.write("\"")?;
        }
        context.writer.write(">[2,")?;
        write_usize(context.writer, record_sequence)?;
        context.writer.write(",")?;
        write_usize(context.writer, RECORD_KIND_TERMINAL)?;
        context
            .writer
            .write(",0,{}]</script><webui-hydrate></webui-hydrate>")?;
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
        let Some(plan) = context
            .streaming
            .as_ref()
            .and_then(|streaming| streaming.update_plans.get(boundary_id))
            .and_then(Option::as_ref)
        else {
            return Err(super::error::boundary_not_updatable_error(boundary_id));
        };

        context
            .writer
            .write("<script type=\"application/json\" data-webui-boundary")?;
        if let Some(nonce) = context.nonce {
            context.writer.write(" nonce=\"")?;
            context.writer.write(nonce)?;
            context.writer.write("\"")?;
        }
        context.writer.write(">[2,")?;
        write_usize(context.writer, record_sequence)?;
        context.writer.write(",")?;
        write_usize(context.writer, RECORD_KIND_STATE_UPDATE)?;
        context.writer.write(",")?;
        write_usize(context.writer, boundary_id)?;
        context.writer.write(",")?;
        let selection = if plan.requires_full_state {
            StateSelection::Full
        } else {
            StateSelection::Keys(plan.keys.iter().map(Box::as_ref).collect())
        };
        write_selected_state(
            context.writer,
            &mut context.json_scratch,
            context.state,
            &selection,
        )?;
        context
            .writer
            .write("]</script><webui-hydrate></webui-hydrate>")?;
        flush_streaming_transport(context)
    }
}

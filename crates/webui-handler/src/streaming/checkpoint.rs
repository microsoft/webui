// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

//! Boundary envelope emission — the wire format half of a commit.
//!
//! A checkpoint carries only what changed since the previous one: the newly
//! rendered component delta, first-delivery template metadata, and the state
//! keys this surface actually reads.

use super::inventory::{
    commit_checkpoint_inventory, expand_static_checkpoint_reachability,
    mark_streaming_template_sent, replace_checkpoint_reachability,
};
use super::{flush_streaming_transport, streaming_state};
use crate::plugin::WebUiTemplatePayload;
use crate::{
    collect_hydration_state_into, write_usize, write_webui_bootstrap, Result, StateSelection,
    WebUIHandler, WebUIProcessContext, WebUiBootstrap,
};

impl WebUIHandler {
    pub(super) fn emit_streaming_checkpoint(
        &self,
        sequence: usize,
        terminal: bool,
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
            commit_checkpoint_inventory(streaming, component_index)?;
            !expand_static_checkpoint_reachability(streaming, component_index)?
        };

        if needs_route_walk {
            // Only a component surface containing authored routes needs request
            // matching. Route-free surfaces use the startup-built integer graph.
            let roots = context.streaming.as_ref().map_or(&[][..], |streaming| {
                streaming.checkpoint_route_roots.as_slice()
            });
            let reachable = crate::route_handler::collect_reachable_components_from_roots(
                context.protocol,
                roots,
                context.request_path,
                context.route_index,
            );
            replace_checkpoint_reachability(streaming_state(context)?, component_index, &reachable);
        }

        // Move the reusable tag vector out so plugin/state code can borrow it
        // without retaining a borrow of the render context.
        let checkpoint_tags = {
            let streaming = streaming_state(context)?;
            std::mem::take(&mut streaming.checkpoint_tags)
        };
        let mut new_template_tags = {
            let streaming = streaming_state(context)?;
            std::mem::take(&mut streaming.template_tag_scratch)
        };
        new_template_tags.clear();
        {
            let streaming = streaming_state(context)?;
            for &name in &checkpoint_tags {
                if mark_streaming_template_sent(streaming, component_index, name)? {
                    new_template_tags.push(name);
                }
            }
        }

        // Rendered components emitted module importmaps inline before their
        // declarative shadow roots. Emit the same metadata here only for
        // reachable-but-unrendered descendants.
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
            checkpoint_tags.iter().copied(),
            &mut state_key_scratch,
        );
        let chain = if first_checkpoint {
            crate::route_handler::collect_route_chain(
                context.protocol,
                context.entry_id,
                context.request_path,
                context.route_index,
            )
            .iter()
            .map(crate::route_handler::RouteChainEntry::to_json)
            .collect::<Vec<_>>()
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
        let is_link = context.protocol.css_strategy() == webui_protocol::CssStrategy::Link;
        for &name in &new_template_tags {
            if let Some(component) = context.protocol.components.get(name) {
                if is_link && !component.css_href.is_empty() {
                    css_hrefs.push(component.css_href.as_str());
                }
                if !component.css.is_empty() {
                    style_specs.push(name);
                }
            }
        }

        let empty_payloads: [WebUiTemplatePayload<'_>; 0] = [];
        let payloads = template_payloads.as_deref().unwrap_or(&empty_payloads);
        context
            .writer
            .write("<script type=\"application/json\" data-webui-boundary")?;
        if let Some(nonce) = context.nonce {
            context.writer.write(" nonce=\"")?;
            context.writer.write(nonce)?;
            context.writer.write("\"")?;
        }
        context.writer.write(">[1,")?;
        write_usize(context.writer, sequence)?;
        context.writer.write(if terminal { ",1," } else { ",0," })?;
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
            state_key_scratch.clear();
            streaming.state_key_scratch = state_key_scratch;
            // Reset the exact-capture buffers for the next checkpoint, retaining
            // their capacity.
            let mut checkpoint_tags = checkpoint_tags;
            checkpoint_tags.clear();
            streaming.checkpoint_tags = checkpoint_tags;
            streaming.checkpoint_route_roots.clear();
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
        sequence: usize,
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
        context.writer.write(">[1,")?;
        write_usize(context.writer, sequence)?;
        context
            .writer
            .write(",1,{}]</script><webui-hydrate></webui-hydrate>")?;
        flush_streaming_transport(context)
    }
}

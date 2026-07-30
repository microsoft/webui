// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

//! Host-driven lifetime for one streamed HTML response.

use std::borrow::Cow;
use std::collections::{HashMap, HashSet};

use serde_json::Value;

use super::error::{boundary_order_error, unknown_boundary_name_error};
use super::state::increment_streaming_record_sequence;
use super::{
    flush_streaming_transport, validate_streaming_head_start, StreamingEntryPlan,
    StreamingRenderState, StreamingSink,
};
use crate::plugin::HandlerPlugin;
use crate::route_handler::Protocol;
use crate::{
    FlushWriter, HandlerError, RenderOptions, ResponseWriter, Result, WebUIHandler,
    WebUIProcessContext, INITIAL_KEY_CAPACITY,
};

/// Integer handle for a compile-time streaming boundary.
///
/// Resolve an authored boundary name once with
/// [`StreamingResponse::boundary`], then reuse this allocation-free handle for
/// every response operation.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct BoundaryId(u32);

impl BoundaryId {
    pub(super) fn from_index(index: usize) -> Result<Self> {
        u32::try_from(index)
            .map(Self)
            .map_err(|_| HandlerError::Invariant("boundary index exceeds u32".to_string()))
    }

    fn index(self) -> usize {
        self.0 as usize
    }
}

/// Whether a committed boundary may receive later state records.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BoundaryMode {
    /// Hydrate once and release every boundary-local reference after activation.
    Final,
    /// Retain the boundary roots and compiled state projection until terminal.
    Updatable,
}

/// A host-controlled progressive HTML response.
///
/// Each method synchronously borrows only the state value supplied to that
/// call. The host may await backend work between calls while this response
/// retains renderer inventory, transport backpressure, and hydration metadata.
pub struct StreamingResponse<'a, W: FlushWriter + ?Sized> {
    handler: &'a WebUIHandler,
    protocol: &'a Protocol,
    plan: ResponsePlan<'a>,
    sink: StreamingSink<'a, W>,
    request_path: &'a str,
    entry_id: &'a str,
    nonce: Option<&'a str>,
    head_inject: Option<&'a str>,
    body_inject: Option<&'a str>,
    cursor: usize,
    next_boundary: usize,
    shell_written: bool,
    finished: bool,
    local_vars: HashMap<String, Value>,
    component_attrs: HashMap<String, Value>,
    route_base: Cow<'a, str>,
    rendered_components: HashSet<String>,
    plugin: Option<Box<dyn HandlerPlugin>>,
    route_children: Vec<webui_protocol::WebUiFragmentRoute>,
    head_end_emitted: bool,
    body_end_emitted: bool,
    route_chain_index: usize,
    entry_route: Option<(String, crate::route_matcher::RouteMatch)>,
    streaming: StreamingRenderState<'a>,
    json_scratch: Vec<u8>,
    scope_pool: Vec<HashMap<String, Value>>,
}

enum ResponsePlan<'a> {
    Shared(&'a StreamingEntryPlan),
    Request(StreamingEntryPlan),
}

impl ResponsePlan<'_> {
    fn get(&self) -> &StreamingEntryPlan {
        match self {
            Self::Shared(plan) => plan,
            Self::Request(plan) => plan,
        }
    }
}

impl WebUIHandler {
    /// Start a host-driven progressive HTML response.
    ///
    /// # Errors
    ///
    /// Returns an error when the entry is missing or its compiled streaming
    /// structure is malformed.
    pub fn stream_response<'a, W: FlushWriter + ?Sized>(
        &'a self,
        protocol: &'a Protocol,
        options: &RenderOptions<'a>,
        writer: &'a mut W,
    ) -> Result<StreamingResponse<'a, W>> {
        let document = protocol.protocol();
        let fragments = document
            .fragments
            .get(options.entry_id)
            .ok_or_else(|| HandlerError::MissingFragment(options.entry_id.to_string()))?;
        validate_streaming_head_start(document, options.entry_id)?;
        let plan = protocol.streaming_plan(options.entry_id).map_or_else(
            || {
                ResponsePlan::Request(StreamingEntryPlan::new(
                    options.entry_id,
                    &fragments.fragments,
                    None,
                ))
            },
            ResponsePlan::Shared,
        );
        let inventory_bytes = protocol.component_index().len().div_ceil(8);
        let entry_route = crate::route_renderer::find_best_route_match(
            &fragments.fragments,
            options.request_path,
            "/",
            protocol.route_index(),
        );

        Ok(StreamingResponse {
            handler: self,
            protocol,
            plan,
            sink: StreamingSink { transport: writer },
            request_path: options.request_path,
            entry_id: options.entry_id,
            nonce: options.nonce.filter(|nonce| !nonce.is_empty()),
            head_inject: options.head_inject.filter(|html| !html.is_empty()),
            body_inject: options.body_inject.filter(|html| !html.is_empty()),
            cursor: 0,
            next_boundary: 0,
            shell_written: false,
            finished: false,
            local_vars: HashMap::new(),
            component_attrs: HashMap::new(),
            route_base: Cow::Borrowed("/"),
            rendered_components: HashSet::new(),
            plugin: self.plugin_factory.map(|factory| factory()),
            route_children: Vec::new(),
            head_end_emitted: false,
            body_end_emitted: false,
            route_chain_index: 0,
            entry_route,
            streaming: StreamingRenderState {
                component_reachability: protocol.component_reachability(),
                head_marker_emitted: false,
                active_boundary: None,
                pending_root: None,
                generated_root_ready: false,
                next_boundary_id: 0,
                next_record_sequence: 0,
                checkpoint_updatable: false,
                bootstrap_sent: false,
                body_ended: false,
                inventory: vec![0; inventory_bytes],
                inventory_delta: vec![0; inventory_bytes],
                inventory_hex: String::with_capacity(inventory_bytes * 2),
                template_inventory: vec![0; inventory_bytes],
                checkpoint_tags: Vec::new(),
                checkpoint_walk_roots: Vec::new(),
                checkpoint_seen: vec![0; inventory_bytes],
                checkpoint_needs_expansion: false,
                state_key_scratch: Vec::with_capacity(INITIAL_KEY_CAPACITY),
                template_tag_scratch: Vec::new(),
                css_href_scratch: Vec::new(),
                style_spec_scratch: Vec::new(),
                reachability_stack: Vec::new(),
                update_plans: Vec::new(),
            },
            json_scratch: Vec::new(),
            scope_pool: Vec::new(),
        })
    }
}

impl<'a, W: FlushWriter + ?Sized> StreamingResponse<'a, W> {
    /// Resolve a free-form authored name to an integer response handle.
    ///
    /// # Errors
    ///
    /// Returns an actionable error with valid names and a typo suggestion when
    /// the entry does not declare `name`.
    pub fn boundary(&self, name: &str) -> Result<BoundaryId> {
        let names = self.protocol.streaming_boundary_names(self.entry_id);
        let Some(index) = names.iter().position(|candidate| candidate == name) else {
            return Err(unknown_boundary_name_error(name, names));
        };
        BoundaryId::from_index(index)
    }

    /// Number of compile-time boundaries in this entry.
    #[must_use]
    pub fn boundary_count(&self) -> usize {
        self.plan.get().boundary_count()
    }

    /// Render and flush the document prefix before the first boundary.
    pub fn write_shell(&mut self, state: &Value) -> Result<()> {
        self.write_shell_internal(state, true)
    }

    pub(super) fn write_shell_buffered(&mut self, state: &Value) -> Result<()> {
        self.write_shell_internal(state, false)
    }

    fn write_shell_internal(&mut self, state: &Value, flush: bool) -> Result<()> {
        if self.shell_written {
            return Err(boundary_order_error(
                "write_shell",
                "the response shell has already been written",
            ));
        }
        let shell_end = self.plan.get().shell_end();
        self.run_range(state, 0..shell_end)?;
        self.cursor = shell_end;
        self.shell_written = true;
        if flush && !self.body_ended() {
            self.with_context(state, |_handler, context| {
                flush_streaming_transport(context)
            })?;
        }
        Ok(())
    }

    /// Render, commit, and flush the next compile-time boundary.
    pub fn write_boundary(
        &mut self,
        boundary: BoundaryId,
        state: &Value,
        mode: BoundaryMode,
    ) -> Result<()> {
        self.require_open("write_boundary")?;
        if !self.shell_written {
            return Err(boundary_order_error(
                "write_boundary",
                "write_shell must be called before the first boundary",
            ));
        }
        let index = boundary.index();
        if index != self.next_boundary {
            return Err(boundary_order_error(
                "write_boundary",
                "boundaries must be written once in declaration order",
            ));
        }
        let range = self
            .plan
            .get()
            .boundary(index)
            .ok_or_else(|| boundary_order_error("write_boundary", "boundary ID is out of range"))?
            .clone();
        if range.start < self.cursor {
            return Err(boundary_order_error(
                "write_boundary",
                "boundary content has already been written",
            ));
        }
        self.streaming.checkpoint_updatable = mode == BoundaryMode::Updatable;
        self.run_range(state, self.cursor..range.end)?;
        self.cursor = range.end;
        self.next_boundary += 1;
        Ok(())
    }

    /// Push a projected state patch to an already committed updatable boundary.
    pub fn update(&mut self, boundary: BoundaryId, state: &Value) -> Result<()> {
        self.require_open("update")?;
        let target = boundary.index();
        if target >= self.next_boundary {
            return Err(boundary_order_error(
                "update",
                "the target boundary has not committed yet",
            ));
        }
        self.with_context(state, |handler, context| {
            let record_sequence = context
                .streaming
                .as_ref()
                .map_or(0, |streaming| streaming.next_record_sequence);
            handler.emit_streaming_state_update(record_sequence, target, context)?;
            increment_streaming_record_sequence("state_update", super::streaming_state(context)?)
        })
    }

    /// Render the document tail, emit the terminal record, and end the writer.
    pub fn finish(mut self, state: &Value) -> Result<()> {
        if self.finished {
            return Err(boundary_order_error(
                "finish",
                "the streaming response has already finished",
            ));
        }
        if !self.shell_written {
            return Err(boundary_order_error(
                "finish",
                "write_shell must be called before finish",
            ));
        }
        if self.next_boundary != self.plan.get().boundary_count() {
            return Err(boundary_order_error(
                "finish",
                "every boundary must be committed before finish",
            ));
        }
        if !self.body_ended() {
            let fragment_count = self
                .protocol
                .protocol()
                .fragments
                .get(self.entry_id)
                .map_or(0, |fragments| fragments.fragments.len());
            self.run_range(state, self.cursor..fragment_count)?;
        }
        if !self.body_ended() {
            return Err(HandlerError::MissingStreamingBodyEnd);
        }
        self.finished = true;
        self.sink.end()
    }

    fn require_open(&self, operation: &str) -> Result<()> {
        if self.finished || self.body_ended() {
            Err(boundary_order_error(
                operation,
                "the streaming response has already finished",
            ))
        } else {
            Ok(())
        }
    }

    fn body_ended(&self) -> bool {
        self.streaming.body_ended
    }

    fn run_range(&mut self, state: &Value, range: std::ops::Range<usize>) -> Result<()> {
        let protocol: &'a Protocol = self.protocol;
        let fragments = protocol
            .protocol()
            .fragments
            .get(self.entry_id)
            .ok_or_else(|| HandlerError::MissingFragment(self.entry_id.to_string()))?;
        let entry_route = self.entry_route.take();
        let result = self.with_context(state, |handler, context| {
            handler.process_fragment_range(&fragments.fragments, range, &entry_route, context)
        });
        self.entry_route = entry_route;
        result
    }

    fn with_context<'state, T>(
        &mut self,
        state: &'state Value,
        operation: impl for<'output> FnOnce(
            &WebUIHandler,
            &mut WebUIProcessContext<'a, 'state, 'output>,
        ) -> Result<T>,
    ) -> Result<T> {
        let mut context = WebUIProcessContext {
            protocol: self.protocol.protocol(),
            legacy_structural_signals: self.protocol.legacy_structural_signals(),
            state,
            writer: &mut self.sink,
            local_vars: std::mem::take(&mut self.local_vars),
            component_attrs: std::mem::take(&mut self.component_attrs),
            request_path: self.request_path,
            route_base: std::mem::replace(&mut self.route_base, Cow::Borrowed("/")),
            rendered_components: std::mem::take(&mut self.rendered_components),
            plugin: self.plugin.take(),
            route_children: std::mem::take(&mut self.route_children),
            entry_id: self.entry_id,
            nonce: self.nonce,
            component_index: self.protocol.component_index(),
            head_inject: self.head_inject,
            body_inject: self.body_inject,
            head_end_emitted: self.head_end_emitted,
            body_end_emitted: self.body_end_emitted,
            route_index: self.protocol.route_index(),
            route_chain_index: self.route_chain_index,
            streaming: Some(&mut self.streaming),
            json_scratch: std::mem::take(&mut self.json_scratch),
            scope_pool: std::mem::take(&mut self.scope_pool),
        };

        let result = operation(self.handler, &mut context);
        self.local_vars = std::mem::take(&mut context.local_vars);
        self.component_attrs = std::mem::take(&mut context.component_attrs);
        self.route_base = std::mem::replace(&mut context.route_base, Cow::Borrowed("/"));
        self.rendered_components = std::mem::take(&mut context.rendered_components);
        self.plugin = context.plugin.take();
        self.route_children = std::mem::take(&mut context.route_children);
        self.head_end_emitted = context.head_end_emitted;
        self.body_end_emitted = context.body_end_emitted;
        self.route_chain_index = context.route_chain_index;
        self.json_scratch = std::mem::take(&mut context.json_scratch);
        self.scope_pool = std::mem::take(&mut context.scope_pool);
        result
    }
}

// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

//! Public borrowed session API and shared owned continuation state.

use std::borrow::Cow;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use serde_json::{Number, Value};

use super::error::{boundary_order_error, state_update_type_error};
use super::state::{
    increment_streaming_record_sequence, overlay_full_state, overlay_selected_state,
    selected_state_snapshot, StreamingProgress, StreamingRenderState,
};
use super::vm::{ContinuationVm, StepGoal};
use super::StreamingSink;
use crate::plugin::HandlerPlugin;
use crate::route_handler::Protocol;
use crate::{
    FlushWriter, HandlerError, RenderOptions, ResponseWriter, Result, WebUIHandler,
    WebUIProcessContext,
};

/// Maximum continuation frames retained by one response.
pub const MAX_CONTINUATION_DEPTH: usize = 256;
/// Maximum nested unfinished component spans.
pub const MAX_SPAN_NESTING: usize = 32;
/// Maximum runtime boundary occurrences in one response.
pub const MAX_BOUNDARY_OCCURRENCES: usize = 512;
/// Maximum runtime boundary occurrences one response may commit as
/// [`BoundaryMode::Updatable`].
///
/// Mirrors the browser coordinator's retained-boundary cap: the client refuses
/// to retain a 129th updatable occurrence, so the server refuses to emit one
/// rather than stream a checkpoint the page would fail on.
pub(crate) const MAX_UPDATABLE_OCCURRENCES: usize = 128;
/// Maximum keyed runtime occurrences tracked for uniqueness.
pub const MAX_KEYED_INSTANCES: usize = 512;
/// Maximum top-level state keys retained by a continuation snapshot.
pub(crate) const MAX_FROZEN_STATE_KEYS: usize = 1_024;

/// Response-local runtime boundary occurrence identity.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct BoundaryInstanceId(u32);

impl BoundaryInstanceId {
    /// Rebuild an ID round-tripped through a host binding.
    #[must_use]
    pub const fn from_raw(raw: u32) -> Self {
        Self(raw)
    }

    /// Return the wire integer.
    #[must_use]
    pub const fn raw(self) -> u32 {
        self.0
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
    /// Index of the next entry fragment to write.
    cursor: usize,
    next_boundary: usize,
    shell_written: bool,
    finished: bool,
    failed: bool,
    local_vars: HashMap<String, Value>,
    component_attrs: HashMap<String, Value>,
    route_base: Cow<'a, str>,
    rendered_components: HashSet<String>,
    document_style_resources: HashSet<String>,
    shadow_style_roots: Vec<crate::ShadowStyleRoot>,
    plugin: Option<Box<dyn HandlerPlugin>>,
    route_children: Vec<webui_protocol::WebUiFragmentRoute>,
    head_end_emitted: bool,
    body_start_emitted: bool,
    body_end_emitted: bool,
    route_chain_index: usize,
    route_chain: Option<Vec<crate::route_handler::RouteChainEntry>>,
    route_document_style_targets: Vec<bool>,
    reachable_components: Option<Vec<String>>,
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

/// The half of a [`StreamingResponse`] that carries no borrow.
///
/// A host-owned session parks this between calls and rebuilds the borrowed half
/// from its retained protocol, so a response can outlive any single `&Protocol`
/// borrow without self-referential storage. Every field here is either owned
/// outright or reduced to an owned form (`route_base`, the streaming progress)
/// precisely so that this type has no lifetime parameter.
pub(super) struct ParkedResponse {
    cursor: usize,
    next_boundary: usize,
    shell_written: bool,
    finished: bool,
    failed: bool,
    local_vars: HashMap<String, Value>,
    component_attrs: HashMap<String, Value>,
    route_base: Box<str>,
    rendered_components: HashSet<String>,
    document_style_resources: HashSet<String>,
    shadow_style_roots: Vec<crate::ShadowStyleRoot>,
    plugin: Option<Box<dyn HandlerPlugin>>,
    route_children: Vec<webui_protocol::WebUiFragmentRoute>,
    head_end_emitted: bool,
    body_start_emitted: bool,
    body_end_emitted: bool,
    route_chain_index: usize,
    route_chain: Option<Vec<crate::route_handler::RouteChainEntry>>,
    route_document_style_targets: Vec<bool>,
    reachable_components: Option<Vec<String>>,
    entry_route: Option<(String, crate::route_matcher::RouteMatch)>,
    streaming: StreamingProgress,
    json_scratch: Vec<u8>,
    scope_pool: Vec<HashMap<String, Value>>,
    /// Retained only for entries with no shared precomputed plan; shared plans
    /// are re-resolved from the protocol on every rebuild.
    request_plan: Option<StreamingEntryPlan>,
}

impl<'a, W: FlushWriter + ?Sized> StreamingResponse<'a, W> {
    /// Drop every borrow and keep the owned progress.
    pub(super) fn park(self) -> ParkedResponse {
        ParkedResponse {
            cursor: self.cursor,
            next_boundary: self.next_boundary,
            shell_written: self.shell_written,
            finished: self.finished,
            failed: self.failed,
            local_vars: self.local_vars,
            component_attrs: self.component_attrs,
            route_base: self.route_base.into_owned().into_boxed_str(),
            rendered_components: self.rendered_components,
            document_style_resources: self.document_style_resources,
            shadow_style_roots: self.shadow_style_roots,
            plugin: self.plugin,
            route_children: self.route_children,
            head_end_emitted: self.head_end_emitted,
            body_start_emitted: self.body_start_emitted,
            body_end_emitted: self.body_end_emitted,
            route_chain_index: self.route_chain_index,
            route_chain: self.route_chain,
            route_document_style_targets: self.route_document_style_targets,
            reachable_components: self.reachable_components,
            entry_route: self.entry_route,
            streaming: self.streaming.into_progress(),
            json_scratch: self.json_scratch,
            scope_pool: self.scope_pool,
            request_plan: match self.plan {
                ResponsePlan::Shared(_) => None,
                ResponsePlan::Request(plan) => Some(plan),
            },
        }
    }

    /// Rebuild a borrowed response around parked progress.
    pub(super) fn unpark(
        parked: ParkedResponse,
        handler: &'a WebUIHandler,
        protocol: &'a Protocol,
        options: &RenderOptions<'a>,
        writer: &'a mut W,
    ) -> Result<Self> {
        let plan = match parked.request_plan {
            Some(plan) => ResponsePlan::Request(plan),
            None => match protocol.streaming_plan(options.entry_id)? {
                Some(plan) => ResponsePlan::Shared(plan),
                None => {
                    return Err(HandlerError::Invariant(
                        "streaming response plan disappeared between calls".to_string(),
                    ))
                }
            },
        };
        Ok(Self {
            handler,
            protocol,
            plan,
            sink: StreamingSink { transport: writer },
            request_path: options.request_path,
            entry_id: options.entry_id,
            nonce: options.nonce.filter(|nonce| !nonce.is_empty()),
            head_inject: options.head_inject.filter(|html| !html.is_empty()),
            body_inject: options.body_inject.filter(|html| !html.is_empty()),
            cursor: parked.cursor,
            next_boundary: parked.next_boundary,
            shell_written: parked.shell_written,
            finished: parked.finished,
            failed: parked.failed,
            local_vars: parked.local_vars,
            component_attrs: parked.component_attrs,
            route_base: Cow::Owned(parked.route_base.into_string()),
            rendered_components: parked.rendered_components,
            document_style_resources: parked.document_style_resources,
            shadow_style_roots: parked.shadow_style_roots,
            plugin: parked.plugin,
            route_children: parked.route_children,
            head_end_emitted: parked.head_end_emitted,
            body_start_emitted: parked.body_start_emitted,
            body_end_emitted: parked.body_end_emitted,
            route_chain_index: parked.route_chain_index,
            route_chain: parked.route_chain,
            route_document_style_targets: parked.route_document_style_targets,
            reachable_components: parked.reachable_components,
            entry_route: parked.entry_route,
            streaming: StreamingRenderState::from_progress(
                parked.streaming,
                protocol.component_reachability(),
            ),
            json_scratch: parked.json_scratch,
            scope_pool: parked.scope_pool,
        })
    }
}

/// Response-local generated component span identity.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct SpanInstanceId(u32);

impl SpanInstanceId {
    /// Rebuild an ID round-tripped through a host binding.
    #[must_use]
    pub const fn from_raw(raw: u32) -> Self {
        Self(raw)
    }

    /// Return the wire integer.
    #[must_use]
    pub const fn raw(self) -> u32 {
        self.0
    }

    pub(crate) const fn new(raw: u32) -> Self {
        Self(raw)
    }
}

/// Valid evaluated key for a repeated boundary declaration.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub enum BoundaryKey {
    /// Authored key resolved to a JSON string.
    String(String),
    /// Authored key resolved to a finite JSON number.
    Number(Number),
}

impl BoundaryKey {
    pub(crate) fn diagnostic(&self) -> String {
        match self {
            Self::String(value) => {
                let mut out = String::with_capacity(value.len() + 2);
                out.push('"');
                out.push_str(value);
                out.push('"');
                out
            }
            Self::Number(value) => value.to_string(),
        }
    }
}

/// Runtime occurrence returned when traversal suspends.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BoundaryDescriptor {
    /// Gapless response-local occurrence ID.
    pub instance_id: BoundaryInstanceId,
    /// Stable compiler declaration ID.
    pub declaration_id: u32,
    /// Entry or component template that owns the declaration.
    ///
    /// Interned per protocol, so producing a descriptor shares the compiled
    /// string instead of allocating a copy per occurrence.
    pub owner: Arc<str>,
    /// Free-form authored declaration name.
    ///
    /// Interned per protocol alongside [`Self::owner`].
    pub name: Arc<str>,
    /// Evaluated repeat key, when authored.
    pub key: Option<BoundaryKey>,
}

/// Whether a committed occurrence may receive later state updates.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BoundaryMode {
    /// Hydrate once and release boundary-local roots.
    Final,
    /// Retain live roots until terminal for later [`StreamingResponse::update`].
    Updatable,
}

/// Borrowed-writer result of one semantic streaming step.
///
/// The pair of fields names exactly one state:
///
/// | `boundary` | `done` | meaning |
/// |------------|--------|---------|
/// | `Some`     | `false`| the occurrence is waiting for [`StreamingResponse::resume`] |
/// | `None`     | `false`| the committed occurrence flushed; call [`StreamingResponse::advance`] |
/// | `None`     | `true` | the terminal record and writer end completed |
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StreamStatus {
    /// The next occurrence waiting for [`StreamingResponse::resume`].
    pub boundary: Option<BoundaryDescriptor>,
    /// True after the terminal record and writer end completed.
    pub done: bool,
}

/// A progressive response that writes directly through a [`FlushWriter`].
///
/// Steps alternate: [`Self::start`] writes the shell prefix and stops before
/// the first occurrence, [`Self::resume`] writes exactly one occurrence through
/// its checkpoint, and [`Self::advance`] writes the ordinary parent bytes that
/// follow it until the next occurrence or the terminal. Every step is one
/// independently writable, independently flushed segment.
pub struct StreamingResponse<'a, W: FlushWriter + ?Sized> {
    handler: &'a WebUIHandler,
    protocol: &'a Protocol,
    options: RenderOptions<'a>,
    sink: StreamingSink<'a, W>,
    pub(crate) core: SessionCore,
}

impl WebUIHandler {
    /// Create a runtime-discovered progressive response.
    pub fn stream_response<'a, W: FlushWriter + ?Sized>(
        &'a self,
        protocol: &'a Protocol,
        options: &RenderOptions<'a>,
        writer: &'a mut W,
    ) -> Result<StreamingResponse<'a, W>> {
        protocol.ensure_style_metadata()?;
        let document = protocol.protocol();
        let fragments = document
            .fragments
            .get(options.entry_id)
            .ok_or_else(|| HandlerError::MissingFragment(options.entry_id.to_string()))?;
        validate_streaming_head_start(document, options.entry_id)?;
        let plan = match protocol.streaming_plan(options.entry_id)? {
            Some(plan) => ResponsePlan::Shared(plan),
            None => ResponsePlan::Request(StreamingEntryPlan::new(
                options.entry_id,
                &fragments.fragments,
                None,
            )?),
        };
        let component_count = protocol.component_index().len();
        let style_resource_count = protocol.style_resource_index().len();
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
            failed: false,
            local_vars: HashMap::new(),
            component_attrs: HashMap::new(),
            route_base: Cow::Borrowed("/"),
            rendered_components: HashSet::new(),
            document_style_resources: HashSet::new(),
            shadow_style_roots: Vec::new(),
            plugin: self.plugin_factory.map(|factory| factory()),
            route_children: Vec::new(),
            head_end_emitted: false,
            body_start_emitted: false,
            body_end_emitted: false,
            route_chain_index: 0,
            route_chain: None,
            route_document_style_targets: Vec::new(),
            reachable_components: None,
            entry_route,
            streaming: StreamingRenderState::from_progress(
                StreamingProgress::new(component_count, style_resource_count),
                protocol.component_reachability(),
            ),
            json_scratch: Vec::new(),
            scope_pool: Vec::new(),
        })
    }
}

impl<W: FlushWriter + ?Sized> StreamingResponse<'_, W> {
    /// Render until the first runtime boundary occurrence or terminal.
    pub fn start(&mut self, state: &Value) -> Result<StreamStatus> {
        let (core, call) = self.parts();
        core.start(call, state)
    }

    /// Commit the pending occurrence through its checkpoint, then stop.
    ///
    /// The bytes written by this call are exactly the occurrence's own record —
    /// no parent or tail bytes follow it — so the host can release the
    /// occurrence the moment it resolves. Call [`Self::advance`] next.
    ///
    /// [`BoundaryMode::Updatable`] is refused once the response has committed
    /// as many updatable occurrences as the browser retains. The refusal is
    /// raised before any byte or state moves, so the same occurrence stays
    /// pending and can be committed with [`BoundaryMode::Final`] instead.
    pub fn resume(
        &mut self,
        instance_id: BoundaryInstanceId,
        state: &Value,
        mode: BoundaryMode,
    ) -> Result<StreamStatus> {
        let (core, call) = self.parts();
        core.resume(call, instance_id, state, mode)
    }

    /// Write the ordinary parent bytes that follow a committed occurrence.
    ///
    /// Valid only after [`Self::resume`]. Renders the shell until the next
    /// occurrence suspends or the terminal record completes.
    pub fn advance(&mut self) -> Result<StreamStatus> {
        let (core, call) = self.parts();
        core.advance(call)
    }

    /// Emit one projected state update for a committed updatable occurrence.
    ///
    /// Valid between [`Self::resume`] and [`Self::advance`] as well as after
    /// the response has moved on, so a host can revise the occurrence it just
    /// committed while the response is still open.
    pub fn update(&mut self, instance_id: BoundaryInstanceId, patch: &Value) -> Result<()> {
        let (core, call) = self.parts();
        core.update(call, instance_id, patch)
    }

    /// Commit the pending occurrence and continue to the next one.
    ///
    /// Used by [`WebUIHandler::render_streaming`], which has no host to hand
    /// the checkpoint bytes to between the two steps and drives every
    /// occurrence from the single value it already snapshotted at start.
    pub(crate) fn resume_current_and_advance(
        &mut self,
        instance_id: BoundaryInstanceId,
        mode: BoundaryMode,
    ) -> Result<StreamStatus> {
        let (core, call) = self.parts();
        core.resume_current_and_advance(call, instance_id, mode)
    }

    /// Whether the response has emitted its terminal and ended its writer.
    #[must_use]
    pub fn is_done(&self) -> bool {
        self.core.done
    }

    /// Split the response into its continuation and one call's borrowed
    /// runtime, so every entry point shares the same wiring.
    fn parts(&mut self) -> (&mut SessionCore, SessionCall<'_, '_>) {
        let Self {
            handler,
            protocol,
            options,
            sink,
            core,
        } = self;
        (
            core,
            SessionCall {
                handler,
                protocol,
                options,
                writer: sink,
            },
        )
    }
}

pub(crate) struct SessionCore {
    pub(crate) vm: ContinuationVm,
    frozen_keys: Arc<[Box<str>]>,
    requires_full_state: bool,
    frozen_state: Value,
    started: bool,
    pub(crate) done: bool,
    failed: bool,
    /// True between a committed occurrence and the `advance` that writes the
    /// parent bytes following it.
    awaiting_advance: bool,
    local_vars: HashMap<String, Value>,
    component_attrs: HashMap<String, Value>,
    route_base: Option<String>,
    rendered_components: HashSet<String>,
    plugin: Option<Box<dyn HandlerPlugin>>,
    route_children: Vec<webui_protocol::WebUiFragmentRoute>,
    head_end_emitted: bool,
    body_start_emitted: bool,
    component_asset_styles_emitted: bool,
    body_end_emitted: bool,
    route_chain_index: usize,
    streaming: Option<StreamingProgress>,
    json_scratch: Vec<u8>,
    scope_pool: Vec<HashMap<String, Value>>,
}

pub(crate) struct SessionCall<'call, 'data> {
    pub(crate) handler: &'call WebUIHandler,
    pub(crate) protocol: &'data Protocol,
    pub(crate) options: &'call RenderOptions<'data>,
    pub(crate) writer: &'call mut dyn ResponseWriter,
}

impl SessionCore {
    pub(crate) fn new(handler: &WebUIHandler, protocol: &Protocol, entry_id: &str) -> Result<Self> {
        if !protocol.protocol().fragments.contains_key(entry_id) {
            return Err(HandlerError::MissingFragment(entry_id.to_string()));
        }
        let state_plan = protocol.continuation_state_plan(entry_id)?.resolve()?;
        Ok(Self {
            vm: ContinuationVm::new(entry_id, protocol)?,
            frozen_keys: Arc::clone(&state_plan.keys),
            requires_full_state: state_plan.requires_full_state,
            frozen_state: Value::Object(serde_json::Map::new()),
            started: false,
            done: false,
            failed: false,
            awaiting_advance: false,
            local_vars: HashMap::new(),
            component_attrs: HashMap::new(),
            route_base: None,
            rendered_components: HashSet::new(),
            plugin: handler.plugin_factory.map(|factory| factory()),
            route_children: Vec::new(),
            head_end_emitted: false,
            body_start_emitted: false,
            component_asset_styles_emitted: false,
            body_end_emitted: false,
            route_chain_index: 0,
            streaming: Some(StreamingProgress::new(protocol.component_index().len())),
            json_scratch: Vec::new(),
            scope_pool: Vec::new(),
        })
    }

    pub(crate) fn start(
        &mut self,
        call: SessionCall<'_, '_>,
        state: &Value,
    ) -> Result<StreamStatus> {
        self.require_usable("start")?;
        if self.started {
            return Err(boundary_order_error(
                "start",
                "the streaming response has already started",
            ));
        }
        // The value is moved into each render context and back, so protocols
        // requiring full projection pay for exactly one response-local clone.
        self.frozen_state = if self.requires_full_state {
            state.clone()
        } else {
            selected_state_snapshot(state, &self.frozen_keys)
        };
        self.started = true;
        self.run_step(call, StepGoal::NextBoundary, None)
    }

    pub(crate) fn resume(
        &mut self,
        call: SessionCall<'_, '_>,
        instance_id: BoundaryInstanceId,
        state: &Value,
        mode: BoundaryMode,
    ) -> Result<StreamStatus> {
        self.require_resumable()?;
        self.vm.validate_resume(instance_id)?;
        self.vm.validate_resume_mode(mode)?;
        if self.requires_full_state {
            overlay_full_state(&mut self.frozen_state, state);
        } else {
            overlay_selected_state(&mut self.frozen_state, state, &self.frozen_keys);
        }
        self.run_step(call, StepGoal::CommitBoundary, Some((instance_id, mode)))
    }

    pub(crate) fn advance(&mut self, call: SessionCall<'_, '_>) -> Result<StreamStatus> {
        self.require_advanceable()?;
        self.run_step(call, StepGoal::NextBoundary, None)
    }

    /// Commit the pending occurrence and continue to the next one in a single
    /// continuation setup.
    ///
    /// The public step machine deliberately returns at the checkpoint so a host
    /// can write and release that occurrence on its own. The one-shot
    /// [`WebUIHandler::render_streaming`] helper has no host in between, and
    /// splitting the work would cost a second continuation hand-off — every
    /// retained scope map, inventory buffer, and scratch vector moving in and
    /// back out — plus a re-resolution of the parked record the step-local
    /// cache already holds. [`StepGoal::NextBoundary`] commits the active
    /// occurrence and keeps walking, so the fused call emits exactly the bytes
    /// and flushes the split steps would.
    pub(crate) fn resume_current_and_advance(
        &mut self,
        call: SessionCall<'_, '_>,
        instance_id: BoundaryInstanceId,
        mode: BoundaryMode,
    ) -> Result<StreamStatus> {
        self.require_resumable()?;
        self.vm.validate_resume(instance_id)?;
        self.vm.validate_resume_mode(mode)?;
        self.run_step(call, StepGoal::NextBoundary, Some((instance_id, mode)))
    }

    pub(crate) fn update(
        &mut self,
        call: SessionCall<'_, '_>,
        instance_id: BoundaryInstanceId,
        patch: &Value,
    ) -> Result<()> {
        self.require_usable("update")?;
        self.require_started("update")?;
        if self.done {
            return Err(boundary_order_error(
                "update",
                "the streaming response has already completed",
            ));
        }
        if !patch.is_object() {
            return Err(state_update_type_error());
        }
        let target = self.vm.validate_update(instance_id)?;
        let handler = call.handler;
        let result = self.with_context(call, patch, |_, context| {
            let sequence = super::streaming_state(context)?.next_record_sequence;
            handler.emit_streaming_state_update(sequence, target, context)?;
            increment_streaming_record_sequence("update", super::streaming_state(context)?)
        });
        if result.is_err() {
            self.failed = true;
        }
        result
    }

    fn run_step(
        &mut self,
        call: SessionCall<'_, '_>,
        goal: StepGoal,
        resume: Option<(BoundaryInstanceId, BoundaryMode)>,
    ) -> Result<StreamStatus> {
        let state = std::mem::replace(
            &mut self.frozen_state,
            Value::Object(serde_json::Map::new()),
        );
        let handler = call.handler;
        let protocol = call.protocol;
        let result = self.with_context(call, &state, |vm, context| {
            if let Some((instance_id, mode)) = resume {
                vm.begin_resume(instance_id, mode, context)?;
            }
            let status = vm.advance(goal, handler, protocol, context)?;
            if !status.done {
                context.writer.stream_flush()?;
            }
            Ok(status)
        });
        self.frozen_state = state;
        match result {
            Ok(status) => {
                self.done = status.done;
                self.awaiting_advance = goal == StepGoal::CommitBoundary && !status.done;
                Ok(status)
            }
            Err(error) => {
                self.failed = true;
                Err(error)
            }
        }
    }

    fn with_context<'data, 'state, T>(
        &mut self,
        call: SessionCall<'_, 'data>,
        state: &'state Value,
        operation: impl for<'output> FnOnce(
            &mut ContinuationVm,
            &mut WebUIProcessContext<'data, 'state, 'output>,
        ) -> Result<T>,
    ) -> Result<T> {
        let SessionCall {
            protocol,
            options,
            writer,
            ..
        } = call;
        let component_asset_style_manifest = protocol.component_asset_style_manifest()?;
        let progress = self.streaming.take().ok_or_else(missing_progress_error)?;
        let mut streaming =
            StreamingRenderState::from_progress(progress, protocol.component_reachability());
        let mut context = WebUIProcessContext {
            protocol: protocol.protocol(),
            component_asset_style_manifest,
            component_asset_style_links: protocol.component_asset_style_links(),
            state,
            writer,
            local_vars: std::mem::take(&mut self.local_vars),
            component_attrs: std::mem::take(&mut self.component_attrs),
            request_path: options.request_path,
            route_base: self
                .route_base
                .take()
                .map_or(Cow::Borrowed("/"), Cow::Owned),
            rendered_components: std::mem::take(&mut self.rendered_components),
            plugin: self.plugin.take(),
            route_children: std::mem::take(&mut self.route_children),
            entry_id: options.entry_id,
            nonce: options.nonce.filter(|nonce| !nonce.is_empty()),
            component_index: protocol.component_index(),
            head_inject: options.head_inject.filter(|html| !html.is_empty()),
            body_inject: options.body_inject.filter(|html| !html.is_empty()),
            state_inject: crate::StateInject::resolve(state),
            head_end_emitted: self.head_end_emitted,
            body_start_emitted: self.body_start_emitted,
            component_asset_styles_emitted: self.component_asset_styles_emitted,
            body_end_emitted: self.body_end_emitted,
            route_index: protocol.route_index(),
            route_chain_index: self.route_chain_index,
            streaming: Some(&mut streaming),
            json_scratch: std::mem::take(&mut self.json_scratch),
            scope_pool: std::mem::take(&mut self.scope_pool),
        };
        let result = operation(&mut self.vm, &mut context);
        self.local_vars = std::mem::take(&mut context.local_vars);
        self.component_attrs = std::mem::take(&mut context.component_attrs);
        self.route_base = match std::mem::replace(&mut context.route_base, Cow::Borrowed("/")) {
            Cow::Owned(base) => Some(base),
            Cow::Borrowed(_) => None,
        };
        self.rendered_components = std::mem::take(&mut context.rendered_components);
        self.plugin = context.plugin.take();
        self.route_children = std::mem::take(&mut context.route_children);
        self.head_end_emitted = context.head_end_emitted;
        self.body_start_emitted = context.body_start_emitted;
        self.component_asset_styles_emitted = context.component_asset_styles_emitted;
        self.body_end_emitted = context.body_end_emitted;
        self.route_chain_index = context.route_chain_index;
        self.json_scratch = std::mem::take(&mut context.json_scratch);
        self.scope_pool = std::mem::take(&mut context.scope_pool);
        self.streaming = Some(streaming.into_progress());
        result
    }

    fn require_usable(&self, operation: &str) -> Result<()> {
        if self.failed {
            return Err(boundary_order_error(
                operation,
                "the session is poisoned by a previous render or transport failure; start a new response",
            ));
        }
        Ok(())
    }

    fn require_started(&self, operation: &str) -> Result<()> {
        if !self.started {
            return Err(boundary_order_error(
                operation,
                "start must be called before this operation",
            ));
        }
        Ok(())
    }

    /// Reject a `resume` that is out of order without writing a byte.
    ///
    /// Ordering is checked before any render work, so a rejected call leaves
    /// the response exactly as it was and the host can retry with the right
    /// instance ID or the missing `advance`.
    fn require_resumable(&self) -> Result<()> {
        self.require_usable("resume")?;
        self.require_started("resume")?;
        if self.done {
            return Err(boundary_order_error(
                "resume",
                "the streaming response has already completed",
            ));
        }
        if self.awaiting_advance {
            return Err(boundary_order_error(
                "resume",
                "the previously committed boundary has not been advanced past; call advance \
                 before resuming the next occurrence",
            ));
        }
        Ok(())
    }

    /// Reject an `advance` that is out of order without writing a byte.
    fn require_advanceable(&self) -> Result<()> {
        self.require_usable("advance")?;
        self.require_started("advance")?;
        if self.done {
            return Err(boundary_order_error(
                "advance",
                "the streaming response has already completed",
            ));
        }
        if !self.awaiting_advance {
            return Err(boundary_order_error(
                "advance",
                "there is no committed boundary to advance past; resume the pending occurrence \
                 first",
            ));
        }
        Ok(())
    }
}

#[cold]
#[inline(never)]
fn missing_progress_error() -> HandlerError {
    HandlerError::Invariant("streaming progress is unavailable".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{FlushWriter, ResponseWriter};
    use webui_parser::{ComponentRegistration, HtmlParser};
    use webui_protocol::{ComponentData, InitialStateStrategy, StateProjectionMode, WebUIProtocol};
    use webui_test_utils::test_json;

    const ISLAND_TAG: &str = "state-island";

    struct TestSink {
        output: String,
    }

    impl ResponseWriter for TestSink {
        fn write(&mut self, content: &str) -> Result<()> {
            self.output.push_str(content);
            Ok(())
        }

        fn end(&mut self) -> Result<()> {
            Ok(())
        }
    }

    impl FlushWriter for TestSink {
        fn flush(&mut self) -> Result<()> {
            Ok(())
        }
    }

    /// A sink whose bytes stay readable while the response holds it, so a test
    /// can prove a refused step wrote nothing.
    #[derive(Clone, Default)]
    struct SharedSink(std::rc::Rc<std::cell::RefCell<String>>);

    impl ResponseWriter for SharedSink {
        fn write(&mut self, content: &str) -> Result<()> {
            self.0.borrow_mut().push_str(content);
            Ok(())
        }

        fn end(&mut self) -> Result<()> {
            Ok(())
        }
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
            style_resource_index: self.protocol.style_resource_index(),
            css_strategy: self.protocol.css_strategy(),
            head_inject: self.head_inject,
            body_inject: self.body_inject,
            state_inject: crate::StateInject::resolve(state),
            head_end_emitted: self.head_end_emitted,
            body_start_emitted: self.body_start_emitted,
            body_end_emitted: self.body_end_emitted,
            route_index: self.protocol.route_index(),
            route_chain_index: self.route_chain_index,
            route_chain: std::mem::take(&mut self.route_chain),
            route_document_style_targets: std::mem::take(&mut self.route_document_style_targets),
            reachable_components: std::mem::take(&mut self.reachable_components),
            streaming: Some(&mut self.streaming),
            json_scratch: std::mem::take(&mut self.json_scratch),
            scope_pool: std::mem::take(&mut self.scope_pool),
            document_style_resources: std::mem::take(&mut self.document_style_resources),
            shadow_style_roots: std::mem::take(&mut self.shadow_style_roots),
        };

        let result = operation(self.handler, &mut context);
        self.local_vars = std::mem::take(&mut context.local_vars);
        self.component_attrs = std::mem::take(&mut context.component_attrs);
        self.route_base = std::mem::replace(&mut context.route_base, Cow::Borrowed("/"));
        self.rendered_components = std::mem::take(&mut context.rendered_components);
        self.plugin = context.plugin.take();
        self.route_children = std::mem::take(&mut context.route_children);
        self.head_end_emitted = context.head_end_emitted;
        self.body_start_emitted = context.body_start_emitted;
        self.body_end_emitted = context.body_end_emitted;
        self.route_chain_index = context.route_chain_index;
        self.route_chain = std::mem::take(&mut context.route_chain);
        self.route_document_style_targets =
            std::mem::take(&mut context.route_document_style_targets);
        self.reachable_components = std::mem::take(&mut context.reachable_components);
        self.json_scratch = std::mem::take(&mut context.json_scratch);
        self.scope_pool = std::mem::take(&mut context.scope_pool);
        if !context.shadow_style_roots.is_empty() {
            return Err(HandlerError::Invariant(
                "a Shadow CSS tree escaped its component instance".to_string(),
            ));
        }
        self.document_style_resources = std::mem::take(&mut context.document_style_resources);
        self.shadow_style_roots = std::mem::take(&mut context.shadow_style_roots);
        result
    }
}

// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

//! Public borrowed session API and shared owned continuation state.

use std::borrow::Cow;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use serde_json::{Number, Value};

use super::error::{boundary_order_error, state_update_type_error};
use super::state::{increment_state_revision, StreamingProgress, StreamingRenderState};
use super::vm::{ContinuationVm, StepGoal};
use super::StreamingSink;
use crate::plugin::HandlerPlugin;
use crate::render_scope::{
    RenderScopes, RepeatScratch, SavedSharedScope, SharedAlias, SharedBindings,
};
use crate::route_handler::Protocol;
use crate::state_view::{SharedState, StateView};
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

    pub(crate) fn index(self) -> Result<usize> {
        usize::try_from(self.0).map_err(|_| {
            HandlerError::Invariant("boundary instance ID does not fit usize".to_string())
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

/// Borrowed or owned JSON state supplied to a streaming step.
///
/// `Value`, `&Value`, and references to smart pointers such as `&Arc<Value>`
/// convert implicitly, so callers use one method name without losing ownership
/// control.
pub enum StreamingState<'a> {
    /// State retained by the caller; the session clones only its projection.
    Borrowed(&'a Value),
    /// State transferred to the session so changed subtrees can move.
    Owned(Value),
}

impl<'a, T> From<&'a T> for StreamingState<'a>
where
    T: std::borrow::Borrow<Value> + ?Sized,
{
    fn from(state: &'a T) -> Self {
        Self::Borrowed(std::borrow::Borrow::borrow(state))
    }
}

impl<'a, T> From<&'a mut T> for StreamingState<'a>
where
    T: std::borrow::Borrow<Value> + ?Sized,
{
    fn from(state: &'a mut T) -> Self {
        Self::Borrowed(std::borrow::Borrow::borrow(&*state))
    }
}

impl From<Value> for StreamingState<'_> {
    fn from(state: Value) -> Self {
        Self::Owned(state)
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
        let core = SessionCore::new(self, protocol, options.entry_id)?;
        Ok(StreamingResponse {
            handler: self,
            protocol,
            options: RenderOptions {
                entry_id: options.entry_id,
                request_path: options.request_path,
                nonce: options.nonce,
                head_inject: options.head_inject,
                body_inject: options.body_inject,
            },
            sink: StreamingSink {
                transport: writer,
                component_opening: None,
                written: 0,
                flushed: 0,
            },
            core,
        })
    }
}

impl<W: FlushWriter + ?Sized> StreamingResponse<'_, W> {
    /// Render until the first runtime boundary occurrence or terminal.
    ///
    /// Pass an owned [`Value`] to move it into the continuation, or a reference
    /// to retain caller ownership and clone only the required projection.
    pub fn start<'state>(
        &mut self,
        state: impl Into<StreamingState<'state>>,
    ) -> Result<StreamStatus> {
        let (core, call) = self.parts();
        match state.into() {
            StreamingState::Borrowed(state) => core.start(call, state),
            StreamingState::Owned(state) => core.start_owned(call, state),
        }
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
    /// Pass an owned [`Value`] to move changed subtrees, or a reference to retain
    /// caller ownership. Use [`Self::resume_current`] when no state changed.
    pub fn resume<'state>(
        &mut self,
        instance_id: BoundaryInstanceId,
        state: impl Into<StreamingState<'state>>,
        mode: BoundaryMode,
    ) -> Result<StreamStatus> {
        let (core, call) = self.parts();
        match state.into() {
            StreamingState::Borrowed(state) => core.resume(call, instance_id, state, mode),
            StreamingState::Owned(state) => core.resume_owned(call, instance_id, state, mode),
        }
    }

    /// Commit the pending occurrence against the retained state snapshot.
    ///
    /// This preserves the checkpoint flush and async pause point while avoiding
    /// an overlay when no top-level state changed since the preceding step.
    pub fn resume_current(
        &mut self,
        instance_id: BoundaryInstanceId,
        mode: BoundaryMode,
    ) -> Result<StreamStatus> {
        let (core, call) = self.parts();
        core.resume_current(call, instance_id, mode)
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

    pub(crate) fn render_all(&mut self, state: &Value) -> Result<()> {
        let (core, call) = self.parts();
        core.render_all(call, state)
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
    frozen_state: SharedState,
    started: bool,
    pub(crate) done: bool,
    failed: bool,
    /// True between a committed occurrence and the `advance` that writes the
    /// parent bytes following it.
    awaiting_advance: bool,
    local_vars: HashMap<String, Value>,
    component_attrs: HashMap<String, Value>,
    shared_alias: SharedAlias,
    shared_local_vars: SharedBindings,
    shared_component_attrs: SharedBindings,
    shared_scope_saves: Vec<SavedSharedScope>,
    route_base: Option<String>,
    rendered_components: HashSet<String>,
    document_style_resources: HashSet<String>,
    shadow_style_roots: Vec<crate::ShadowStyleRoot>,
    plugin: Option<Box<dyn HandlerPlugin>>,
    route_children: std::ops::Range<u32>,
    head_end_emitted: bool,
    body_start_emitted: bool,
    component_asset_styles_emitted: bool,
    body_end_emitted: bool,
    route_chain_index: usize,
    route_chain: Option<Vec<crate::route_handler::RouteChainEntry>>,
    route_document_style_targets: Vec<bool>,
    reachable_components: Option<Vec<String>>,
    streaming: Option<StreamingProgress>,
    json_scratch: Vec<u8>,
    attribute_buffers: crate::AttributeBuffers,
    scope_pool: Vec<HashMap<String, Value>>,
    repeat_scratch: RepeatScratch,
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
            frozen_state: SharedState::default(),
            started: false,
            done: false,
            failed: false,
            awaiting_advance: false,
            local_vars: HashMap::new(),
            component_attrs: HashMap::new(),
            shared_alias: None,
            shared_local_vars: HashMap::new(),
            shared_component_attrs: HashMap::new(),
            shared_scope_saves: Vec::new(),
            route_base: None,
            rendered_components: HashSet::new(),
            document_style_resources: HashSet::new(),
            shadow_style_roots: Vec::new(),
            plugin: handler.plugin_factory.map(|factory| factory()),
            route_children: 0..0,
            head_end_emitted: false,
            body_start_emitted: false,
            component_asset_styles_emitted: false,
            body_end_emitted: false,
            route_chain_index: 0,
            route_chain: None,
            route_document_style_targets: Vec::new(),
            reachable_components: None,
            streaming: Some(StreamingProgress::new(
                protocol.component_index().len(),
                protocol.style_resource_index().len(),
            )),
            json_scratch: Vec::new(),
            attribute_buffers: crate::AttributeBuffers::default(),
            scope_pool: Vec::new(),
            repeat_scratch: RepeatScratch::default(),
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
        // Captures share immutable roots, so nested calls never copy the
        // response-local snapshot again.
        let provenance = call.protocol.render_fragments().provenance_policy();
        self.frozen_state = if self.requires_full_state {
            SharedState::from_borrowed_with_provenance(state, provenance)
        } else {
            SharedState::from_selected_with_provenance(state, &self.frozen_keys, provenance)
        };
        self.started = true;
        self.run_step(call, StepGoal::NextBoundary, None)
    }

    pub(crate) fn start_owned(
        &mut self,
        call: SessionCall<'_, '_>,
        state: Value,
    ) -> Result<StreamStatus> {
        self.require_usable("start")?;
        if self.started {
            return Err(boundary_order_error(
                "start",
                "the streaming response has already started",
            ));
        }
        let provenance = call.protocol.render_fragments().provenance_policy();
        self.frozen_state = if self.requires_full_state {
            SharedState::from_owned_with_provenance(state, provenance)
        } else {
            SharedState::from_selected_owned_with_provenance(state, &self.frozen_keys, provenance)
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
        let keys = (!self.requires_full_state).then_some(self.frozen_keys.as_ref());
        let changed = self.frozen_state.overlay_with_provenance(
            state,
            keys,
            call.protocol.render_fragments().provenance_policy(),
        );
        self.record_state_change(changed)?;
        self.run_step(call, StepGoal::CommitBoundary, Some((instance_id, mode)))
    }

    pub(crate) fn resume_owned(
        &mut self,
        call: SessionCall<'_, '_>,
        instance_id: BoundaryInstanceId,
        state: Value,
        mode: BoundaryMode,
    ) -> Result<StreamStatus> {
        self.require_resumable()?;
        self.vm.validate_resume(instance_id)?;
        self.vm.validate_resume_mode(mode)?;
        let keys = (!self.requires_full_state).then_some(self.frozen_keys.as_ref());
        let changed = self.frozen_state.overlay_owned_with_provenance(
            state,
            keys,
            call.protocol.render_fragments().provenance_policy(),
        );
        self.record_state_change(changed)?;
        self.run_step(call, StepGoal::CommitBoundary, Some((instance_id, mode)))
    }

    pub(crate) fn resume_current(
        &mut self,
        call: SessionCall<'_, '_>,
        instance_id: BoundaryInstanceId,
        mode: BoundaryMode,
    ) -> Result<StreamStatus> {
        self.require_resumable()?;
        self.vm.validate_resume(instance_id)?;
        self.vm.validate_resume_mode(mode)?;
        self.run_step(call, StepGoal::CommitBoundary, Some((instance_id, mode)))
    }

    pub(crate) fn advance(&mut self, call: SessionCall<'_, '_>) -> Result<StreamStatus> {
        self.require_advanceable()?;
        self.run_step(call, StepGoal::NextBoundary, None)
    }

    /// Render a one-shot streaming response in one prepared process context.
    ///
    /// The synchronous helper has no async host handoff, so retaining the
    /// caller's state borrow for the complete traversal avoids both a state clone
    /// and per-boundary context teardown while preserving every transport flush.
    pub(crate) fn render_all(&mut self, call: SessionCall<'_, '_>, state: &Value) -> Result<()> {
        self.require_usable("render_streaming")?;
        if self.started {
            return Err(boundary_order_error(
                "render_streaming",
                "the streaming response has already started",
            ));
        }
        self.started = true;
        let handler = call.handler;
        let protocol = call.protocol;
        let result = self.with_context(call, state.into(), |vm, context| {
            let mut status = vm.advance(StepGoal::NextBoundary, handler, protocol, context)?;
            while !status.done {
                context.writer.stream_flush()?;
                let Some(boundary) = status.boundary.as_ref() else {
                    return Err(HandlerError::Invariant(
                        "unfinished streaming step has no pending boundary".to_string(),
                    ));
                };
                vm.validate_resume(boundary.instance_id)?;
                vm.validate_resume_mode(BoundaryMode::Final)?;
                vm.begin_resume(boundary.instance_id, BoundaryMode::Final, context)?;
                status = vm.advance(StepGoal::NextBoundary, handler, protocol, context)?;
            }
            Ok(())
        });
        self.release();
        match result {
            Ok(()) if !self.shadow_style_roots.is_empty() => {
                self.failed = true;
                Err(HandlerError::Invariant(
                    "a Shadow CSS tree escaped its component instance".to_string(),
                ))
            }
            Ok(()) => {
                self.done = true;
                Ok(())
            }
            Err(error) => {
                self.failed = true;
                Err(error)
            }
        }
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
        let result = self
            .streaming
            .as_mut()
            .ok_or_else(missing_progress_error)
            .and_then(|progress| {
                super::checkpoint::emit_streaming_state_update(
                    call,
                    progress,
                    &mut self.json_scratch,
                    target,
                    patch,
                )
            });
        if result.is_err() {
            self.failed = true;
            self.release();
        }
        result
    }

    fn run_step(
        &mut self,
        call: SessionCall<'_, '_>,
        goal: StepGoal,
        resume: Option<(BoundaryInstanceId, BoundaryMode)>,
    ) -> Result<StreamStatus> {
        let state = std::mem::take(&mut self.frozen_state);
        let handler = call.handler;
        let protocol = call.protocol;
        let result = self.with_context(call, StateView::shared(&state), |vm, context| {
            if let Some((instance_id, mode)) = resume {
                vm.begin_resume(instance_id, mode, context)?;
            }
            let status = vm.advance(goal, handler, protocol, context)?;
            if !status.done {
                context.writer.stream_flush()?;
            }
            Ok(status)
        });
        match result {
            Ok(status) => {
                if status.done && !self.shadow_style_roots.is_empty() {
                    self.failed = true;
                    self.release();
                    return Err(HandlerError::Invariant(
                        "a Shadow CSS tree escaped its component instance".to_string(),
                    ));
                }
                self.done = status.done;
                self.awaiting_advance = goal == StepGoal::CommitBoundary && !status.done;
                if status.done {
                    self.release();
                } else {
                    self.frozen_state = state;
                }
                Ok(status)
            }
            Err(error) => {
                self.failed = true;
                self.release();
                Err(error)
            }
        }
    }

    fn record_state_change(&mut self, changed: bool) -> Result<()> {
        if changed {
            let result = self
                .streaming
                .as_mut()
                .ok_or_else(missing_progress_error)
                .and_then(increment_state_revision);
            if let Err(error) = result {
                self.failed = true;
                self.release();
                return Err(error);
            }
        }
        Ok(())
    }

    fn release(&mut self) {
        self.vm.release();
        self.repeat_scratch = RepeatScratch::default();
        if let Some(progress) = &mut self.streaming {
            progress.fragment_sources.clear();
        }
        self.frozen_state.clear();
        self.shared_alias = None;
        self.shared_local_vars.clear();
        self.shared_component_attrs.clear();
        self.shared_scope_saves.clear();
        self.local_vars.clear();
        self.component_attrs.clear();
        self.scope_pool.clear();
    }

    fn with_context<'data, 'state, T>(
        &mut self,
        call: SessionCall<'_, 'data>,
        state: StateView<'state>,
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
            render_fragments: protocol.render_fragments().resolve(protocol.protocol()),
            component_asset_style_manifest,
            component_asset_style_links: protocol.component_asset_style_links(),
            state,
            writer,
            scopes: RenderScopes {
                shared_alias: self.shared_alias.take(),
                locals: std::mem::take(&mut self.shared_local_vars),
                attrs: std::mem::take(&mut self.shared_component_attrs),
                shared_saves: std::mem::take(&mut self.shared_scope_saves),
                repeats: self.repeat_scratch.take(),
                ..RenderScopes::default()
            },
            local_vars: std::mem::take(&mut self.local_vars),
            local_borrowed_vars: super::super::BorrowedScope::default(),
            loop_vars: Vec::new(),
            visible_loop_scope: super::super::VisibleLoopScope::EMPTY,
            component_attrs: std::mem::take(&mut self.component_attrs),
            component_borrowed_attrs: super::super::BorrowedScope::default(),
            collecting_component_attrs: false,
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
            style_resource_index: protocol.style_resource_index(),
            style_resources_requiring_escape: protocol.style_resources_requiring_escape(),
            style_chunk_index: protocol.protocol().style_chunk_index(),
            css_strategy: protocol.css_strategy(),
            head_inject: options.head_inject.filter(|html| !html.is_empty()),
            body_inject: options.body_inject.filter(|html| !html.is_empty()),
            state_inject: crate::StateInject::resolve(state),
            head_end_emitted: self.head_end_emitted,
            body_start_emitted: self.body_start_emitted,
            component_asset_styles_emitted: self.component_asset_styles_emitted,
            body_end_emitted: self.body_end_emitted,
            route_index: protocol.route_index(),
            route_chain_index: self.route_chain_index,
            route_chain: std::mem::take(&mut self.route_chain),
            route_document_style_targets: std::mem::take(&mut self.route_document_style_targets),
            reachable_components: std::mem::take(&mut self.reachable_components),
            streaming: Some(&mut streaming),
            json_scratch: std::mem::take(&mut self.json_scratch),
            attribute_buffers: std::mem::take(&mut self.attribute_buffers),
            scope_pool: std::mem::take(&mut self.scope_pool),
            document_style_resources: std::mem::take(&mut self.document_style_resources),
            shadow_style_roots: std::mem::take(&mut self.shadow_style_roots),
            borrowed_scope_pool: Vec::new(),
        };
        let result = operation(&mut self.vm, &mut context).and_then(|value| {
            if context.scopes.loops.is_empty() && context.scopes.repeats.is_empty() {
                Ok(value)
            } else {
                Err(suspended_loop_scope_error())
            }
        });
        if result.is_ok() {
            self.repeat_scratch
                .recycle(std::mem::take(&mut context.scopes.repeats));
        }
        self.shared_alias = context.scopes.shared_alias.take();
        self.shared_local_vars = std::mem::take(&mut context.scopes.locals);
        self.shared_component_attrs = std::mem::take(&mut context.scopes.attrs);
        self.shared_scope_saves = std::mem::take(&mut context.scopes.shared_saves);
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
        self.route_chain = std::mem::take(&mut context.route_chain);
        self.route_document_style_targets =
            std::mem::take(&mut context.route_document_style_targets);
        self.reachable_components = std::mem::take(&mut context.reachable_components);
        self.json_scratch = std::mem::take(&mut context.json_scratch);
        self.attribute_buffers = std::mem::take(&mut context.attribute_buffers);
        self.scope_pool = std::mem::take(&mut context.scope_pool);
        self.shadow_style_roots = std::mem::take(&mut context.shadow_style_roots);
        self.document_style_resources = std::mem::take(&mut context.document_style_resources);
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

#[cold]
#[inline(never)]
fn suspended_loop_scope_error() -> HandlerError {
    HandlerError::Invariant("a loop scope escaped its streaming step".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{FlushWriter, ResponseWriter};
    use webui_parser::{ComponentRegistration, HtmlParser};
    use webui_protocol::{
        ComponentData, FragmentList, InitialStateStrategy, StateProjectionMode, WebUIFragment,
        WebUIProtocol,
    };
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

    impl FlushWriter for SharedSink {
        fn flush(&mut self) -> Result<()> {
            Ok(())
        }
    }

    /// Build a parser-produced entry with `boundaries` runtime occurrences,
    /// each hosting one island component.
    fn boundary_protocol(boundaries: usize, hydration_mode: StateProjectionMode) -> Protocol {
        let mut html = String::from("<!doctype html><html><head></head><body>");
        for sequence in 0..boundaries {
            html.push_str("<boundary name=\"b");
            html.push_str(&sequence.to_string());
            html.push_str("\"><article><");
            html.push_str(ISLAND_TAG);
            html.push_str("></");
            html.push_str(ISLAND_TAG);
            html.push_str("></article></boundary>");
        }
        html.push_str("</body></html>");

        let mut parser = HtmlParser::new();
        match parser
            .component_registry_mut()
            .register_component(ComponentRegistration::new(
                ISLAND_TAG,
                "<button>{{title}}</button>",
                None,
                true,
            )) {
            Ok(()) => {}
            Err(error) => panic!("registering the island failed: {error}"),
        }
        if let Err(error) = parser.parse("index.html", &html) {
            panic!("parsing the streaming entry failed: {error}");
        }
        let mut document = WebUIProtocol::new(parser.into_fragment_records());
        document.initial_state_strategy = InitialStateStrategy::Components as i32;
        document.components.insert(
            ISLAND_TAG.to_string(),
            ComponentData {
                template_json: r#"{"h":"<button></button>","th":1}"#.to_string(),
                uses_shadow_dom: true,
                hydration_mode: hydration_mode as i32,
                hydration_keys: if matches!(hydration_mode, StateProjectionMode::Keys) {
                    vec!["count".to_string(), "title".to_string()]
                } else {
                    Vec::new()
                },
                ..Default::default()
            },
        );
        document.populate_style_closures(&["index.html"]);
        Protocol::new(document)
    }

    fn render_boundary_protocol() -> Protocol {
        let mut parser = HtmlParser::new();
        if let Err(error) = parser.parse(
            "body.html",
            concat!(
                "<!doctype html><html><head></head><body>",
                "<boundary name=\"first\"><p>{{selected.title}}</p></boundary>",
                "<boundary name=\"second\"><p>{{selected.title}}</p></boundary>",
                "<footer>{{selected.title}}</footer></body></html>",
            ),
        ) {
            panic!("parsing the streaming fragment failed: {error}");
        }
        let mut records = parser.into_fragment_records();
        for (id, target, scope, alias) in [
            ("index.html", "outer", "payload", "outer"),
            ("outer", "body.html", "outer", "selected"),
        ] {
            records.insert(
                id.to_string(),
                FragmentList {
                    fragments: vec![WebUIFragment::render(target, scope, alias)],
                    contains_boundary: true,
                },
            );
        }
        let mut document = WebUIProtocol::new(records);
        document.populate_style_closures(&["index.html"]);
        Protocol::new(document)
    }

    fn owner_prop_boundary_protocol() -> Protocol {
        let mut parser = HtmlParser::new();
        parser
            .component_registry_mut()
            .register_component(ComponentRegistration::new(
                ISLAND_TAG,
                concat!(
                    "<boundary name=\"first\"><p>{{model.title}}/{{label}}</p></boundary>",
                    "<boundary name=\"second\"><p>{{model.title}}/{{label}}</p></boundary>",
                ),
                None,
                true,
            ))
            .unwrap_or_else(|error| panic!("registering the owner failed: {error}"));
        parser
            .parse(
                "index.html",
                concat!(
                    "<!doctype html><html><head></head><body>",
                    "<state-island :model=\"{{payload}}\" label=\"prefix-{{title}}\">",
                    "</state-island></body></html>",
                ),
            )
            .unwrap_or_else(|error| panic!("parsing the owner entry failed: {error}"));
        let mut document = WebUIProtocol::new(parser.into_fragment_records());
        document.components.insert(
            ISLAND_TAG.to_owned(),
            ComponentData {
                uses_shadow_dom: true,
                hydration_mode: StateProjectionMode::All as i32,
                ..Default::default()
            },
        );
        document.populate_style_closures(&["index.html"]);
        Protocol::new(document)
    }

    /// A state whose payload is large enough that a per-boundary copy would be
    /// unmistakable in both time and allocation.
    fn large_state(rows: usize) -> Value {
        let mut items = Vec::with_capacity(rows);
        for row in 0..rows {
            items.push(test_json!({
                "id": row,
                "label": format!("row-{row}"),
                "tags": ["alpha", "beta", "gamma"],
            }));
        }
        test_json!({
            "count": 42,
            "title": "large state",
            "rows": items,
        })
    }

    /// Heap address of the retained snapshot's `rows` buffer, or `None` when
    /// the snapshot does not hold it.
    fn rows_address(state: StateView<'_>) -> Option<usize> {
        state
            .get("rows")
            .and_then(Value::as_array)
            .map(|rows| rows.as_ptr().addr())
    }

    fn options<'a>() -> RenderOptions<'a> {
        RenderOptions::new("index.html", "/")
    }

    #[test]
    fn active_repeat_cannot_escape_a_step_and_releases_shared_input_on_error() -> Result<()> {
        use crate::render_scope::{RepeatFrame, RepeatSource};
        use crate::state_view::SharedValue;

        let protocol = boundary_protocol(1, StateProjectionMode::Keys);
        let handler = WebUIHandler::new();
        let render_options = options();
        let declaration = webui_protocol::WebUIFragmentFor {
            item: "item".to_owned(),
            collection: "items".to_owned(),
            fragment_id: "body".to_owned(),
        };
        let values = [test_json!({"value": "borrowed"})];
        let state = test_json!({});
        for shared in [false, true] {
            for fail in [false, true] {
                let mut core = SessionCore::new(&handler, &protocol, "index.html")?;
                let mut sink = TestSink {
                    output: String::new(),
                };
                let items = if shared {
                    RepeatSource::Shared(SharedValue::new(test_json!([{"value": "shared"}])))
                } else {
                    RepeatSource::Borrowed(&values)
                };
                let origin = match &items {
                    RepeatSource::Shared(value) => Some(Arc::downgrade(value.origin())),
                    RepeatSource::Borrowed(_) => None,
                };
                let result = core.with_context(
                    SessionCall {
                        handler: &handler,
                        protocol: &protocol,
                        options: &render_options,
                        writer: &mut sink,
                    },
                    (&state).into(),
                    |_, context| {
                        context.scopes.repeats.push(RepeatFrame {
                            slot: 0,
                            declaration: &declaration,
                            items,
                            index: 0,
                            saved_value: None,
                            visible: crate::VisibleLoopScope::EMPTY,
                        });
                        if fail {
                            Err(HandlerError::Invariant("test failure".to_owned()))
                        } else {
                            Ok(())
                        }
                    },
                );
                let Err(error) = result else {
                    panic!("a live repeat must not escape a successful host step");
                };
                let expected = if fail {
                    HandlerError::Invariant("test failure".to_owned())
                } else {
                    suspended_loop_scope_error()
                };
                assert_eq!(error.to_string(), expected.to_string());
                if let Some(origin) = origin {
                    assert!(origin.upgrade().is_none(), "repeat input must be released");
                }
            }
        }
        Ok(())
    }

    #[test]
    fn nested_render_aliases_share_snapshot_and_survive_root_replacement() -> Result<()> {
        let protocol = render_boundary_protocol();
        let handler = WebUIHandler::new();
        let render_options = options();
        for owned in [false, true] {
            let state = test_json!({ "payload": large_state(64) });
            let input_address = state
                .get("payload")
                .and_then(|payload| rows_address(payload.into()));
            let mut sink = TestSink {
                output: String::new(),
            };
            let mut response = handler.stream_response(&protocol, &render_options, &mut sink)?;
            let status = if owned {
                response.start(state)?
            } else {
                response.start(&state)?
            };
            let snapshot_address = response
                .core
                .frozen_state
                .get("payload")
                .and_then(|payload| rows_address(payload.into()));
            assert!(snapshot_address.is_some());
            if owned {
                assert_eq!(snapshot_address, input_address, "owned roots must move");
            } else {
                assert_ne!(snapshot_address, input_address, "borrowed roots clone once");
            }
            let Some((alias, selected)) = response.core.shared_alias.as_ref() else {
                panic!("the nested render must retain its selected input");
            };
            assert_eq!(alias.as_ref(), "selected");
            assert_eq!(
                rows_address(selected.get().into()),
                snapshot_address,
                "nested captures must share the retained root"
            );
            let Some(boundary) = status.boundary else {
                panic!("the first boundary must suspend");
            };
            let replacement = test_json!({ "payload": { "title": "replacement", "rows": [] } });
            response.resume(boundary.instance_id, replacement, BoundaryMode::Final)?;
            assert_ne!(
                response
                    .core
                    .frozen_state
                    .get("payload")
                    .and_then(|payload| rows_address(payload.into())),
                snapshot_address,
                "the owner root must be replaced"
            );
            assert_eq!(
                response
                    .core
                    .shared_alias
                    .as_ref()
                    .and_then(|(_, selected)| rows_address(selected.get().into())),
                snapshot_address,
                "an active selection must keep its original root"
            );
            let Some(boundary) = response.advance()?.boundary else {
                panic!("the second boundary must suspend");
            };
            response.resume_current(boundary.instance_id, BoundaryMode::Final)?;
            assert!(response.advance()?.done);
            assert!(response.core.frozen_state.get("payload").is_none());
            assert!(response.core.shared_alias.is_none());
            assert!(response.core.shared_local_vars.is_empty());
            assert!(response.core.shared_component_attrs.is_empty());
            assert!(response.core.shared_scope_saves.is_empty());
            assert_eq!(sink.output.matches("<p>large state</p>").count(), 2);
            assert!(sink.output.contains("<footer>large state</footer>"));
        }
        Ok(())
    }

    #[test]
    fn no_render_owner_props_keep_original_structured_snapshots_across_resumes() -> Result<()> {
        let protocol = owner_prop_boundary_protocol();
        assert_eq!(
            protocol.render_fragments().provenance_policy(),
            crate::state_view::Provenance::Omit
        );
        let handler = WebUIHandler::with_plugin(|| {
            Box::new(crate::plugin::webui::WebUIHydrationPlugin::new())
        });
        let render_options = options();
        for owned in [false, true] {
            let state = test_json!({"payload": large_state(4), "title": "before"});
            let input = rows_address((&state["payload"]).into());
            let mut sink = TestSink {
                output: String::new(),
            };
            let mut response = handler.stream_response(&protocol, &render_options, &mut sink)?;
            let start = if owned {
                response.start(state)?
            } else {
                response.start(&state)?
            };
            let model = &response.core.shared_local_vars["model"];
            let captured = rows_address(model.get().into());
            assert!(captured.is_some());
            assert_eq!(captured == input, owned);
            assert!(model.provenance().is_none());
            assert!(response.core.shared_local_vars["label"]
                .provenance()
                .is_none());
            let Some(first) = start.boundary else {
                panic!("the owner must suspend at its first boundary");
            };
            let replacement = test_json!({
                "payload": {"title": "replacement", "rows": []},
                "title": "after"
            });
            if owned {
                response.resume(first.instance_id, replacement, BoundaryMode::Final)?;
            } else {
                response.resume(first.instance_id, &replacement, BoundaryMode::Final)?;
            }
            assert_eq!(
                rows_address(response.core.shared_local_vars["model"].get().into()),
                captured
            );
            assert_ne!(
                response
                    .core
                    .frozen_state
                    .get("payload")
                    .and_then(|value| rows_address(value.into())),
                captured
            );
            let Some(second) = response.advance()?.boundary else {
                panic!("the owner must suspend at its second boundary");
            };
            response.resume_current(second.instance_id, BoundaryMode::Final)?;
            assert!(response.advance()?.done);
            assert!(response.core.shared_local_vars.is_empty());
            assert_eq!(sink.output.matches("large state/prefix-before").count(), 2);
            assert!(!sink.output.contains("fragmentSources"));
            let span = sink
                .output
                .split("<script type=\"application/json\" data-webui-boundary>")
                .skip(1)
                .filter_map(|script| script.split("</script>").next())
                .map(|record| {
                    serde_json::from_str::<Value>(record)
                        .unwrap_or_else(|error| panic!("invalid stream record: {error}"))
                })
                .find(|record| record[1] == 3)
                .unwrap_or_else(|| panic!("the owner must emit its completion record"));
            assert_eq!(span[3]["state"]["model"], large_state(4));
            assert_eq!(span[3]["state"]["label"], "prefix-before");
            assert_eq!(span[3]["state"]["payload"]["title"], "replacement");
        }
        Ok(())
    }

    #[test]
    fn capture_promotions_synthetic_lengths_and_owner_fallbacks_follow_protocol_policy(
    ) -> Result<()> {
        use crate::state_view::{Provenance, SharedValue};

        let handler = WebUIHandler::new();
        let render_options = options();
        for policy in [Provenance::Omit, Provenance::Track] {
            let protocol = if policy == Provenance::Track {
                render_boundary_protocol()
            } else {
                boundary_protocol(1, StateProjectionMode::Keys)
            };
            let mut core = SessionCore::new(&handler, &protocol, "index.html")?;
            let state = test_json!({"rows": [1, 2, 3]});
            let mut sink = TestSink {
                output: String::new(),
            };
            core.with_context(
                SessionCall {
                    handler: &handler,
                    protocol: &protocol,
                    options: &render_options,
                    writer: &mut sink,
                },
                (&state).into(),
                |_, context| {
                    let owned = test_json!({"child": [1, 2]});
                    let child = &owned["child"] as *const Value;
                    context.local_vars.insert("prop".to_owned(), owned);
                    let captured = crate::render_scope::capture("prop.child", context)
                        .unwrap_or_else(|| panic!("owned props must be promoted"));
                    assert!(std::ptr::eq(captured.get(), child));
                    let length = crate::render_scope::capture("rows.length", context)
                        .unwrap_or_else(|| panic!("synthetic lengths must be captured"));
                    assert_eq!(length.get(), 3);
                    for value in [&captured, &length, &context.scopes.locals["prop"]] {
                        assert_eq!(value.provenance().is_some(), policy == Provenance::Track);
                    }
                    assert!(!context.local_vars.contains_key("prop"));
                    Ok(())
                },
            )?;
            let shared = SharedState::from_owned_with_provenance(
                test_json!({"item": {"name": "owner"}}),
                policy,
            );
            core.with_context(
                SessionCall {
                    handler: &handler,
                    protocol: &protocol,
                    options: &render_options,
                    writer: &mut sink,
                },
                StateView::shared(&shared),
                |_, context| {
                    context.scopes.loops.insert(
                        "item".to_owned(),
                        SharedValue::with_provenance(test_json!({"missing": 1}), policy),
                    );
                    let fallback = crate::render_scope::capture("item.name", context)
                        .unwrap_or_else(|| panic!("missing loop fields fall back to owner state"));
                    assert_eq!(fallback.get(), "owner");
                    assert_eq!(fallback.provenance().is_some(), policy == Provenance::Track);
                    context.scopes.loops.clear();
                    Ok(())
                },
            )?;
        }
        Ok(())
    }

    #[test]
    fn rejected_resume_preserves_roots_but_transport_failure_releases_them() -> Result<()> {
        #[derive(Default)]
        struct DisconnectSink(std::rc::Rc<std::cell::Cell<bool>>);

        impl ResponseWriter for DisconnectSink {
            fn write(&mut self, _: &str) -> Result<()> {
                if self.0.get() {
                    Err(HandlerError::ClientDisconnected)
                } else {
                    Ok(())
                }
            }

            fn end(&mut self) -> Result<()> {
                Ok(())
            }
        }

        impl FlushWriter for DisconnectSink {
            fn flush(&mut self) -> Result<()> {
                Ok(())
            }
        }

        let protocol = render_boundary_protocol();
        let handler = WebUIHandler::new();
        let render_options = options();
        let mut sink = DisconnectSink::default();
        let disconnected = std::rc::Rc::clone(&sink.0);
        let mut response = handler.stream_response(&protocol, &render_options, &mut sink)?;
        let status = response.start(test_json!({ "payload": large_state(64) }))?;
        let snapshot = response
            .core
            .shared_alias
            .as_ref()
            .and_then(|(_, selected)| rows_address(selected.get().into()));
        assert!(snapshot.is_some());
        let Some(boundary) = status.boundary else {
            panic!("the first boundary must suspend");
        };
        assert!(response
            .resume(
                BoundaryInstanceId::from_raw(u32::MAX),
                test_json!({ "payload": null }),
                BoundaryMode::Final,
            )
            .is_err());
        assert!(!response.core.failed);
        assert_eq!(
            response
                .core
                .frozen_state
                .get("payload")
                .and_then(|payload| rows_address(payload.into())),
            snapshot
        );
        disconnected.set(true);
        assert!(matches!(
            response.resume_current(boundary.instance_id, BoundaryMode::Final),
            Err(HandlerError::ClientDisconnected)
        ));
        assert!(response.core.failed);
        assert!(!response.is_done());
        assert!(response.core.frozen_state.get("payload").is_none());
        assert!(response.core.shared_alias.is_none());
        assert!(response.core.shared_local_vars.is_empty());
        assert!(response.core.shared_component_attrs.is_empty());
        assert!(response.core.shared_scope_saves.is_empty());
        assert!(response
            .resume_current(boundary.instance_id, BoundaryMode::Final)
            .is_err());
        Ok(())
    }

    #[test]
    fn session_parks_route_addresses_and_retains_scope_pool() -> Result<()> {
        let protocol = boundary_protocol(2, StateProjectionMode::Keys);
        let handler = WebUIHandler::new();
        let state = test_json!({ "count": 3, "title": "pooled" });
        let render_options = options();
        let mut sink = TestSink {
            output: String::new(),
        };
        let mut response = handler.stream_response(&protocol, &render_options, &mut sink)?;

        let first = response.start(&state)?;
        assert!(response.core.route_children.is_empty());
        let Some(boundary) = first.boundary else {
            panic!("the first boundary should suspend");
        };

        response.resume(boundary.instance_id, &state, BoundaryMode::Final)?;
        assert!(response.core.route_children.is_empty());
        let pooled_capacity = response
            .core
            .scope_pool
            .first()
            .map(HashMap::capacity)
            .unwrap_or_else(|| panic!("the completed component should recycle its scope map"));

        response.advance()?;
        assert!(response.core.route_children.is_empty());
        assert_eq!(
            response.core.scope_pool.first().map(HashMap::capacity),
            Some(pooled_capacity),
            "a suspension step must preserve the scope-map pool"
        );
        Ok(())
    }

    #[test]
    fn session_reuses_only_empty_repeat_capacity_and_releases_it_on_completion() -> Result<()> {
        let mut parser = HtmlParser::new();
        parser
            .parse(
                "index.html",
                concat!(
                    "<!doctype html><html><head></head><body>",
                    "<boundary name=\"first\"><for each=\"item in rows\"><p>{{item}}</p></for></boundary>",
                    "<boundary name=\"second\"><for each=\"item in rows\"><p>{{item}}</p></for></boundary>",
                    "</body></html>",
                ),
            )
            .unwrap_or_else(|error| panic!("parsing repeats failed: {error}"));
        let mut document = WebUIProtocol::new(parser.into_fragment_records());
        document.populate_style_closures(&["index.html"]);
        let protocol = Protocol::new(document);
        let handler = WebUIHandler::new();
        let render_options = options();
        let mut sink = TestSink {
            output: String::new(),
        };
        let mut response = handler.stream_response(&protocol, &render_options, &mut sink)?;
        let mut status = response.start(test_json!({"rows": ["one", "two"]}))?;
        let mut allocation = None;
        for _ in 0..2 {
            let Some(boundary) = status.boundary else {
                panic!("the next boundary must suspend");
            };
            response.resume_current(boundary.instance_id, BoundaryMode::Final)?;
            let frames = response.core.repeat_scratch.take();
            assert!(frames.is_empty());
            assert!(frames.capacity() > 0);
            let current = (frames.as_ptr().addr(), frames.capacity());
            if let Some(previous) = allocation {
                assert_eq!(current, previous, "repeat storage must survive host steps");
            }
            allocation = Some(current);
            response.core.repeat_scratch.recycle(frames);
            status = response.advance()?;
        }
        assert!(status.done);
        assert_eq!(response.core.repeat_scratch.take().capacity(), 0);
        assert_eq!(sink.output.matches("<p>one</p><p>two</p>").count(), 2);
        Ok(())
    }

    #[test]
    fn session_failure_releases_previously_retained_repeat_capacity() -> Result<()> {
        let protocol = boundary_protocol(1, StateProjectionMode::Keys);
        let handler = WebUIHandler::new();
        let render_options = options();
        let mut core = SessionCore::new(&handler, &protocol, "index.html")?;
        let frames = Vec::with_capacity(2);
        core.repeat_scratch.recycle(frames);
        let retained = core.repeat_scratch.take();
        assert_eq!(retained.capacity(), 2);
        core.repeat_scratch.recycle(retained);
        let mut sink = TestSink {
            output: String::new(),
        };
        let state = test_json!({});
        let result: Result<()> = core.with_context(
            SessionCall {
                handler: &handler,
                protocol: &protocol,
                options: &render_options,
                writer: &mut sink,
            },
            (&state).into(),
            |_, context| {
                assert_eq!(context.scopes.repeats.capacity(), 2);
                Err(HandlerError::ClientDisconnected)
            },
        );
        assert!(matches!(result, Err(HandlerError::ClientDisconnected)));
        assert_eq!(core.repeat_scratch.take().capacity(), 0);
        Ok(())
    }

    #[test]
    fn render_streaming_projects_full_state_once_per_response() -> Result<()> {
        // Full-state protocols retain the caller's whole tree. Committing each
        // occurrence against the snapshot the response already holds must not
        // re-copy that tree, so the retained buffer keeps its identity for the
        // entire response no matter how many boundaries commit.
        let protocol = boundary_protocol(8, StateProjectionMode::All);
        let handler = WebUIHandler::new();
        let state = large_state(256);
        let render_options = options();
        let mut sink = TestSink {
            output: String::new(),
        };
        let mut response = handler.stream_response(&protocol, &render_options, &mut sink)?;

        let mut status = response.start(&state)?;
        let snapshot = rows_address(StateView::shared(&response.core.frozen_state));
        assert!(
            snapshot.is_some(),
            "a full-state protocol must retain the caller's payload"
        );
        assert_ne!(
            snapshot,
            rows_address((&state).into()),
            "the response owns its snapshot rather than borrowing the caller's tree"
        );

        let mut committed = 0usize;
        while !status.done {
            status = match status.boundary.as_ref() {
                Some(boundary) => {
                    let next =
                        response.resume(boundary.instance_id, &state, BoundaryMode::Final)?;
                    committed += 1;
                    assert!(
                        next.boundary.is_none() && !next.done,
                        "a commit step stops at its checkpoint and waits for advance"
                    );
                    assert_eq!(
                        rows_address(StateView::shared(&response.core.frozen_state)),
                        snapshot,
                        "committing occurrence {committed} must not re-copy the retained snapshot"
                    );
                    next
                }
                None => response.advance()?,
            };
        }
        assert_eq!(committed, 8, "every authored boundary must commit");

        // The one-shot helper fuses commit and advance into one continuation
        // setup, so its bytes must match the split steps it stands in for.
        let mut helper_sink = TestSink {
            output: String::new(),
        };
        handler.render_streaming(&protocol, &state, &render_options, &mut helper_sink)?;
        assert_eq!(
            helper_sink.output, sink.output,
            "render_streaming must resume against the retained snapshot"
        );
        Ok(())
    }

    #[test]
    fn fused_helper_matches_split_step_bytes_and_flushes() -> Result<()> {
        // `render_streaming` skips the host hand-off between a commit and the
        // parent bytes that follow it. That is a scheduling shortcut only: the
        // emitted bytes and the positions at which they are flushed must match
        // the split step machine exactly.
        #[derive(Default)]
        struct FlushSink {
            output: String,
            flushes: Vec<usize>,
        }

        impl ResponseWriter for FlushSink {
            fn write(&mut self, content: &str) -> Result<()> {
                self.output.push_str(content);
                Ok(())
            }

            fn end(&mut self) -> Result<()> {
                Ok(())
            }
        }

        impl FlushWriter for FlushSink {
            fn flush(&mut self) -> Result<()> {
                self.flushes.push(self.output.len());
                Ok(())
            }
        }

        let protocol = boundary_protocol(4, StateProjectionMode::Keys);
        let handler = WebUIHandler::new();
        let state = test_json!({ "count": 3, "title": "fused" });
        let render_options = options();

        let mut split = FlushSink::default();
        {
            let mut response = handler.stream_response(&protocol, &render_options, &mut split)?;
            let mut status = response.start(&state)?;
            while !status.done {
                status = match status.boundary.as_ref() {
                    Some(boundary) => {
                        response.resume(boundary.instance_id, &state, BoundaryMode::Final)?
                    }
                    None => response.advance()?,
                };
            }
        }

        let mut fused = FlushSink::default();
        handler.render_streaming(&protocol, &state, &render_options, &mut fused)?;

        assert_eq!(fused.output, split.output, "bytes must match");
        assert_eq!(fused.flushes, split.flushes, "flush positions must match");
        Ok(())
    }

    #[test]
    fn resume_overlays_new_state_and_reuses_unchanged_subtrees() -> Result<()> {
        // The public resume keeps its patch semantics: a changed key lands in
        // the snapshot, an omitted key survives, and an unchanged subtree is
        // left in place instead of being copied again.
        let protocol = boundary_protocol(2, StateProjectionMode::All);
        let handler = WebUIHandler::new();
        let state = large_state(64);
        let render_options = options();
        let mut sink = TestSink {
            output: String::new(),
        };
        let mut response = handler.stream_response(&protocol, &render_options, &mut sink)?;

        let status = response.start(&state)?;
        let snapshot = rows_address(StateView::shared(&response.core.frozen_state));
        let Some(boundary) = status.boundary.as_ref() else {
            panic!("the first occurrence must suspend");
        };

        let mut next = state.clone();
        if let Some(object) = next.as_object_mut() {
            object.insert("title".to_string(), Value::String("second".to_string()));
            object.remove("count");
        }
        let status = response.resume(boundary.instance_id, &next, BoundaryMode::Final)?;

        assert_eq!(
            response.core.frozen_state.get("title"),
            Some(&Value::String("second".to_string())),
            "a changed key must land in the snapshot"
        );
        assert_eq!(
            response.core.frozen_state.get("count"),
            Some(&test_json!(42)),
            "an omitted key keeps the value the snapshot already holds"
        );
        assert_eq!(
            rows_address(StateView::shared(&response.core.frozen_state)),
            snapshot,
            "an unchanged subtree must not be copied again"
        );
        assert!(
            status.boundary.is_none() && !status.done,
            "the commit step stops at its checkpoint"
        );
        assert!(
            response.advance()?.boundary.is_some(),
            "the second occurrence follows the parent bytes"
        );
        Ok(())
    }

    #[test]
    fn semantic_steps_reuse_projection_scratch() -> Result<()> {
        // The record projection scratch lives in the retained progress as plain
        // integers, so after the first record every later step reuses the same
        // allocation instead of rebuilding one per step.
        let protocol = boundary_protocol(6, StateProjectionMode::Keys);
        let handler = WebUIHandler::new();
        let state = test_json!({ "count": 1, "title": "scratch", "unused": "x" });
        let render_options = options();
        let mut sink = TestSink {
            output: String::new(),
        };
        let mut response = handler.stream_response(&protocol, &render_options, &mut sink)?;

        let mut status = response.start(&state)?;
        let mut retained: Option<(usize, usize)> = None;
        while !status.done {
            let Some(boundary) = status.boundary.as_ref() else {
                status = response.advance()?;
                continue;
            };
            status = response.resume(boundary.instance_id, &state, BoundaryMode::Updatable)?;
            let Some(progress) = response.core.streaming.as_ref() else {
                panic!("a suspended response must retain its progress");
            };
            let observed = (
                progress.state_key_ids.capacity(),
                progress.state_key_ids.as_ptr().addr(),
            );
            assert!(
                observed.0 > 0,
                "the projection scratch must survive the record that filled it"
            );
            match retained {
                None => retained = Some(observed),
                Some(previous) => assert_eq!(
                    observed, previous,
                    "a later step must reuse the projection buffer, not allocate a new one"
                ),
            }
        }
        Ok(())
    }

    #[test]
    fn updates_reuse_their_committed_projection_buffer() -> Result<()> {
        // An update writes through the plan captured at commit time: no key
        // list is rebuilt, so the plan's buffer keeps its identity across every
        // update it serves.
        let protocol = boundary_protocol(2, StateProjectionMode::Keys);
        let handler = WebUIHandler::new();
        let state = test_json!({ "count": 1, "title": "updates" });
        let render_options = options();
        let mut sink = TestSink {
            output: String::new(),
        };
        let mut response = handler.stream_response(&protocol, &render_options, &mut sink)?;

        let status = response.start(&state)?;
        let Some(boundary) = status.boundary.as_ref() else {
            panic!("the first occurrence must suspend");
        };
        let instance_id = boundary.instance_id;
        response.resume(instance_id, &state, BoundaryMode::Updatable)?;

        let mut retained: Option<(usize, usize)> = None;
        for _ in 0..4 {
            response.update(instance_id, &state)?;
            let Some(progress) = response.core.streaming.as_ref() else {
                panic!("a live response must retain its progress");
            };
            let Some(Some(plan)) = progress.update_plans.first() else {
                panic!("an updatable occurrence must retain its projection plan");
            };
            let observed = (plan.key_ids.capacity(), plan.key_ids.as_ptr().addr());
            assert!(!plan.requires_full_state, "keyed islands project keys");
            assert!(observed.0 > 0, "the plan must retain its key buffer");
            match retained {
                None => retained = Some(observed),
                Some(previous) => assert_eq!(
                    observed, previous,
                    "every update must reuse the committed projection buffer"
                ),
            }
        }
        assert_eq!(
            sink.output.matches(",2,0,{").count(),
            4,
            "each update emits exactly one typed state-update record"
        );
        Ok(())
    }

    #[test]
    fn state_only_updates_preserve_nonce_projection_and_the_render_snapshot() -> Result<()> {
        for mode in [StateProjectionMode::Keys, StateProjectionMode::All] {
            let protocol = boundary_protocol(2, mode);
            let handler = WebUIHandler::new();
            let render_options = options().with_nonce("a\"&b");
            let state = test_json!({"count": 1, "title": "original"});
            let patch = test_json!({"count": 2, "title": "updated", "extra": true});
            let mut sink = SharedSink::default();
            let output = std::rc::Rc::clone(&sink.0);
            let mut response = handler.stream_response(&protocol, &render_options, &mut sink)?;
            let Some(boundary) = response.start(&state)?.boundary else {
                panic!("the first boundary must suspend");
            };
            response.resume_current(boundary.instance_id, BoundaryMode::Updatable)?;
            let start = output.borrow().len();
            response.update(boundary.instance_id, &patch)?;
            {
                let output = output.borrow();
                let update = &output[start..];
                assert!(update.contains(" nonce=\"a&quot;&amp;b\">[1,2,0,"));
                assert!(update.contains("\"count\":2"));
                assert!(update.contains("\"title\":\"updated\""));
                assert_eq!(
                    update.contains("\"extra\":true"),
                    mode == StateProjectionMode::All,
                );
            }
            assert_eq!(response.core.frozen_state.get("count"), state.get("count"));
            assert_eq!(response.core.frozen_state.get("title"), state.get("title"));
            let Some(second) = response.advance()?.boundary else {
                panic!("the second boundary must suspend");
            };
            response.resume_current(second.instance_id, BoundaryMode::Final)?;
            assert!(response.advance()?.done);
            assert!(output.borrow().contains("<button>original</button>"));
        }
        Ok(())
    }

    #[test]
    fn state_only_update_write_and_flush_errors_poison_and_release_the_session() -> Result<()> {
        #[derive(Default)]
        struct UpdateSink(std::rc::Rc<std::cell::Cell<Option<bool>>>);

        impl ResponseWriter for UpdateSink {
            fn write(&mut self, _: &str) -> Result<()> {
                if self.0.get() == Some(true) {
                    Err(HandlerError::ClientDisconnected)
                } else {
                    Ok(())
                }
            }

            fn end(&mut self) -> Result<()> {
                Ok(())
            }
        }

        impl FlushWriter for UpdateSink {
            fn flush(&mut self) -> Result<()> {
                if self.0.get() == Some(false) {
                    Err(HandlerError::ClientDisconnected)
                } else {
                    Ok(())
                }
            }
        }

        let protocol = boundary_protocol(2, StateProjectionMode::Keys);
        let handler = WebUIHandler::new();
        let render_options = options();
        for fail_on_write in [true, false] {
            let mut sink = UpdateSink::default();
            let failure = std::rc::Rc::clone(&sink.0);
            let mut response = handler.stream_response(&protocol, &render_options, &mut sink)?;
            let state = test_json!({"count": 1, "title": "retained"});
            let Some(boundary) = response.start(&state)?.boundary else {
                panic!("the first boundary must suspend");
            };
            response.resume_current(boundary.instance_id, BoundaryMode::Updatable)?;
            response.core.repeat_scratch.recycle(Vec::with_capacity(2));
            failure.set(Some(fail_on_write));
            assert!(matches!(
                response.update(boundary.instance_id, &state),
                Err(HandlerError::ClientDisconnected)
            ));
            assert!(response.core.failed);
            assert!(response.core.frozen_state.get("title").is_none());
            assert_eq!(response.core.repeat_scratch.take().capacity(), 0);
            failure.set(None);
            assert!(response.update(boundary.instance_id, &state).is_err());
        }
        Ok(())
    }

    #[test]
    fn state_only_update_sequence_overflow_is_recoverable_and_releases_inputs() -> Result<()> {
        let protocol = boundary_protocol(2, StateProjectionMode::Keys);
        let handler = WebUIHandler::new();
        let render_options = options();
        let mut sink = SharedSink::default();
        let mut response = handler.stream_response(&protocol, &render_options, &mut sink)?;
        let state = test_json!({"count": 1, "title": "retained"});
        let Some(boundary) = response.start(&state)?.boundary else {
            panic!("the first boundary must suspend");
        };
        response.resume_current(boundary.instance_id, BoundaryMode::Updatable)?;
        let Some(progress) = response.core.streaming.as_mut() else {
            panic!("the suspended response must retain streaming progress");
        };
        progress.next_record_sequence = usize::MAX;
        assert!(matches!(
            response.update(boundary.instance_id, &state),
            Err(HandlerError::StreamingBoundary(error)) if error.signal == "update"
        ));
        assert!(response.core.failed);
        assert!(response.core.frozen_state.get("title").is_none());
        Ok(())
    }

    /// Drive `count` occurrences to completion in `mode`, returning the status
    /// the response stopped on.
    fn commit_occurrences<W: FlushWriter + ?Sized>(
        response: &mut StreamingResponse<'_, W>,
        status: StreamStatus,
        state: &Value,
        count: usize,
        mode: BoundaryMode,
    ) -> Result<StreamStatus> {
        let mut status = status;
        for committed in 0..count {
            let Some(boundary) = status.boundary.as_ref() else {
                panic!("occurrence {committed} must suspend before it can commit");
            };
            let instance_id = boundary.instance_id;
            response.resume(instance_id, state, mode)?;
            status = response.advance()?;
        }
        Ok(status)
    }

    #[test]
    fn updatable_commits_stop_at_the_browser_retention_cap() -> Result<()> {
        // The browser retains every updatable occurrence for the life of the
        // response and refuses the one past its cap, so the server refuses to
        // emit that checkpoint at all. The refusal lands before a byte is
        // written and before the caller's state reaches the snapshot, leaving
        // the same occurrence pending so the host can commit it as final.
        let protocol = boundary_protocol(MAX_UPDATABLE_OCCURRENCES + 1, StateProjectionMode::Keys);
        let handler = WebUIHandler::new();
        let state = test_json!({ "count": 1, "title": "retained" });
        let render_options = options();
        let mut sink = SharedSink::default();
        let written = SharedSink::clone(&sink).0;
        let mut response = handler.stream_response(&protocol, &render_options, &mut sink)?;

        let status = response.start(&state)?;
        let status = commit_occurrences(
            &mut response,
            status,
            &state,
            MAX_UPDATABLE_OCCURRENCES,
            BoundaryMode::Updatable,
        )?;

        let Some(boundary) = status.boundary.as_ref() else {
            panic!("the occurrence past the cap must suspend like any other");
        };
        let instance_id = boundary.instance_id;
        let bytes_before = written.borrow().len();
        let refused = test_json!({ "count": 2, "title": "refused" });
        let rejected = response.resume(instance_id, &refused, BoundaryMode::Updatable);
        match rejected {
            Err(HandlerError::StreamingBoundary(error)) => {
                assert_eq!(error.signal, "resume");
                assert!(
                    error.reason.contains("BoundaryMode::Final"),
                    "the refusal must name the recovery: {}",
                    error.reason
                );
            }
            _ => panic!("committing past the cap must fail with a typed boundary error"),
        }
        assert_eq!(
            written.borrow().len(),
            bytes_before,
            "a refused commit must not write a byte"
        );
        assert_eq!(
            response.core.frozen_state.get("title"),
            Some(&Value::String("retained".to_string())),
            "a refused commit must not merge its state into the snapshot"
        );

        // The occurrence is untouched, so the same ID commits as final.
        let status = response.resume(instance_id, &state, BoundaryMode::Final)?;
        assert!(
            status.boundary.is_none() && !status.done,
            "the retried commit stops at its own checkpoint"
        );
        assert!(
            written.borrow().len() > bytes_before,
            "the retry writes its checkpoint"
        );
        assert!(
            response.advance()?.done,
            "the response still reaches its terminal"
        );
        Ok(())
    }

    #[test]
    fn final_commits_do_not_consume_the_updatable_cap() -> Result<()> {
        // Only occurrences the browser retains count against the cap: a final
        // boundary releases its roots at hydration, so a response may commit
        // any number of them and still use its full updatable budget.
        let protocol = boundary_protocol(MAX_UPDATABLE_OCCURRENCES + 2, StateProjectionMode::Keys);
        let handler = WebUIHandler::new();
        let state = test_json!({ "count": 1, "title": "mixed" });
        let render_options = options();
        let mut sink = TestSink {
            output: String::new(),
        };
        let mut response = handler.stream_response(&protocol, &render_options, &mut sink)?;

        let status = response.start(&state)?;
        let status = commit_occurrences(&mut response, status, &state, 2, BoundaryMode::Final)?;
        let status = commit_occurrences(
            &mut response,
            status,
            &state,
            MAX_UPDATABLE_OCCURRENCES,
            BoundaryMode::Updatable,
        )?;
        assert!(
            status.done && response.is_done(),
            "final commits must leave the whole updatable budget available"
        );
        Ok(())
    }

    #[test]
    fn owned_sessions_enforce_the_same_updatable_cap() -> Result<()> {
        // The owned session shares the borrowed session's continuation, so the
        // cap, the refusal, and the final-mode retry behave identically.
        let protocol = Arc::new(boundary_protocol(
            MAX_UPDATABLE_OCCURRENCES + 1,
            StateProjectionMode::Keys,
        ));
        let mut session = crate::streaming::StreamingSession::new(
            Arc::new(WebUIHandler::new()),
            protocol,
            crate::streaming::SessionOptions::new("index.html", "/"),
        )?;
        let state = test_json!({ "count": 1, "title": "owned" });

        let mut step = session.start(&state)?;
        for committed in 0..MAX_UPDATABLE_OCCURRENCES {
            let Some(boundary) = step.boundary.as_ref() else {
                panic!("occurrence {committed} must suspend before it can commit");
            };
            let instance_id = boundary.instance_id;
            session.resume(instance_id, &state, BoundaryMode::Updatable)?;
            step = session.advance()?;
        }

        let Some(boundary) = step.boundary.as_ref() else {
            panic!("the occurrence past the cap must suspend like any other");
        };
        let instance_id = boundary.instance_id;
        assert!(
            session
                .resume(instance_id, &state, BoundaryMode::Updatable)
                .is_err(),
            "an owned session refuses the occurrence past the cap"
        );
        let step = session.resume(instance_id, &state, BoundaryMode::Final)?;
        assert!(
            !step.bytes.is_empty(),
            "the retried commit still delivers its checkpoint bytes"
        );
        assert!(
            session.advance()?.done,
            "the owned response still reaches its terminal"
        );
        Ok(())
    }
}

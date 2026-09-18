// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

//! Shared iterative rendering cursor with owned streaming continuations.

use std::borrow::Cow;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use serde_json::Value;
use webui_protocol::{
    condition_expr, web_ui_fragment::Fragment, BoundaryPhase, ConditionExpr, InitialStateStrategy,
    StateProjectionMode, WebUIFragment, WebUIFragmentBoundary, WebUIFragmentComponent,
    WebUIFragmentFor, WebUIFragmentIf, WebUIProtocol, WebUiFragmentRoute,
};

use super::checkpoint::RangeRecord;
use super::error::{
    boundary_in_repeat_error, boundary_limit_error, boundary_order_error, continuation_limit_error,
    duplicate_boundary_key_error, invalid_boundary_key_error, keyed_instance_limit_error,
    malformed_span_signal_error, span_id_overflow_error, span_nesting_error, updatable_limit_error,
};
use super::root::ComponentHostOrigin;
use super::session::{
    BoundaryDescriptor, BoundaryInstanceId, BoundaryKey, BoundaryMode, SpanInstanceId,
    StreamStatus, MAX_BOUNDARY_OCCURRENCES, MAX_CONTINUATION_DEPTH, MAX_KEYED_INSTANCES,
    MAX_SPAN_NESTING, MAX_UPDATABLE_OCCURRENCES,
};
use super::state::{increment_streaming_record_sequence, protocol_fragment, RecordCapture};
use super::{
    consume_streaming_component_root, prepare_generated_streaming_root, record_checkpoint_tag,
    streaming_state,
};
use crate::render_scope::{RepeatFrame, RepeatSource};
use crate::route_matcher::RouteMatch;
use crate::{
    structural_signal_value, write_interaction_marker, HandlerError, Result, WebUIHandler,
    WebUIProcessContext, STATE_INJECT_KEY,
};

const SPAN_START_PREFIX: &str = "streaming_span_start:";
const SPAN_END_PREFIX: &str = "streaming_span_end:";
const CAPTURE_POOL_LIMIT: usize = 8;
/// Frames retained by a typical entry before any growth.
///
/// Sized from the deepest continuation an ordinary page reaches (entry record,
/// a component host, and one conditional or loop body) so the common response
/// never reallocates its frame stack.
const INITIAL_FRAME_CAPACITY: usize = 16;
pub(crate) struct ContinuationVm<M = StreamingVmState> {
    mode: M,
    frames: Vec<Frame>,
    route_work: Vec<RouteWork>,
    route_matches: Vec<SelectedRoute>,
    next_entry: Option<Entry>,
    input_owner: Option<usize>,
    scopes: Vec<crate::render_scope::SavedScope>,
    active_calls: usize,
    invocations: usize,
    start_index: usize,
}

pub(crate) struct OrdinaryVm;

pub(crate) trait VmMode {
    const ORDINARY: bool;
    fn streaming(&self) -> Option<&StreamingVmState>;
    fn streaming_mut(&mut self) -> Option<&mut StreamingVmState>;
}

impl VmMode for OrdinaryVm {
    const ORDINARY: bool = true;
    fn streaming(&self) -> Option<&StreamingVmState> {
        None
    }
    fn streaming_mut(&mut self) -> Option<&mut StreamingVmState> {
        None
    }
}

impl VmMode for StreamingVmState {
    const ORDINARY: bool = false;
    fn streaming(&self) -> Option<&StreamingVmState> {
        Some(self)
    }
    fn streaming_mut(&mut self) -> Option<&mut StreamingVmState> {
        Some(self)
    }
}

pub(crate) struct StreamingVmState {
    // Only boundary yields materialize a descriptor; ordinary record returns
    // keep a small Result<bool> rather than moving a StreamStatus per item.
    yielded: Option<StreamStatus>,
    pending: Option<PendingBoundary>,
    active: Option<ActiveBoundary>,
    open_spans: Vec<OpenSpan>,
    capture_pool: Vec<RecordCapture>,
    next_boundary_id: u32,
    next_span_id: u32,
    keyed_instances: HashMap<u32, HashSet<BoundaryKey>>,
    keyed_instance_count: usize,
    committed_modes: Vec<BoundaryMode>,
    /// Occurrences already committed as [`BoundaryMode::Updatable`].
    ///
    /// The browser retains every updatable occurrence for the life of the
    /// response, so the cap is a running total rather than a live count.
    /// Keeping it as a counter makes the pre-commit check one integer compare
    /// instead of a scan of every mode already committed.
    updatable_count: usize,
    component_count: usize,
    pending_span_candidate: Option<Box<str>>,
}

/// Immutable per-entry projection surface shared by every response.
///
/// Built once per entry by [`crate::Protocol`] and handed to sessions as a
/// cheap pointer clone, so no response walks the fragment graph merely to
/// decide which top-level state keys its continuation retains.
pub(crate) struct ContinuationStatePlan {
    pub(crate) keys: Arc<[Box<str>]>,
    pub(crate) requires_full_state: bool,
}

/// A memoized [`ContinuationStatePlan`], including a replayable failure.
///
/// Building the plan can fail on a malformed protocol. Capturing that failure
/// keeps the memo authoritative: a bad entry is diagnosed identically on every
/// response without re-walking a graph that is already known to be unusable.
/// The failure is boxed so the memo table stores one small cell per compiled
/// record instead of reserving the diagnostic's payload for every slot.
pub(crate) struct PreparedContinuationStatePlan {
    result: std::result::Result<ContinuationStatePlan, Box<ContinuationStatePlanError>>,
}

enum ContinuationStatePlanError {
    Boundary { signal: String, reason: String },
    MissingFragment(String),
    Invariant(String),
}

impl PreparedContinuationStatePlan {
    pub(crate) fn new(protocol: &WebUIProtocol, entry_id: &str) -> Self {
        Self {
            result: ContinuationVm::collect_state_keys(
                protocol,
                entry_id,
                super::session::MAX_FROZEN_STATE_KEYS,
            )
            .map_err(|error| Box::new(ContinuationStatePlanError::capture(error))),
        }
    }

    pub(crate) fn resolve(&self) -> Result<&ContinuationStatePlan> {
        self.result
            .as_ref()
            .map_err(|error| error.to_handler_error())
    }
}

impl ContinuationStatePlanError {
    #[cold]
    #[inline(never)]
    fn capture(error: HandlerError) -> Self {
        match error {
            HandlerError::StreamingBoundary(error) => Self::Boundary {
                signal: error.signal,
                reason: error.reason,
            },
            HandlerError::MissingFragment(id) => Self::MissingFragment(id),
            HandlerError::Invariant(message) => Self::Invariant(message),
            error => Self::Invariant(error.to_string()),
        }
    }

    #[cold]
    #[inline(never)]
    fn to_handler_error(&self) -> HandlerError {
        match self {
            Self::Boundary { signal, reason } => {
                HandlerError::StreamingBoundary(Box::new(crate::StreamingBoundaryError {
                    signal: signal.clone(),
                    reason: reason.clone(),
                }))
            }
            Self::MissingFragment(id) => HandlerError::MissingFragment(id.clone()),
            Self::Invariant(message) => HandlerError::Invariant(message.clone()),
        }
    }
}

/// The suspended occurrence awaiting `resume`.
///
/// Only the identity is retained: the descriptor handed to the host owns its
/// authored strings, so keeping a second copy here would allocate once more per
/// occurrence for data the continuation never reads.
#[derive(Clone, Copy)]
struct PendingBoundary {
    instance_id: BoundaryInstanceId,
    declaration_id: u32,
}

struct ActiveBoundary {
    instance_id: BoundaryInstanceId,
    declaration_id: u32,
    mode: BoundaryMode,
}

struct OpenSpan {
    id: SpanInstanceId,
    tag: Box<str>,
    capture: RecordCapture,
}

enum Frame {
    EnterFragment(Entry),
    Fragment(FragmentFrame),
    ComponentEnd(ComponentEndFrame),
    IfEnd {
        slot: u32,
    },
    RenderEnd {
        slot: u32,
    },
    Repeat(bool),
    GeneratedComponentStart {
        tag: Box<str>,
        ordinary_routes: bool,
    },
    GeneratedComponentEnd {
        tag: Box<str>,
        spanning: bool,
    },
    RouteWork,
}

enum RouteWork {
    End {
        saved_route_base: Option<String>,
        saved_route_children: std::ops::Range<u32>,
    },
    Outlet(OutletFrame),
}

struct ComponentEndFrame {
    component_slot: u32,
    owns_css_tree: bool,
    saved_scope: bool,
    previous_input_owner: Option<usize>,
}

#[derive(Clone, Copy)]
struct FragmentFrame {
    slot: u32,
    index: usize,
    best_route: bool,
    // Inherited outlet ordering, independent of streaming execution mode.
    ordinary_routes: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct Entry {
    slot: u32,
    ordinary_routes: bool,
}

impl FragmentFrame {
    #[inline(always)]
    fn restart<M: VmMode>(&mut self, ordinary_routes: bool) {
        self.index = 0;
        if !M::ORDINARY {
            self.ordinary_routes = ordinary_routes;
        }
    }
}

struct OutletFrame {
    routes: std::ops::Range<u32>,
    selection: OutletSelection,
}

enum OutletSelection {
    None,
    Pending(SelectedRoute),
    // The winner has started; its body may still be suspended.
    Dispatched(u32),
}

struct SelectedRoute {
    slot: u32,
    consumed_segments: usize,
}

/// What the current [`ContinuationVm::advance`] call is walking toward.
#[derive(Clone, Copy, Eq, PartialEq)]
pub(crate) enum StepGoal {
    /// Complete an ordinary render without streaming markers or limits.
    Ordinary,
    /// Write ordinary parent/shell bytes until the next occurrence or terminal.
    NextBoundary,
    /// Write the pending occurrence through its checkpoint, then stop.
    CommitBoundary,
}

/// Borrowed runtime shared by every record one step walks.
#[derive(Clone, Copy)]
struct StepRuntime<'call, 'data> {
    goal: StepGoal,
    handler: &'call WebUIHandler,
    protocol: &'data crate::Protocol,
    ordinary_routes: bool,
}

impl ContinuationVm {
    pub(crate) fn new(entry_id: &str, protocol: &crate::Protocol) -> Result<Self> {
        let entry_slot = protocol
            .fragment_slot(entry_id)
            .ok_or_else(|| HandlerError::MissingFragment(entry_id.to_string()))?;
        Ok(Self::from_slot(
            entry_slot,
            protocol.component_index().len(),
        ))
    }

    fn from_slot(entry_slot: u32, component_count: usize) -> Self {
        Self::with_mode(
            entry_slot,
            StreamingVmState {
                yielded: None,
                pending: None,
                active: None,
                open_spans: Vec::new(),
                capture_pool: Vec::new(),
                next_boundary_id: 0,
                next_span_id: 0,
                keyed_instances: HashMap::new(),
                keyed_instance_count: 0,
                committed_modes: Vec::new(),
                updatable_count: 0,
                component_count,
                pending_span_candidate: None,
            },
        )
    }
}

impl ContinuationVm<OrdinaryVm> {
    pub(crate) fn ordinary(entry_slot: u32, start: usize) -> Self {
        let mut vm = Self::with_mode(entry_slot, OrdinaryVm);
        vm.start_index = start;
        vm
    }
}

impl<M: VmMode> ContinuationVm<M> {
    fn with_mode(entry_slot: u32, mode: M) -> Self {
        Self {
            mode,
            frames: Vec::new(),
            route_work: Vec::new(),
            route_matches: Vec::new(),
            next_entry: Some(Entry {
                slot: entry_slot,
                ordinary_routes: M::ORDINARY,
            }),
            input_owner: None,
            scopes: Vec::new(),
            active_calls: 0,
            invocations: 0,
            start_index: 0,
        }
    }

    fn streaming(&self) -> Result<&StreamingVmState> {
        self.mode.streaming().ok_or_else(missing_streaming_vm_error)
    }

    fn streaming_mut(&mut self) -> Result<&mut StreamingVmState> {
        self.mode
            .streaming_mut()
            .ok_or_else(missing_streaming_vm_error)
    }

    pub(crate) fn release(&mut self) {
        self.frames.clear();
        self.route_work.clear();
        self.route_matches.clear();
        self.next_entry = None;
        self.scopes.clear();
        self.input_owner = None;
        if let Some(streaming) = self.mode.streaming_mut() {
            streaming.yielded = None;
            streaming.open_spans.clear();
            streaming.capture_pool.clear();
            streaming.pending = None;
            streaming.active = None;
        }
    }

    pub(crate) fn validate_resume(&self, instance_id: BoundaryInstanceId) -> Result<()> {
        let Some(pending) = self.streaming()?.pending.as_ref() else {
            return Err(boundary_order_error(
                "resume",
                "there is no pending boundary occurrence",
            ));
        };
        if pending.instance_id != instance_id {
            return Err(boundary_order_error(
                "resume",
                "the supplied instance ID is stale or does not match the pending occurrence",
            ));
        }
        Ok(())
    }

    /// Reject an `Updatable` commit the browser could not retain.
    ///
    /// Checked before the resume writes a byte or takes the pending
    /// occurrence, so a rejected attempt leaves the response exactly as it was
    /// and the host can commit the same occurrence as
    /// [`BoundaryMode::Final`] instead.
    pub(crate) fn validate_resume_mode(&self, mode: BoundaryMode) -> Result<()> {
        if mode == BoundaryMode::Updatable
            && self.streaming()?.updatable_count >= MAX_UPDATABLE_OCCURRENCES
        {
            return Err(updatable_limit_error(MAX_UPDATABLE_OCCURRENCES));
        }
        Ok(())
    }

    pub(crate) fn validate_update(&self, instance_id: BoundaryInstanceId) -> Result<usize> {
        let index = instance_id.index()?;
        let Some(mode) = self.streaming()?.committed_modes.get(index) else {
            return Err(boundary_order_error(
                "update",
                "the target boundary occurrence has not committed",
            ));
        };
        if *mode != BoundaryMode::Updatable {
            return Err(super::error::boundary_not_updatable_error(index));
        }
        Ok(index)
    }

    pub(crate) fn begin_resume(
        &mut self,
        instance_id: BoundaryInstanceId,
        mode: BoundaryMode,
        context: &mut WebUIProcessContext<'_, '_, '_>,
    ) -> Result<()> {
        self.validate_resume(instance_id)?;
        let Some(pending) = self.streaming_mut()?.pending.take() else {
            return Err(boundary_order_error(
                "resume",
                "there is no pending boundary occurrence",
            ));
        };
        super::write_range_marker(context.writer, "<!--wb:", instance_id.raw())?;
        let streaming = streaming_state(context)?;
        streaming.active_boundary = Some(instance_id.raw());
        streaming.checkpoint_tags.clear();
        streaming.checkpoint_walk_roots.clear();
        streaming.checkpoint_seen.fill(0);
        streaming.checkpoint_needs_expansion = false;
        self.streaming_mut()?.active = Some(ActiveBoundary {
            instance_id: pending.instance_id,
            declaration_id: pending.declaration_id,
            mode,
        });
        Ok(())
    }

    /// Walk the continuation until `goal` is met.
    ///
    /// Both entry points use this cursor; the constant mode removes streaming
    /// validation and signal dispatch from ordinary record instructions.
    ///
    /// [`StepGoal::CommitBoundary`] stops on the active occurrence's checkpoint
    /// so its bytes are one independently writable step;
    /// [`StepGoal::NextBoundary`] writes ordinary parent/shell bytes until the
    /// next occurrence suspends or the terminal record completes.
    pub(crate) fn advance<'data>(
        &mut self,
        goal: StepGoal,
        handler: &WebUIHandler,
        protocol: &'data crate::Protocol,
        context: &mut WebUIProcessContext<'data, '_, '_>,
    ) -> Result<StreamStatus> {
        let runtime = StepRuntime {
            goal,
            handler,
            protocol,
            ordinary_routes: M::ORDINARY,
        };
        let mut next = self.next_frame();
        while let Some(frame) = next {
            match frame {
                Frame::EnterFragment(entry) => {
                    let list = render_fragment(context, entry.slot)?;
                    let mut frame = open_fragment(entry, &list, context, &mut self.route_matches)?;
                    frame.index = std::mem::take(&mut self.start_index);
                    if self.run_fragment(frame, list, runtime, context)? {
                        return self
                            .streaming_mut()?
                            .yielded
                            .take()
                            .ok_or_else(missing_yield_error);
                    }
                }
                Frame::Fragment(frame) => {
                    let list = render_fragment(context, frame.slot)?;
                    if self.run_fragment(frame, list, runtime, context)? {
                        return self
                            .streaming_mut()?
                            .yielded
                            .take()
                            .ok_or_else(missing_yield_error);
                    }
                }
                Frame::ComponentEnd(frame) => self.end_component(frame, protocol, context)?,
                Frame::IfEnd { slot } => {
                    if let Some(plugin) = context.plugin.as_mut() {
                        let fragment_id = protocol
                            .fragment_id(slot)
                            .ok_or_else(|| unknown_fragment_slot_error(slot))?;
                        plugin.pop_scope();
                        plugin.on_if_end(fragment_id, context.writer)?;
                    }
                }
                Frame::RenderEnd { slot } => {
                    self.scopes
                        .pop()
                        .ok_or_else(missing_scope_error)?
                        .exit(context)?;
                    self.active_calls -= 1;
                    if let Some(plugin) = context.plugin.as_mut() {
                        plugin.pop_scope();
                        let id = protocol
                            .fragment_id(slot)
                            .ok_or_else(|| unknown_fragment_slot_error(slot))?;
                        plugin.on_render_end(id, context.writer)?;
                    }
                }
                Frame::Repeat(ordinary_routes) => {
                    self.step_repeat(ordinary_routes, protocol, context)?;
                }
                Frame::GeneratedComponentStart {
                    tag,
                    ordinary_routes,
                } => {
                    self.start_generated_component(
                        tag,
                        StepRuntime {
                            ordinary_routes,
                            ..runtime
                        },
                        context,
                    )?;
                }
                Frame::GeneratedComponentEnd { tag, spanning } => {
                    context.writer.write("</")?;
                    context.writer.write(&tag)?;
                    context.writer.write(">")?;
                    if spanning {
                        self.finish_span(&tag, handler, context)?;
                    }
                }
                Frame::RouteWork => match self.route_work.pop().ok_or_else(missing_scope_error)? {
                    RouteWork::End {
                        saved_route_base,
                        saved_route_children,
                    } => {
                        context.writer.write("</webui-route>")?;
                        context.route_base =
                            saved_route_base.map_or(Cow::Borrowed("/"), Cow::Owned);
                        context.route_children = saved_route_children;
                    }
                    RouteWork::Outlet(frame) => {
                        self.step_outlet(frame, runtime, context)?;
                    }
                },
            }
            next = self.next_frame();
        }

        if !context.scopes.repeats.is_empty() {
            return Err(HandlerError::Invariant(
                "traversal completed while a repeat was still open".to_string(),
            ));
        }
        if M::ORDINARY {
            return Ok(StreamStatus {
                boundary: None,
                done: true,
            });
        }
        let streaming = self.streaming()?;
        if streaming.pending.is_some() || streaming.active.is_some() {
            return Err(HandlerError::Invariant(
                "pending boundary lost its continuation".to_string(),
            ));
        }
        if !streaming.open_spans.is_empty() {
            return Err(malformed_span_signal_error(
                "component span",
                "traversal completed before every component span closed",
            ));
        }
        if streaming.pending_span_candidate.is_some() {
            return Err(malformed_span_signal_error(
                "component span",
                "traversal completed with an unfinished component host",
            ));
        }
        if !context
            .streaming
            .as_ref()
            .is_some_and(|streaming| streaming.head_marker_emitted)
        {
            return Err(HandlerError::MissingStreamingHeadStart { before: "terminal" });
        }
        if !context
            .streaming
            .as_ref()
            .is_some_and(|streaming| streaming.body_ended)
        {
            return Err(HandlerError::MissingStreamingBodyEnd);
        }
        let sequence = streaming_state(context)?.next_record_sequence;
        handler.emit_streaming_terminal(sequence, context)?;
        increment_streaming_record_sequence(
            "terminal",
            &mut streaming_state(context)?.next_record_sequence,
        )?;
        context.writer.end()?;
        Ok(StreamStatus {
            boundary: None,
            done: true,
        })
    }

    // Walk borrowed records without returning through the frame dispatcher
    // for structural descents or consecutive repeat items. Scope/route
    // unwinding and streaming yields still use the same continuation frames.
    fn run_fragment<'data>(
        &mut self,
        mut frame: FragmentFrame,
        mut list: crate::RenderFragmentView<'data>,
        mut runtime: StepRuntime<'_, 'data>,
        context: &mut WebUIProcessContext<'data, '_, '_>,
    ) -> Result<bool> {
        let StepRuntime {
            goal,
            handler,
            protocol,
            ..
        } = runtime;
        loop {
            runtime.ordinary_routes = M::ORDINARY || frame.ordinary_routes;
            loop {
                let index = frame.index;
                let Some(fragment) = list.fragments.get(index) else {
                    if frame.best_route {
                        self.route_matches.pop().ok_or_else(missing_scope_error)?;
                    }
                    if !M::ORDINARY {
                        super::ensure_no_pending_streaming_root(
                            context,
                            "the end of the containing fragment",
                        )?;
                    }
                    break;
                };
                if !M::ORDINARY {
                    super::validate_pending_streaming_root(fragment, context)?;
                    super::validate_streaming_root_opening(&list.fragments[..index], fragment)?;
                }
                frame.index = index + 1;

                match fragment.fragment.as_ref() {
                    Some(Fragment::Raw(raw)) => context.writer.write(&raw.value)?,
                    Some(Fragment::Signal(signal)) => {
                        if M::ORDINARY {
                            handler.process_signal(signal, context)?;
                        } else {
                            self.process_signal(signal, handler, context)?;
                        }
                    }
                    Some(Fragment::Attribute(attribute)) => {
                        handler.process_attribute(
                            attribute,
                            list.target(index),
                            list.attribute_name(index, attribute.attr_skip),
                            context,
                        )?;
                    }
                    Some(Fragment::Plugin(plugin)) => {
                        if let Some(active) = context.plugin.as_mut() {
                            active.on_element_data(&plugin.data, context.writer)?;
                        }
                    }
                    Some(Fragment::Boundary(boundary)) => {
                        if M::ORDINARY {
                            continue;
                        }
                        if boundary.phase() == BoundaryPhase::End {
                            self.finish_boundary(boundary, handler, context)?;
                            if goal == StepGoal::CommitBoundary {
                                // The checkpoint just flushed, so the committed
                                // occurrence ends this step: the parent bytes that
                                // follow belong to the caller's next `advance`.
                                self.push(Frame::Fragment(frame))?;
                                self.streaming_mut()?.yielded = Some(StreamStatus {
                                    boundary: None,
                                    done: false,
                                });
                                return Ok(true);
                            }
                            continue;
                        }
                        let descriptor =
                            self.discover_boundary(boundary, handler, protocol, context)?;
                        self.push(Frame::Fragment(frame))?;
                        self.streaming_mut()?.yielded = Some(StreamStatus {
                            boundary: Some(descriptor),
                            done: false,
                        });
                        return Ok(true);
                    }
                    Some(Fragment::Component(component)) => {
                        let target = list.target(index);
                        self.push(Frame::Fragment(frame))?;
                        self.begin_component(
                            (component, target),
                            ComponentHostOrigin::ParserProduced,
                            runtime,
                            context,
                        )?;
                        break;
                    }
                    Some(Fragment::IfCond(if_cond)) => {
                        let target = list.target(index);
                        if self.begin_if((if_cond, target), frame, runtime, context)? {
                            break;
                        }
                    }
                    Some(Fragment::ForLoop(for_loop)) => {
                        let target = list.target(index);
                        self.push(Frame::Fragment(frame))?;
                        self.begin_repeat((for_loop, target), runtime, context)?;
                        break;
                    }
                    Some(Fragment::Render(render)) => {
                        let target = list.target(index);
                        self.push(Frame::Fragment(frame))?;
                        self.begin_render(render, target, runtime, context)?;
                        break;
                    }
                    Some(Fragment::Route(route)) => {
                        self.begin_route((route, list.target(index)), frame, runtime, context)?;
                        break;
                    }
                    Some(Fragment::Outlet(_)) => {
                        self.push(Frame::Fragment(frame))?;
                        self.begin_outlet(runtime, context)?;
                        break;
                    }
                    None => {}
                }
            }
            // Keep record entry and consecutive repeat items in this cursor.
            // The outer dispatcher is needed only for scope/route unwinding.
            if self.next_entry.is_none() {
                if let Some(Frame::Repeat(ordinary_routes)) = self.frames.last() {
                    self.step_repeat(*ordinary_routes, protocol, context)?;
                }
            }
            let Some(entry) = self.next_entry.take() else {
                if let Some(parent) = self.resume_parent_fragment() {
                    if parent.slot != frame.slot {
                        list = render_fragment(context, parent.slot)?;
                    }
                    frame = parent;
                    continue;
                }
                return Ok(false);
            };
            if entry.slot == frame.slot && !list.has_routes() {
                frame.restart::<M>(entry.ordinary_routes);
            } else {
                list = render_fragment(context, entry.slot)?;
                frame = open_fragment(entry, &list, context, &mut self.route_matches)?;
            }
        }
    }

    fn begin_render<'data, 'state>(
        &mut self,
        render: &'data webui_protocol::WebUiFragmentRender,
        target: Option<usize>,
        runtime: StepRuntime<'_, '_>,
        context: &mut WebUIProcessContext<'data, 'state, '_>,
    ) -> Result<()> {
        let protocol = runtime.protocol;
        if self.active_calls == crate::MAX_FRAGMENT_CALL_DEPTH {
            return Err(fragment_depth_error());
        }
        if self.invocations == crate::MAX_FRAGMENT_INVOCATIONS {
            return Err(fragment_budget_error());
        }
        let slot = match target {
            Some(slot) => u32::try_from(slot).map_err(|_| fragment_depth_error())?,
            None => fragment_slot(protocol, &render.fragment_id)?,
        };
        let (borrowed, captured) = if render.scope.is_empty() && render.alias.is_empty() {
            (None, None)
        } else {
            if render.scope.is_empty() || render.alias.is_empty() {
                return Err(fragment_scope_error(render));
            }
            let borrowed = (!context.state.is_shared())
                .then(|| crate::render_scope::borrowed(&render.scope, value_sources!(context)))
                .flatten();
            let captured = if borrowed.is_none() {
                crate::render_scope::capture(&render.scope, context).or_else(|| {
                    match crate::render_scope::resolve(&render.scope, value_sources!(context)) {
                        Some(Cow::Owned(value)) => {
                            Some(crate::state_view::SharedValue::with_provenance(
                                value,
                                context.render_fragments.provenance_policy(),
                            ))
                        }
                        _ => None,
                    }
                })
            } else {
                None
            };
            if borrowed.is_none() && captured.is_none() {
                return Err(fragment_scope_error(render));
            }
            (borrowed, captured)
        };
        let source_id = if context.plugin.is_some() {
            self.record_fragment_input(captured.as_ref(), context)?
        } else {
            None
        };
        crate::render_buffer::push(
            &mut self.scopes,
            crate::render_scope::SavedScope::enter(context, false),
        );
        context.scopes.alias = borrowed.map(|value| (render.alias.as_str(), value));
        context.scopes.shared_alias = captured.map(|value| (render.alias.clone().into(), value));
        self.active_calls += 1;
        self.invocations += 1;
        if let Some(plugin) = context.plugin.as_mut() {
            plugin.on_render_start(&render.fragment_id, source_id, context.writer)?;
            plugin.push_scope();
        }
        self.push(Frame::RenderEnd { slot })?;
        self.enter(slot, runtime.ordinary_routes)
    }

    /// Register one host-driven call's captured input and retain the identifier its
    /// owning span's host needs while that host is still dormant.
    ///
    /// Definitions are response-wide and deduplicated, so a repeated capture of
    /// the same value costs one hash lookup and adds no wire bytes.
    fn record_fragment_input(
        &mut self,
        captured: Option<&crate::state_view::SharedValue>,
        context: &mut WebUIProcessContext<'_, '_, '_>,
    ) -> Result<Option<u32>> {
        let Some(captured) = captured else {
            return Ok(None);
        };
        // One-shot rendering may own synthetic inputs without needing to
        // transport their provenance across a host-driven state replacement.
        if !context.state.is_shared() {
            return Ok(None);
        }
        let Some(streaming) = context.streaming.as_mut() else {
            return Ok(None);
        };
        let source_id = streaming.fragment_sources.capture(captured)?;
        if let Some(owner) = self.input_owner {
            let span = self
                .streaming_mut()?
                .open_spans
                .get_mut(owner)
                .ok_or_else(missing_scope_error)?;
            span.capture.retain_source(source_id);
        }
        Ok(Some(source_id))
    }

    fn process_signal<'data>(
        &mut self,
        signal: &'data webui_protocol::WebUIFragmentSignal,
        handler: &WebUIHandler,
        context: &mut WebUIProcessContext<'data, '_, '_>,
    ) -> Result<()> {
        if let Some(value) = structural_signal_value(signal) {
            if let Some(tag) = value.strip_prefix(SPAN_START_PREFIX) {
                if self.streaming()?.pending_span_candidate.is_some() {
                    return Err(malformed_span_signal_error(
                        tag,
                        "a component span start arrived before the previous host opening completed",
                    ));
                }
                context.writer.stream_begin_component()?;
                self.streaming_mut()?.pending_span_candidate = Some(tag.into());
                return Ok(());
            }
            if let Some(tag) = value.strip_prefix(SPAN_END_PREFIX) {
                return self.finish_span(tag, handler, context);
            }
            if let Some(tag) = value.strip_prefix(super::root::STREAMING_ROOT_PREFIX) {
                if let Some(candidate) = self.streaming()?.pending_span_candidate.as_ref() {
                    if candidate.as_ref() != tag {
                        return Err(malformed_span_signal_error(
                            tag,
                            "component root does not match its pending span host",
                        ));
                    }
                    context.writer.stream_mark_component_root()?;
                    return Ok(());
                }
            }
        }
        handler.process_signal(signal, context)
    }

    fn begin_component<'data>(
        &mut self,
        component: (&WebUIFragmentComponent, Option<usize>),
        origin: ComponentHostOrigin,
        runtime: StepRuntime<'_, 'data>,
        context: &mut WebUIProcessContext<'data, '_, '_>,
    ) -> Result<()> {
        let StepRuntime {
            handler, protocol, ..
        } = runtime;
        let parser_produced = matches!(origin, ComponentHostOrigin::ParserProduced);
        let (component, target) = component;
        let mut input_owner = None;
        if M::ORDINARY {
            // Ordinary hosts share all structural transitions but no streaming inventory.
        } else if let Some(candidate) = self.streaming_mut()?.pending_span_candidate.take() {
            if candidate.as_ref() != component.fragment_id {
                return Err(malformed_span_signal_error(
                    &component.fragment_id,
                    "component fragment does not match its buffered span opening",
                ));
            }
            if !protocol_fragment(protocol.protocol(), &component.fragment_id)?.contains_boundary {
                return Err(malformed_span_signal_error(
                    &component.fragment_id,
                    "component span signals require a boundary-containing fragment record",
                ));
            }
            let active_boundary = streaming_state(context)?.active_boundary.is_some();
            let enclosing_span = streaming_state(context)?.current_span;
            let span_id = self.open_span(&component.fragment_id, context, false)?;
            input_owner = Some(self.streaming()?.open_spans.len() - 1);
            context.writer.stream_commit_component(
                Some(span_id.raw()),
                if active_boundary {
                    enclosing_span
                } else {
                    None
                },
                true,
            )?;
            streaming_state(context)?.current_span = Some(span_id.raw());
            self.record_component(&component.fragment_id, context)?;
        } else {
            consume_streaming_component_root(&component.fragment_id, origin, context)?;
            let pending_span = streaming_state(context)?.pending_span_host.take();
            if let Some(span_id) = pending_span {
                let Some(open) = self.streaming()?.open_spans.last() else {
                    return Err(malformed_span_signal_error(
                        &component.fragment_id,
                        "component host references a span that is not open",
                    ));
                };
                if open.id.raw() != span_id {
                    return Err(malformed_span_signal_error(
                        &component.fragment_id,
                        "component host span ID does not match the open span",
                    ));
                }
                streaming_state(context)?.current_span = Some(span_id);
                input_owner = Some(self.streaming()?.open_spans.len() - 1);
            }
            self.record_component(&component.fragment_id, context)?;
        }

        if !context.rendered_components.contains(&component.fragment_id) {
            handler.emit_css_module(&component.fragment_id, context)?;
            context
                .rendered_components
                .insert(component.fragment_id.clone());
        }
        let slot = prepared_slot(target, protocol, &component.fragment_id)?;
        let list = context
            .render_fragments
            .list(slot as usize)
            .ok_or_else(missing_scope_error)?;
        let ordinary_routes = M::ORDINARY
            || runtime.ordinary_routes
            || (parser_produced && !list.list.contains_boundary);
        let owns_css_tree = list.owns_css_tree();
        if owns_css_tree {
            if let Some(index) = list.shadow_style_index() {
                WebUIHandler::push_indexed_shadow_style_root(
                    index,
                    &mut context.shadow_style_roots,
                );
            } else {
                WebUIHandler::push_shadow_style_root(&component.fragment_id, context)?;
            }
        }
        // Snapshot before the scope transition moves the props out of the
        // component attribute accumulators.
        if let Some(owner) = input_owner {
            self.retain_owner_props(owner, context)?;
        }
        let scope = if M::ORDINARY {
            crate::render_scope::SavedScope::enter_component(context)
        } else {
            Some(crate::render_scope::SavedScope::enter(context, true))
        };
        let saved_scope = scope.is_some();
        if let Some(scope) = scope {
            crate::render_buffer::push(&mut self.scopes, scope);
        }
        if let Some(plugin) = context.plugin.as_mut() {
            plugin.push_scope();
        }

        let previous_input_owner = std::mem::replace(&mut self.input_owner, input_owner);
        self.push(Frame::ComponentEnd(ComponentEndFrame {
            component_slot: slot,
            owns_css_tree,
            saved_scope,
            previous_input_owner,
        }))?;
        self.enter(slot, ordinary_routes)?;
        Ok(())
    }

    /// Retain one host's own props on the span that will announce its
    /// completion.
    ///
    /// A span record commits after the host's boundary already resumed with the
    /// caller's next state, so priming the host from that state would hand it
    /// values it never rendered with. Owned props move into the shared
    /// attribute map rather than being cloned: the component resolves them from
    /// exactly the handle the span retains, so the snapshot costs one `Arc`
    /// clone and one name per prop.
    fn retain_owner_props(
        &mut self,
        owner: usize,
        context: &mut WebUIProcessContext<'_, '_, '_>,
    ) -> Result<()> {
        let span = self
            .streaming_mut()?
            .open_spans
            .get_mut(owner)
            .ok_or_else(missing_scope_error)?;
        span.capture.owner_props.clear();
        if context.component_attrs.is_empty() && context.scopes.attrs.is_empty() {
            return Ok(());
        }
        let provenance = context.render_fragments.provenance_policy();
        for (name, value) in context.component_attrs.drain() {
            context.scopes.attrs.insert(
                name,
                crate::state_view::SharedValue::with_provenance(value, provenance),
            );
        }
        span.capture.owner_props.reserve(context.scopes.attrs.len());
        for (name, value) in context.scopes.attrs.iter() {
            span.capture
                .owner_props
                .push((name.as_str().into(), value.clone()));
        }
        // Sorted so a record's props serialize in the same order on every
        // response regardless of hash iteration order.
        span.capture
            .owner_props
            .sort_unstable_by(|left, right| left.0.cmp(&right.0));
        Ok(())
    }

    fn end_component(
        &mut self,
        frame: ComponentEndFrame,
        protocol: &crate::Protocol,
        context: &mut WebUIProcessContext<'_, '_, '_>,
    ) -> Result<()> {
        self.input_owner = frame.previous_input_owner;
        if let Some(plugin) = context.plugin.as_mut() {
            plugin.pop_scope();
        }
        if frame.saved_scope {
            self.scopes
                .pop()
                .ok_or_else(missing_scope_error)?
                .exit(context)?;
        } else {
            crate::render_scope::SavedScope::exit_empty_component(context);
        }
        if frame.owns_css_tree {
            let component = protocol
                .fragment_id(frame.component_slot)
                .ok_or_else(|| unknown_fragment_slot_error(frame.component_slot))?;
            WebUIHandler::pop_shadow_style_root(component, context)?;
        }
        Ok(())
    }

    fn begin_if(
        &mut self,
        if_cond: (&WebUIFragmentIf, Option<usize>),
        parent: FragmentFrame,
        runtime: StepRuntime<'_, '_>,
        context: &mut WebUIProcessContext<'_, '_, '_>,
    ) -> Result<bool> {
        let (if_cond, target) = if_cond;
        let condition = if_cond
            .condition
            .as_ref()
            .ok_or_else(missing_if_condition_error)?;
        let condition_met = runtime.handler.evaluate_condition(condition, context)?;
        if let Some(plugin) = context.plugin.as_mut() {
            plugin.on_if_start(&if_cond.fragment_id, context.writer)?;
        }
        if condition_met {
            self.push(Frame::Fragment(parent))?;
            let slot = prepared_slot(target, runtime.protocol, &if_cond.fragment_id)?;
            let ordinary_routes =
                self.child_outlet_order(runtime.ordinary_routes, slot, context)?;
            if let Some(plugin) = context.plugin.as_mut() {
                plugin.push_scope();
                self.push(Frame::IfEnd { slot })?;
            }
            self.enter(slot, ordinary_routes)?;
        } else if let Some(plugin) = context.plugin.as_mut() {
            plugin.on_if_end(&if_cond.fragment_id, context.writer)?;
        }
        Ok(condition_met)
    }

    /// Open a `<for>` repeat.
    ///
    /// The whole repeat is atomic: it can carry no boundary, so every frame it
    /// pushes is drained before this step returns to the host and the repeat
    /// never becomes resumable continuation state.
    fn begin_repeat<'data, 'state>(
        &mut self,
        for_loop: (&'data WebUIFragmentFor, Option<usize>),
        runtime: StepRuntime<'_, '_>,
        context: &mut WebUIProcessContext<'data, 'state, '_>,
    ) -> Result<()> {
        let StepRuntime {
            handler, protocol, ..
        } = runtime;
        let (for_loop, target) = for_loop;
        let borrowed = (!context.state.is_shared())
            .then(|| crate::render_scope::borrowed(&for_loop.collection, value_sources!(context)))
            .flatten();
        let items = if let Some(value) = borrowed {
            let items = value
                .as_array()
                .ok_or_else(|| non_array_collection_error(&for_loop.collection))?;
            RepeatSource::Borrowed(items)
        } else {
            let source = crate::render_scope::capture(&for_loop.collection, context);
            let source = match source {
                Some(source) => source,
                None => crate::state_view::SharedValue::with_provenance(
                    handler
                        .resolve_value_owned(&for_loop.collection, context)
                        .unwrap_or_else(|| Value::Array(Vec::new())),
                    context.render_fragments.provenance_policy(),
                ),
            };
            if !source.get().is_array() {
                return Err(non_array_collection_error(&for_loop.collection));
            }
            RepeatSource::Shared(source)
        };
        if let Some(plugin) = context.plugin.as_mut() {
            plugin.on_for_start(&for_loop.fragment_id, context.writer)?;
        }
        let saved_value = context.scopes.loops.remove(&for_loop.item);
        let slot = prepared_slot(target, protocol, &for_loop.fragment_id)?;
        let ordinary_routes = self.child_outlet_order(runtime.ordinary_routes, slot, context)?;
        crate::render_buffer::push(
            &mut context.scopes.repeats,
            RepeatFrame {
                slot,
                declaration: for_loop,
                items,
                index: 0,
                saved_value,
                visible: context.visible_loop_scope,
            },
        );
        self.push(Frame::Repeat(ordinary_routes))
    }

    /// Close the item the repeat just rendered, then open the next one.
    #[inline(always)]
    fn step_repeat(
        &mut self,
        ordinary_routes: bool,
        protocol: &crate::Protocol,
        context: &mut WebUIProcessContext<'_, '_, '_>,
    ) -> Result<()> {
        let frame = context
            .scopes
            .repeats
            .last_mut()
            .ok_or_else(missing_scope_error)?;
        // A non-zero index means the previous item's body has just finished.
        if let Some(open) = frame.index.checked_sub(1) {
            if matches!(frame.items, RepeatSource::Borrowed(_)) {
                context.visible_loop_scope = frame.visible;
            }
            if let Some(plugin) = context.plugin.as_mut() {
                plugin.pop_scope();
                plugin.on_repeat_item_end(open, context.writer)?;
            }
        }
        let has_next = match &frame.items {
            RepeatSource::Borrowed(items) => {
                if let Some(item) = items.get(frame.index) {
                    if frame.index == 0 {
                        context.loop_vars.push(crate::LoopBinding {
                            name: &frame.declaration.item,
                            value: item,
                        });
                    } else {
                        context
                            .loop_vars
                            .last_mut()
                            .ok_or_else(missing_scope_error)?
                            .value = item;
                    }
                    context.visible_loop_scope.end = context.loop_vars.len();
                    true
                } else {
                    if frame.index != 0 {
                        context.loop_vars.pop();
                    }
                    false
                }
            }
            RepeatSource::Shared(source) => step_shared_repeat(
                source,
                (&frame.declaration.item, frame.index),
                &mut context.scopes.loops,
            ),
        };
        if has_next {
            let index = frame.index;
            frame.index = index.wrapping_add(1);
            if let Some(plugin) = context.plugin.as_mut() {
                plugin.on_repeat_item_start(index, context.writer)?;
                plugin.push_scope();
            }
            let slot = frame.slot;
            self.enter(slot, ordinary_routes)?;
            return Ok(());
        }
        self.finish_repeat(protocol, context)
    }

    // Exhaustion and durable-value cleanup run once per repeat, not per item.
    #[inline(never)]
    fn finish_repeat(
        &mut self,
        protocol: &crate::Protocol,
        context: &mut WebUIProcessContext<'_, '_, '_>,
    ) -> Result<()> {
        match self.frames.pop() {
            Some(Frame::Repeat(_)) => {}
            _ => return Err(missing_scope_error()),
        }
        let frame = context
            .scopes
            .repeats
            .pop()
            .ok_or_else(missing_scope_error)?;
        match frame.saved_value {
            Some(value) => {
                context
                    .scopes
                    .loops
                    .insert(frame.declaration.item.clone(), value);
            }
            None => {
                context.scopes.loops.remove(frame.declaration.item.as_str());
            }
        }
        if let Some(plugin) = context.plugin.as_mut() {
            let fragment_id = protocol
                .fragment_id(frame.slot)
                .ok_or_else(|| unknown_fragment_slot_error(frame.slot))?;
            plugin.on_for_end(fragment_id, context.writer)?;
        }
        Ok(())
    }

    fn discover_boundary(
        &mut self,
        boundary: &WebUIFragmentBoundary,
        handler: &WebUIHandler,
        protocol: &crate::Protocol,
        context: &WebUIProcessContext<'_, '_, '_>,
    ) -> Result<BoundaryDescriptor> {
        if self.streaming()?.pending.is_some()
            || self.streaming()?.active.is_some()
            || context
                .streaming
                .as_ref()
                .is_some_and(|s| s.active_boundary.is_some())
        {
            return Err(boundary_order_error(
                "boundary",
                "a nested boundary occurrence is not valid",
            ));
        }
        if !context.scopes.repeats.is_empty() {
            return Err(boundary_in_repeat_error(&boundary.name));
        }
        let next_boundary_id = self.streaming()?.next_boundary_id;
        let index = usize::try_from(next_boundary_id)
            .map_err(|_| boundary_limit_error(MAX_BOUNDARY_OCCURRENCES))?;
        if index >= MAX_BOUNDARY_OCCURRENCES {
            return Err(boundary_limit_error(MAX_BOUNDARY_OCCURRENCES));
        }
        let instance_id = BoundaryInstanceId::from_raw(next_boundary_id);
        self.streaming_mut()?.next_boundary_id = next_boundary_id
            .checked_add(1)
            .ok_or_else(|| boundary_limit_error(MAX_BOUNDARY_OCCURRENCES))?;
        let key = self.evaluate_boundary_key(boundary, handler, context)?;
        if boundary.may_repeat {
            let Some(key) = key.as_ref() else {
                return Err(invalid_boundary_key_error(
                    boundary.declaration_id,
                    &boundary.name,
                    "a declaration that may repeat has no key",
                ));
            };
            let streaming = self.streaming_mut()?;
            if streaming.keyed_instance_count >= MAX_KEYED_INSTANCES {
                return Err(keyed_instance_limit_error(MAX_KEYED_INSTANCES));
            }
            let keys = streaming
                .keyed_instances
                .entry(boundary.declaration_id)
                .or_default();
            if !keys.insert(key.clone()) {
                return Err(duplicate_boundary_key_error(
                    boundary.declaration_id,
                    &boundary.name,
                    &key.diagnostic(),
                ));
            }
            streaming.keyed_instance_count += 1;
        }
        let (owner, name) = match protocol.boundary_declaration(boundary.declaration_id) {
            Some(declaration) => (
                Arc::clone(&declaration.owner),
                Arc::clone(&declaration.name),
            ),
            None => (
                Arc::from(boundary.owner_fragment_id.as_str()),
                Arc::from(boundary.name.as_str()),
            ),
        };
        let descriptor = BoundaryDescriptor {
            instance_id,
            declaration_id: boundary.declaration_id,
            owner,
            name,
            key,
        };
        self.streaming_mut()?.pending = Some(PendingBoundary {
            instance_id,
            declaration_id: boundary.declaration_id,
        });
        Ok(descriptor)
    }

    fn evaluate_boundary_key(
        &self,
        boundary: &WebUIFragmentBoundary,
        handler: &WebUIHandler,
        context: &WebUIProcessContext<'_, '_, '_>,
    ) -> Result<Option<BoundaryKey>> {
        let Some(raw) = boundary.key.as_deref() else {
            return Ok(None);
        };
        let trimmed = raw.trim();
        let path = trimmed
            .strip_prefix("{{")
            .and_then(|value| value.strip_suffix("}}"))
            .map_or(trimmed, str::trim);
        let Some(value) = handler.resolve_value_owned(path, context) else {
            return Err(invalid_boundary_key_error(
                boundary.declaration_id,
                &boundary.name,
                "the expression did not resolve",
            ));
        };
        match value {
            Value::String(value) => Ok(Some(BoundaryKey::String(value))),
            Value::Number(value) => Ok(Some(BoundaryKey::Number(value))),
            _ => Err(invalid_boundary_key_error(
                boundary.declaration_id,
                &boundary.name,
                "the expression resolved to a non-number/non-string value",
            )),
        }
    }

    fn finish_boundary(
        &mut self,
        marker: &WebUIFragmentBoundary,
        handler: &WebUIHandler,
        context: &mut WebUIProcessContext<'_, '_, '_>,
    ) -> Result<()> {
        let Some(active) = self.streaming_mut()?.active.take() else {
            return Err(boundary_order_error(
                "boundary",
                "a boundary end marker has no active occurrence",
            ));
        };
        if active.declaration_id != marker.declaration_id
            || streaming_state(context)?.active_boundary != Some(active.instance_id.raw())
        {
            return Err(boundary_order_error(
                "boundary",
                "the active boundary occurrence changed before its body completed",
            ));
        }
        super::write_range_marker(context.writer, "<!--/wb:", active.instance_id.raw())?;
        let enclosing_span_instance_id = streaming_state(context)?.current_span;
        handler.emit_streaming_range_record(
            RangeRecord::Boundary {
                instance_id: active.instance_id.raw(),
                declaration_id: active.declaration_id,
                enclosing_span_instance_id,
                updatable: active.mode == BoundaryMode::Updatable,
            },
            context,
        )?;
        increment_streaming_record_sequence(
            "boundary",
            &mut streaming_state(context)?.next_record_sequence,
        )?;
        streaming_state(context)?.active_boundary = None;
        let streaming = self.streaming_mut()?;
        let expected = streaming.committed_modes.len();
        if active.instance_id.index()? != expected {
            return Err(HandlerError::Invariant(
                "committed boundary IDs are not gapless".to_string(),
            ));
        }
        // Counted here, not at resume: only an occurrence whose checkpoint
        // actually reached the client consumes the browser's retention budget.
        if active.mode == BoundaryMode::Updatable {
            streaming.updatable_count = streaming.updatable_count.saturating_add(1);
        }
        streaming.committed_modes.push(active.mode);
        Ok(())
    }

    fn start_span(
        &mut self,
        tag: &str,
        context: &mut WebUIProcessContext<'_, '_, '_>,
    ) -> Result<()> {
        let id = self.open_span(tag, context, true)?;
        streaming_state(context)?.pending_span_host = Some(id.raw());
        Ok(())
    }

    fn open_span(
        &mut self,
        tag: &str,
        context: &mut WebUIProcessContext<'_, '_, '_>,
        write_marker: bool,
    ) -> Result<SpanInstanceId> {
        if tag.is_empty() {
            return Err(malformed_span_signal_error(
                SPAN_START_PREFIX,
                "component span start is missing its tag",
            ));
        }
        let streaming = self.streaming_mut()?;
        if streaming.open_spans.len() >= MAX_SPAN_NESTING {
            return Err(span_nesting_error(MAX_SPAN_NESTING));
        }
        let id = SpanInstanceId::new(streaming.next_span_id);
        streaming.next_span_id = streaming
            .next_span_id
            .checked_add(1)
            .ok_or_else(span_id_overflow_error)?;
        if write_marker {
            super::write_range_marker(context.writer, "<!--ws:", id.raw())?;
        }
        let mut capture = streaming
            .capture_pool
            .pop()
            .unwrap_or_else(|| RecordCapture::new(streaming.component_count));
        capture.clear();
        streaming.open_spans.push(OpenSpan {
            id,
            tag: tag.into(),
            capture,
        });
        Ok(id)
    }

    fn finish_span(
        &mut self,
        tag: &str,
        handler: &WebUIHandler,
        context: &mut WebUIProcessContext<'_, '_, '_>,
    ) -> Result<()> {
        let Some(mut span) = self.streaming_mut()?.open_spans.pop() else {
            return Err(malformed_span_signal_error(
                tag,
                "component span end has no matching start",
            ));
        };
        if span.tag.as_ref() != tag {
            return Err(malformed_span_signal_error(
                tag,
                "component span end does not match the innermost open host",
            ));
        }
        if streaming_state(context)?.active_boundary.is_some() {
            return Err(malformed_span_signal_error(
                tag,
                "component host closed before its boundary body committed",
            ));
        }
        super::write_range_marker(context.writer, "<!--/ws:", span.id.raw())?;
        streaming_state(context)?.swap_capture(&mut span.capture);
        let result = handler.emit_streaming_range_record(
            RangeRecord::Span {
                instance_id: span.id.raw(),
                owner_props: &span.capture.owner_props,
            },
            context,
        );
        streaming_state(context)?.swap_capture(&mut span.capture);
        result?;
        increment_streaming_record_sequence(
            "span completion",
            &mut streaming_state(context)?.next_record_sequence,
        )?;
        let streaming = self.streaming_mut()?;
        streaming_state(context)?.current_span =
            streaming.open_spans.last().map(|open| open.id.raw());
        streaming_state(context)?.pending_span_host = None;
        if streaming.capture_pool.len() < CAPTURE_POOL_LIMIT {
            span.capture.clear();
            streaming.capture_pool.push(span.capture);
        }
        Ok(())
    }

    fn record_component(
        &mut self,
        tag: &str,
        context: &mut WebUIProcessContext<'_, '_, '_>,
    ) -> Result<()> {
        if context
            .streaming
            .as_ref()
            .is_some_and(|streaming| streaming.active_boundary.is_some())
        {
            record_checkpoint_tag(context, tag);
            return Ok(());
        }
        let Some(span) = self.streaming_mut()?.open_spans.last_mut() else {
            return Err(super::error::streaming_root_outside_boundary_error(tag));
        };
        streaming_state(context)?.swap_capture(&mut span.capture);
        record_checkpoint_tag(context, tag);
        streaming_state(context)?.swap_capture(&mut span.capture);
        Ok(())
    }

    fn start_generated_component<'data>(
        &mut self,
        tag: Box<str>,
        runtime: StepRuntime<'_, 'data>,
        context: &mut WebUIProcessContext<'data, '_, '_>,
    ) -> Result<()> {
        let protocol = runtime.protocol;
        let spanning =
            !M::ORDINARY && protocol_fragment(protocol.protocol(), &tag)?.contains_boundary;
        let enclosed = context.streaming.as_ref().is_some_and(|streaming| {
            streaming.active_boundary.is_some() || streaming.current_span.is_some()
        });
        if !M::ORDINARY && !spanning && !enclosed {
            return Err(super::error::streaming_root_outside_boundary_error(&tag));
        }
        let component = WebUIFragmentComponent {
            fragment_id: tag.to_string(),
        };
        if spanning {
            self.start_span(&tag, context)?;
        }
        context.writer.write("<")?;
        context.writer.write(&tag)?;
        if let Some(plugin) = context.plugin.as_ref() {
            plugin.write_route_component_state(context.state, context.writer)?;
        }
        write_interaction_marker(&tag, context)?;
        prepare_generated_streaming_root(&tag, context)?;
        context.writer.write(">")?;
        self.push(Frame::GeneratedComponentEnd { tag, spanning })?;
        self.begin_component(
            (&component, None),
            ComponentHostOrigin::HandlerGenerated,
            runtime,
            context,
        )
    }

    #[inline(never)]
    fn begin_route(
        &mut self,
        route: (&WebUiFragmentRoute, Option<usize>),
        frame: FragmentFrame,
        runtime: StepRuntime<'_, '_>,
        context: &mut WebUIProcessContext<'_, '_, '_>,
    ) -> Result<()> {
        let (route, target) = route;
        let route_slot = prepared_slot(target, runtime.protocol, &route.fragment_id)?;
        let consumed_segments = if frame.best_route {
            let selected = self.route_matches.last().ok_or_else(missing_scope_error)?;
            let selected_route = runtime
                .protocol
                .render_routes
                .get(selected.slot)
                .ok_or_else(missing_scope_error)?;
            (selected_route.fragment_id == route.fragment_id).then_some(selected.consumed_segments)
        } else {
            None
        };
        self.push(Frame::Fragment(frame))?;
        self.render_route(route_slot, consumed_segments, runtime, context)
    }

    fn render_route(
        &mut self,
        route_slot: u32,
        consumed_segments: Option<usize>,
        runtime: StepRuntime<'_, '_>,
        context: &mut WebUIProcessContext<'_, '_, '_>,
    ) -> Result<()> {
        let StepRuntime {
            handler,
            protocol,
            ordinary_routes,
            ..
        } = runtime;
        let route = protocol
            .render_routes
            .get(route_slot)
            .ok_or_else(missing_scope_error)?;
        context.writer.write("<webui-route path=\"")?;
        context.writer.write(&route.path)?;
        context.writer.write("\"")?;
        if !route.fragment_id.is_empty() {
            context.writer.write(" component=\"")?;
            context.writer.write(&route.fragment_id)?;
            context.writer.write("\"")?;
        }
        if route.exact {
            context.writer.write(" exact")?;
        }
        crate::route_renderer::write_route_navigation_attrs(context.writer, route)?;
        let Some(consumed_segments) = consumed_segments else {
            return context
                .writer
                .write(" style=\"display:none\"></webui-route>");
        };

        let route_index = context.route_chain_index;
        context.route_chain_index = context
            .route_chain_index
            .checked_add(1)
            .ok_or_else(route_chain_limit_error)?;
        context.writer.write(" data-ri=\"")?;
        crate::write_usize(context.writer, route_index)?;
        context.writer.write("\" active>")?;

        let saved_route_base = match std::mem::replace(
            &mut context.route_base,
            Cow::Owned(crate::route_matcher::compute_route_base(
                context.request_path,
                consumed_segments,
            )),
        ) {
            Cow::Owned(base) => Some(base),
            Cow::Borrowed(_) => None,
        };
        let saved_route_children = std::mem::replace(
            &mut context.route_children,
            protocol
                .render_routes
                .children(route_slot)
                .ok_or_else(missing_scope_error)?,
        );
        self.route_work.push(RouteWork::End {
            saved_route_base,
            saved_route_children,
        });
        self.push(Frame::RouteWork)?;
        // Streaming carries route CSS in the checkpoint payload instead of
        // installing it inline, so only the ordinary render writes it here.
        if M::ORDINARY
            && !route.fragment_id.is_empty()
            && !WebUIHandler::component_owns_css_tree(&route.fragment_id, context.protocol)
        {
            handler.emit_component_style_closure(
                &route.fragment_id,
                crate::StyleClosureInstall::Routed,
                context,
            )?;
        }
        if !route.fragment_id.is_empty() {
            self.push(Frame::GeneratedComponentStart {
                tag: route.fragment_id.clone().into(),
                ordinary_routes: M::ORDINARY || ordinary_routes,
            })?;
        }
        if !route.content_fragment_id.is_empty() {
            let slot = fragment_slot(protocol, &route.content_fragment_id)?;
            self.enter(slot, ordinary_routes)?;
        }
        Ok(())
    }

    fn begin_outlet(
        &mut self,
        runtime: StepRuntime<'_, '_>,
        context: &mut WebUIProcessContext<'_, '_, '_>,
    ) -> Result<()> {
        if let Some(plugin) = context.plugin.as_mut() {
            plugin.on_outlet_start(context.writer)?;
        }
        let routes = std::mem::take(&mut context.route_children);
        if routes.is_empty() {
            if let Some(plugin) = context.plugin.as_mut() {
                plugin.on_outlet_end(context.writer)?;
            }
            return Ok(());
        }
        let request_segments = crate::route_matcher::split_request_path(context.request_path);
        let mut best: Option<(u32, RouteMatch)> = None;
        for index in routes.clone() {
            let route = runtime
                .protocol
                .render_routes
                .get(index)
                .ok_or_else(missing_scope_error)?;
            if let Some(route_match) = crate::route_matcher::match_route_indexed_with_segments(
                context.route_index,
                &route.path,
                &context.route_base,
                &request_segments,
                route.exact,
            ) {
                if best
                    .as_ref()
                    .is_none_or(|(_, current)| route_match.specificity > current.specificity)
                {
                    best = Some((index, route_match));
                }
            }
        }
        let best = best.map(|(slot, matched)| SelectedRoute {
            slot,
            consumed_segments: matched.consumed_segments,
        });
        drop(request_segments);
        let (selection, ordinary_match) = match best {
            Some(selected) if M::ORDINARY || runtime.ordinary_routes => (
                OutletSelection::Dispatched(selected.slot),
                Some((selected.slot, selected.consumed_segments)),
            ),
            Some(selected) => (OutletSelection::Pending(selected), None),
            None => (OutletSelection::None, None),
        };
        self.route_work
            .push(RouteWork::Outlet(OutletFrame { routes, selection }));
        self.push(Frame::RouteWork)?;
        if let Some((slot, consumed_segments)) = ordinary_match {
            self.render_route(slot, Some(consumed_segments), runtime, context)?;
        }
        Ok(())
    }

    fn step_outlet(
        &mut self,
        mut frame: OutletFrame,
        runtime: StepRuntime<'_, '_>,
        context: &mut WebUIProcessContext<'_, '_, '_>,
    ) -> Result<()> {
        let next = match &frame.selection {
            OutletSelection::Dispatched(selected) => frame.routes.find(|index| index != selected),
            _ => frame.routes.next(),
        };
        let Some(index) = next else {
            if let Some(plugin) = context.plugin.as_mut() {
                plugin.on_outlet_end(context.writer)?;
            }
            return Ok(());
        };
        let route_match = match &frame.selection {
            OutletSelection::Pending(selected) if selected.slot == index => {
                Some(selected.consumed_segments)
            }
            _ => None,
        };
        if route_match.is_some() {
            frame.selection = OutletSelection::Dispatched(index);
        }
        self.route_work.push(RouteWork::Outlet(frame));
        self.push(Frame::RouteWork)?;
        // Matched-first winners were dispatched at outlet entry.
        self.render_route(
            index,
            route_match,
            StepRuntime {
                ordinary_routes: false,
                ..runtime
            },
            context,
        )
    }

    #[inline]
    fn resume_parent_fragment(&mut self) -> Option<FragmentFrame> {
        let Some(Frame::Fragment(parent)) = self.frames.last() else {
            return None;
        };
        let parent = *parent;
        self.frames.pop();
        Some(parent)
    }

    fn next_frame(&mut self) -> Option<Frame> {
        self.next_entry
            .take()
            .map(Frame::EnterFragment)
            .or_else(|| match self.frames.last() {
                Some(Frame::Repeat(ordinary_routes)) => Some(Frame::Repeat(*ordinary_routes)),
                _ => self.frames.pop(),
            })
    }

    #[inline]
    fn child_outlet_order(
        &self,
        inherited: bool,
        slot: u32,
        context: &WebUIProcessContext<'_, '_, '_>,
    ) -> Result<bool> {
        if M::ORDINARY || inherited {
            return Ok(true);
        }
        let target = context
            .render_fragments
            .list(slot as usize)
            .ok_or_else(missing_scope_error)?;
        Ok(!target.list.contains_boundary)
    }

    fn enter(&mut self, slot: u32, ordinary_routes: bool) -> Result<()> {
        if !M::ORDINARY && self.frames.len() >= MAX_CONTINUATION_DEPTH {
            return Err(continuation_limit_error(MAX_CONTINUATION_DEPTH));
        }
        self.next_entry = Some(Entry {
            slot,
            ordinary_routes: M::ORDINARY || ordinary_routes,
        });
        Ok(())
    }

    fn push(&mut self, frame: Frame) -> Result<()> {
        if !M::ORDINARY && self.frames.len() >= MAX_CONTINUATION_DEPTH {
            return Err(continuation_limit_error(MAX_CONTINUATION_DEPTH));
        }
        if self.frames.capacity() == 0 {
            self.frames.reserve(INITIAL_FRAME_CAPACITY);
        }
        self.frames.push(frame);
        Ok(())
    }
}

impl ContinuationVm {
    /// Build the bounded frozen continuation surface.
    ///
    /// Exact signal/condition/attribute roots and component hydration
    /// projections are retained. A correctness-safe full projection is recorded
    /// once for the response when the protocol or any reachable component
    /// explicitly requires `ALL`.
    pub(crate) fn collect_state_keys(
        protocol: &WebUIProtocol,
        entry_id: &str,
        limit: usize,
    ) -> Result<ContinuationStatePlan> {
        let mut keys = HashSet::new();
        keys.insert(STATE_INJECT_KEY.to_string());
        let mut pending = vec![(entry_id, Vec::<&str>::new())];
        let mut visited = HashSet::new();
        let mut fragment_keys = HashSet::new();
        let mut children = Vec::new();
        let mut requires_full_state =
            protocol.initial_state_strategy != InitialStateStrategy::Components as i32;
        while let Some((id, excluded)) = pending.pop() {
            if !visited.insert((id, excluded.clone())) {
                continue;
            }
            let list = protocol
                .fragments
                .get(id)
                .ok_or_else(|| HandlerError::MissingFragment(id.to_string()))?;
            for fragment in &list.fragments {
                Self::collect_fragment_keys(
                    fragment,
                    protocol,
                    &mut fragment_keys,
                    &mut children,
                    &mut requires_full_state,
                );
                let child_owner = matches!(
                    fragment.fragment.as_ref(),
                    Some(Fragment::Component(_) | Fragment::Route(_))
                );
                keys.extend(
                    fragment_keys.drain().filter(|key| {
                        child_owner || excluded.binary_search(&key.as_str()).is_err()
                    }),
                );
                let mut child_excluded = match fragment.fragment.as_ref() {
                    Some(Fragment::Component(_) | Fragment::Route(_)) => Vec::new(),
                    Some(Fragment::Render(render)) => {
                        if render.alias.is_empty() {
                            Vec::new()
                        } else {
                            vec![render.alias.as_str()]
                        }
                    }
                    _ => excluded.clone(),
                };
                if let Some(Fragment::ForLoop(for_loop)) = fragment.fragment.as_ref() {
                    if let Err(index) = child_excluded.binary_search(&for_loop.item.as_str()) {
                        child_excluded.insert(index, &for_loop.item);
                    }
                }
                for child in children.drain(..) {
                    pending.push((child, child_excluded.clone()));
                }
                if keys.len() > limit {
                    return Err(state_key_limit_error(limit));
                }
            }
        }
        let mut keys: Vec<Box<str>> = keys.into_iter().map(String::into_boxed_str).collect();
        keys.sort_unstable();
        Ok(ContinuationStatePlan {
            keys: Arc::from(keys),
            requires_full_state,
        })
    }

    fn collect_fragment_keys<'a>(
        fragment: &'a WebUIFragment,
        protocol: &'a WebUIProtocol,
        keys: &mut HashSet<String>,
        pending: &mut Vec<&'a str>,
        requires_full_state: &mut bool,
    ) {
        match fragment.fragment.as_ref() {
            Some(Fragment::Signal(signal)) => {
                if structural_signal_value(signal).is_none() {
                    insert_top_level_key(keys, &signal.value);
                }
            }
            Some(Fragment::Component(component)) => {
                pending.push(&component.fragment_id);
                collect_component_hydration_keys(
                    &component.fragment_id,
                    protocol,
                    keys,
                    requires_full_state,
                );
            }
            Some(Fragment::ForLoop(for_loop)) => {
                insert_top_level_key(keys, &for_loop.collection);
                pending.push(&for_loop.fragment_id);
            }
            Some(Fragment::Render(render)) => {
                if !render.scope.is_empty() {
                    insert_top_level_key(keys, &render.scope);
                }
                pending.push(&render.fragment_id);
            }
            Some(Fragment::IfCond(if_cond)) => {
                if let Some(condition) = if_cond.condition.as_ref() {
                    collect_condition_keys(condition, keys);
                }
                pending.push(&if_cond.fragment_id);
            }
            Some(Fragment::Attribute(attribute)) => {
                if !attribute.raw_value && !attribute.value.is_empty() {
                    insert_top_level_key(keys, &attribute.value);
                }
                if let Some(condition) = attribute.condition_tree.as_ref() {
                    collect_condition_keys(condition, keys);
                }
                if !attribute.template.is_empty() {
                    pending.push(&attribute.template);
                }
            }
            Some(Fragment::Boundary(boundary)) => {
                if let Some(key) = boundary.key.as_deref() {
                    insert_top_level_key(keys, key);
                }
            }
            Some(Fragment::Route(route)) => {
                collect_route_keys(route, protocol, keys, pending, requires_full_state);
            }
            _ => {}
        }
    }
}

fn collect_route_keys<'a>(
    route: &'a WebUiFragmentRoute,
    protocol: &WebUIProtocol,
    keys: &mut HashSet<String>,
    pending: &mut Vec<&'a str>,
    requires_full_state: &mut bool,
) {
    let mut routes = vec![route];
    while let Some(current) = routes.pop() {
        if !current.content_fragment_id.is_empty() {
            pending.push(&current.content_fragment_id);
        }
        for component in [
            &current.fragment_id,
            &current.pending_component,
            &current.error_component,
        ] {
            if component.is_empty() {
                continue;
            }
            pending.push(component);
            collect_component_hydration_keys(component, protocol, keys, requires_full_state);
        }
        routes.extend(current.children.iter());
    }
}

fn collect_component_hydration_keys(
    component: &str,
    protocol: &WebUIProtocol,
    keys: &mut HashSet<String>,
    requires_full_state: &mut bool,
) {
    if *requires_full_state {
        return;
    }
    let Some(data) = protocol.components.get(component) else {
        *requires_full_state = true;
        return;
    };
    let mode = data.hydration_mode;
    if mode == StateProjectionMode::All as i32 {
        *requires_full_state = true;
    } else if mode == StateProjectionMode::Keys as i32
        || (mode == StateProjectionMode::None as i32 && !data.hydration_keys.is_empty())
    {
        keys.extend(data.hydration_keys.iter().cloned());
    } else if mode != StateProjectionMode::None as i32 {
        *requires_full_state = true;
    }
}

fn collect_condition_keys(condition: &ConditionExpr, keys: &mut HashSet<String>) {
    let mut pending = vec![condition];
    while let Some(current) = pending.pop() {
        match current.expr.as_ref() {
            Some(condition_expr::Expr::Identifier(identifier)) => {
                insert_top_level_key(keys, &identifier.value);
            }
            Some(condition_expr::Expr::Predicate(predicate)) => {
                insert_top_level_key(keys, &predicate.left);
                insert_top_level_key(keys, &predicate.right);
            }
            Some(condition_expr::Expr::Not(not)) => {
                if let Some(inner) = not.condition.as_deref() {
                    pending.push(inner);
                }
            }
            Some(condition_expr::Expr::Compound(compound)) => {
                if let Some(left) = compound.left.as_deref() {
                    pending.push(left);
                }
                if let Some(right) = compound.right.as_deref() {
                    pending.push(right);
                }
            }
            None => {}
        }
    }
}

fn insert_top_level_key(keys: &mut HashSet<String>, raw: &str) {
    let trimmed = raw.trim();
    let path = trimmed
        .strip_prefix("{{")
        .and_then(|value| value.strip_suffix("}}"))
        .map_or(trimmed, str::trim);
    let Some(first) = path.split('.').next() else {
        return;
    };
    if first.is_empty()
        || first.bytes().all(|byte| byte.is_ascii_digit())
        || matches!(first, "true" | "false" | "null")
    {
        return;
    }
    keys.insert(first.to_string());
}

/// Resolve a compiled record ID to its dense slot for a continuation frame.
fn fragment_slot(protocol: &crate::Protocol, id: &str) -> Result<u32> {
    protocol
        .fragment_slot(id)
        .ok_or_else(|| HandlerError::MissingFragment(id.to_string()))
}

#[inline(never)]
fn step_shared_repeat(
    source: &crate::state_view::SharedValue,
    cursor: (&str, usize),
    loops: &mut crate::render_scope::SharedBindings,
) -> bool {
    let (name, index) = cursor;
    let Some(item) = source.item(index) else {
        return false;
    };
    if let Some(slot) = loops.get_mut(name) {
        *slot = item;
    } else {
        loops.insert(name.to_owned(), item);
    }
    true
}

fn prepared_slot(target: Option<usize>, protocol: &crate::Protocol, id: &str) -> Result<u32> {
    match target {
        Some(target) => u32::try_from(target).map_err(|_| missing_scope_error()),
        None => fragment_slot(protocol, id),
    }
}

/// Park a freshly entered record, pre-selecting its best route match.
///
/// The caller has already resolved this record's execution slices.
fn open_fragment(
    entry: Entry,
    list: &crate::RenderFragmentView<'_>,
    context: &WebUIProcessContext<'_, '_, '_>,
    route_matches: &mut Vec<SelectedRoute>,
) -> Result<FragmentFrame> {
    let best_route = list
        .has_routes()
        .then(|| {
            crate::route_renderer::find_best_route_match_ref(
                list.fragments,
                context.request_path,
                &context.route_base,
                context.route_index,
            )
        })
        .flatten();
    let has_best_route = best_route.is_some();
    if let Some((index, _, matched)) = best_route {
        let target = list.target(index).ok_or_else(missing_scope_error)?;
        crate::render_buffer::push(
            route_matches,
            SelectedRoute {
                slot: u32::try_from(target).map_err(|_| missing_scope_error())?,
                consumed_segments: matched.consumed_segments,
            },
        );
    }
    Ok(FragmentFrame {
        slot: entry.slot,
        index: 0,
        best_route: has_best_route,
        ordinary_routes: entry.ordinary_routes,
    })
}

/// Resolve execution slices on entry; consecutive repeat items reuse this view.
fn render_fragment<'data>(
    context: &WebUIProcessContext<'data, '_, '_>,
    slot: u32,
) -> Result<crate::RenderFragmentView<'data>> {
    context
        .render_fragments
        .view(slot as usize)
        .ok_or_else(|| unknown_fragment_slot_error(slot))
}

#[cold]
#[inline(never)]
fn unknown_fragment_slot_error(slot: u32) -> HandlerError {
    HandlerError::Invariant(format!("continuation frame references unknown slot {slot}"))
}

#[cold]
#[inline(never)]
fn missing_yield_error() -> HandlerError {
    HandlerError::Invariant("continuation yielded without a stream status".to_owned())
}

#[cold]
#[inline(never)]
fn missing_streaming_vm_error() -> HandlerError {
    HandlerError::Invariant("ordinary cursor reached a streaming-only operation".into())
}

#[cold]
#[inline(never)]
fn missing_if_condition_error() -> HandlerError {
    HandlerError::Rendering("if fragment is missing its condition".to_string())
}

#[cold]
#[inline(never)]
fn non_array_collection_error(collection: &str) -> HandlerError {
    HandlerError::TypeError(format!("collection `{collection}` is not an array"))
}

#[cold]
#[inline(never)]
fn route_chain_limit_error() -> HandlerError {
    HandlerError::Invariant("route chain index overflowed usize".to_string())
}

#[cold]
#[inline(never)]
fn state_key_limit_error(limit: usize) -> HandlerError {
    HandlerError::StreamingBoundary(Box::new(crate::StreamingBoundaryError {
        signal: "state snapshot".to_string(),
        reason: format!(
            "continuation state projection exceeds {limit} top-level keys; split the entry or tighten component hydration projections"
        ),
    }))
}

#[cold]
#[inline(never)]
fn missing_scope_error() -> HandlerError {
    HandlerError::Invariant("continuation scope stack was not balanced".to_owned())
}

#[cold]
#[inline(never)]
fn fragment_scope_error(render: &webui_protocol::WebUiFragmentRender) -> HandlerError {
    HandlerError::FragmentScopeMissing(Box::new(crate::FragmentScopeError {
        fragment_id: render.fragment_id.clone(),
        scope: render.scope.clone(),
    }))
}

#[cold]
#[inline(never)]
fn fragment_depth_error() -> HandlerError {
    HandlerError::FragmentCallDepth {
        limit: crate::MAX_FRAGMENT_CALL_DEPTH,
    }
}

#[cold]
#[inline(never)]
fn fragment_budget_error() -> HandlerError {
    HandlerError::FragmentCallBudget {
        limit: crate::MAX_FRAGMENT_INVOCATIONS,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use webui_protocol::{ComponentData, FragmentList};

    #[test]
    fn continuation_frames_keep_payloads_out_of_common_stack() {
        assert!(std::mem::size_of::<Frame>() <= 32);
        assert!(std::mem::size_of::<SelectedRoute>() <= 16);
        assert!(std::mem::size_of::<OutletFrame>() <= 32);
        assert!(std::mem::size_of::<RouteWork>() <= 40);
    }

    #[test]
    fn outlet_policy_carriers_fit_existing_layouts() {
        use std::mem::{align_of, size_of};

        assert_eq!(size_of::<Entry>(), size_of::<Option<u32>>());
        assert_eq!(size_of::<Option<Entry>>(), size_of::<Option<u32>>());
        assert_eq!(align_of::<Option<Entry>>(), align_of::<Option<u32>>());
        assert_eq!(
            size_of::<OutletSelection>(),
            size_of::<Option<SelectedRoute>>()
        );
        assert_eq!(
            align_of::<OutletSelection>(),
            align_of::<Option<SelectedRoute>>()
        );
        assert_eq!(size_of::<StepRuntime<'_, '_>>(), 3 * size_of::<usize>());
        assert_eq!(
            size_of::<ContinuationVm<OrdinaryVm>>(),
            4 * size_of::<Vec<Frame>>()
                + size_of::<Option<u32>>()
                + size_of::<Option<usize>>()
                + 3 * size_of::<usize>()
        );
        #[cfg(target_pointer_width = "32")]
        {
            assert!(size_of::<Frame>() <= 20);
            assert!(size_of::<FragmentFrame>() <= 12);
            assert!(size_of::<OutletFrame>() <= 20);
            assert!(size_of::<RouteWork>() <= 24);
        }
        #[cfg(target_pointer_width = "64")]
        {
            assert!(size_of::<Frame>() <= 32);
            assert!(size_of::<FragmentFrame>() <= 16);
            assert!(size_of::<OutletFrame>() <= 32);
            assert!(size_of::<RouteWork>() <= 40);
        }
    }

    #[test]
    fn direct_entries_and_queued_entries_preserve_order_and_full_slot_width() -> Result<()> {
        let mut streaming = ContinuationVm::from_slot(0, 0);
        assert_eq!(
            streaming.next_entry,
            Some(Entry {
                slot: 0,
                ordinary_routes: false
            })
        );
        assert_eq!(
            ContinuationVm::ordinary(0, 0).next_entry,
            Some(Entry {
                slot: 0,
                ordinary_routes: true
            })
        );
        for slot in [0, u32::MAX] {
            for ordinary_routes in [false, true] {
                streaming.enter(slot, ordinary_routes)?;
                assert!(matches!(
                    streaming.next_frame(),
                    Some(Frame::EnterFragment(entry))
                        if entry.slot == slot && entry.ordinary_routes == ordinary_routes
                ));
                assert!(streaming.next_entry.is_none());
            }
        }
        Ok(())
    }

    #[test]
    fn ordinary_cursor_has_no_streaming_payload() {
        assert_eq!(std::mem::size_of::<OrdinaryVm>(), 0);
        assert!(std::mem::size_of::<ContinuationVm<OrdinaryVm>>() <= 144);
        assert!(
            std::mem::size_of::<ContinuationVm>()
                >= std::mem::size_of::<ContinuationVm<OrdinaryVm>>() + 256
        );
        let vm = ContinuationVm::ordinary(0, 0);
        assert!(vm.mode.streaming().is_none());
        assert!(vm.streaming().is_err());
    }

    #[test]
    fn entry_cursor_does_not_allocate_continuation_frames() -> Result<()> {
        let protocol = crate::Protocol::new(WebUIProtocol::new(HashMap::from([(
            "entry".to_owned(),
            FragmentList::default(),
        )])));
        let slot = protocol
            .fragment_slot("entry")
            .ok_or_else(missing_scope_error)?;
        let vm = ContinuationVm::ordinary(slot, 0);
        assert_eq!(vm.frames.capacity(), 0);
        assert_eq!(
            vm.next_entry.map(|entry| entry.slot),
            protocol.fragment_slot("entry")
        );
        Ok(())
    }

    #[test]
    fn repeat_marker_stays_parked_between_item_records() -> Result<()> {
        let mut vm = ContinuationVm::ordinary(0, 0);
        vm.next_entry = None;
        vm.push(Frame::Repeat(true))?;
        for slot in 1..4 {
            assert!(matches!(vm.next_frame(), Some(Frame::Repeat(true))));
            assert_eq!(vm.frames.len(), 1);
            vm.enter(slot, true)?;
            assert!(
                matches!(vm.next_frame(), Some(Frame::EnterFragment(found)) if found.slot == slot)
            );
            assert_eq!(vm.frames.len(), 1);
        }
        Ok(())
    }

    #[test]
    fn streaming_repeat_marker_retains_its_body_order() -> Result<()> {
        for ordinary_routes in [false, true] {
            let mut vm = ContinuationVm::from_slot(0, 0);
            vm.next_entry = None;
            vm.push(Frame::Repeat(ordinary_routes))?;
            for slot in 1..4 {
                assert!(matches!(
                    vm.next_frame(),
                    Some(Frame::Repeat(found)) if found == ordinary_routes
                ));
                vm.enter(slot, ordinary_routes)?;
                assert!(matches!(
                    vm.next_frame(),
                    Some(Frame::EnterFragment(found))
                        if found.slot == slot && found.ordinary_routes == ordinary_routes
                ));
                assert_eq!(vm.frames.len(), 1);
            }
        }
        Ok(())
    }

    #[test]
    fn same_record_restarts_and_parent_returns_restore_entry_policy() -> Result<()> {
        for ordinary_routes in [false, true] {
            let mut frame = FragmentFrame {
                slot: 7,
                index: 4,
                best_route: false,
                ordinary_routes: !ordinary_routes,
            };
            frame.restart::<StreamingVmState>(ordinary_routes);
            assert_eq!(frame.index, 0);
            assert_eq!(frame.slot, 7);
            assert_eq!(frame.ordinary_routes, ordinary_routes);
            assert!(!frame.best_route);

            let mut vm = ContinuationVm::from_slot(0, 0);
            vm.next_entry = None;
            frame.index = 9;
            vm.push(Frame::Fragment(frame))?;
            let parent = vm
                .resume_parent_fragment()
                .ok_or_else(missing_scope_error)?;
            assert_eq!(parent.slot, frame.slot);
            assert_eq!(parent.index, 9);
            assert_eq!(parent.ordinary_routes, ordinary_routes);
        }
        Ok(())
    }

    #[test]
    fn parent_record_resumes_only_after_scope_and_route_unwinding() -> Result<()> {
        for barrier in [
            Frame::IfEnd { slot: 3 },
            Frame::RenderEnd { slot: 3 },
            Frame::ComponentEnd(ComponentEndFrame {
                component_slot: 3,
                owns_css_tree: true,
                saved_scope: true,
                previous_input_owner: None,
            }),
            Frame::RouteWork,
            Frame::Repeat(true),
        ] {
            let mut vm = ContinuationVm::ordinary(0, 0);
            vm.next_entry = None;
            vm.push(Frame::Fragment(FragmentFrame {
                slot: 7,
                index: 4,
                best_route: true,
                ordinary_routes: true,
            }))?;
            vm.push(barrier)?;
            assert!(vm.resume_parent_fragment().is_none());
            assert_eq!(vm.frames.len(), 2);
            vm.frames.pop();
            assert!(matches!(
                vm.resume_parent_fragment(),
                Some(FragmentFrame {
                    slot: 7,
                    index: 4,
                    best_route: true,
                    ordinary_routes: true,
                })
            ));
            assert!(vm.frames.is_empty());
            assert!(vm.resume_parent_fragment().is_none());
        }
        Ok(())
    }

    #[test]
    fn parked_repeat_counts_toward_physical_streaming_depth() -> Result<()> {
        let mut vm = ContinuationVm::from_slot(0, 0);
        for _ in 0..MAX_CONTINUATION_DEPTH {
            vm.push(Frame::Repeat(false))?;
        }
        assert!(vm.push(Frame::Repeat(false)).is_err());
        assert!(vm.enter(1, false).is_err());
        vm.frames.pop();
        vm.enter(1, false)?;
        assert_eq!(
            vm.frames.len() + usize::from(vm.next_entry.is_some()),
            MAX_CONTINUATION_DEPTH
        );
        Ok(())
    }

    #[test]
    fn recursive_projection_maps_input_roots_and_excludes_aliases() -> Result<()> {
        let fragments = [
            ("entry", vec![WebUIFragment::render("node", "tree", "row")]),
            (
                "node",
                vec![
                    WebUIFragment::signal("row.label", false),
                    WebUIFragment::signal("title", false),
                    WebUIFragment::for_loop("child", "row.children", "item"),
                ],
            ),
            ("item", vec![WebUIFragment::render("node", "child", "row")]),
        ]
        .into_iter()
        .map(|(id, fragments)| {
            (
                id.to_owned(),
                FragmentList {
                    fragments,
                    contains_boundary: false,
                },
            )
        })
        .collect();
        let mut protocol = WebUIProtocol::new(fragments);
        protocol.initial_state_strategy = InitialStateStrategy::Components as i32;
        let plan = ContinuationVm::collect_state_keys(&protocol, "entry", 16)?;
        assert!(!plan.requires_full_state);
        assert_eq!(
            plan.keys.iter().map(Box::as_ref).collect::<Vec<_>>(),
            ["$webui", "title", "tree"]
        );
        Ok(())
    }

    #[test]
    fn projection_revisits_shared_body_with_distinct_alias_roots() -> Result<()> {
        let fragments = [
            (
                "entry",
                vec![
                    WebUIFragment::render("body", "first", "a"),
                    WebUIFragment::render("body", "second", "b"),
                ],
            ),
            (
                "body",
                vec![
                    WebUIFragment::signal("a.label", false),
                    WebUIFragment::signal("b.label", false),
                ],
            ),
        ]
        .into_iter()
        .map(|(id, fragments)| {
            (
                id.to_owned(),
                FragmentList {
                    fragments,
                    contains_boundary: false,
                },
            )
        })
        .collect();
        let mut protocol = WebUIProtocol::new(fragments);
        protocol.initial_state_strategy = InitialStateStrategy::Components as i32;
        let plan = ContinuationVm::collect_state_keys(&protocol, "entry", 16)?;
        assert_eq!(
            plan.keys.iter().map(Box::as_ref).collect::<Vec<_>>(),
            ["$webui", "a", "b", "first", "second"]
        );
        Ok(())
    }

    #[test]
    fn route_key_collection_covers_generated_hosts_content_and_children() -> Result<()> {
        let child = WebUiFragmentRoute {
            path: "child".to_string(),
            fragment_id: "child-page".to_string(),
            ..Default::default()
        };
        let route = WebUiFragmentRoute {
            path: "/".to_string(),
            fragment_id: "route-page".to_string(),
            content_fragment_id: "route-content".to_string(),
            pending_component: "pending-card".to_string(),
            error_component: "error-card".to_string(),
            children: vec![child],
            ..Default::default()
        };
        let mut fragments = HashMap::from([
            (
                "index.html".to_string(),
                FragmentList {
                    fragments: vec![WebUIFragment {
                        fragment: Some(Fragment::Route(route)),
                    }],
                    contains_boundary: true,
                },
            ),
            (
                "route-content".to_string(),
                FragmentList {
                    fragments: vec![WebUIFragment::signal("contentState", false)],
                    contains_boundary: true,
                },
            ),
        ]);
        for tag in ["route-page", "pending-card", "error-card", "child-page"] {
            fragments.insert(tag.to_string(), FragmentList::default());
        }
        let mut protocol = WebUIProtocol::new(fragments);
        protocol.initial_state_strategy = InitialStateStrategy::Components as i32;
        for (tag, key) in [
            ("route-page", "routeHydration"),
            ("pending-card", "pendingHydration"),
            ("error-card", "errorHydration"),
            ("child-page", "childHydration"),
        ] {
            protocol.components.insert(
                tag.to_string(),
                ComponentData {
                    hydration_mode: StateProjectionMode::Keys as i32,
                    hydration_keys: vec![key.to_string()],
                    ..Default::default()
                },
            );
        }

        let plan = ContinuationVm::collect_state_keys(&protocol, "index.html", 16)?;
        assert!(!plan.requires_full_state);
        assert_eq!(
            plan.keys.iter().map(Box::as_ref).collect::<Vec<_>>(),
            [
                "$webui",
                "childHydration",
                "contentState",
                "errorHydration",
                "pendingHydration",
                "routeHydration",
            ]
        );
        Ok(())
    }

    /// A hand-built protocol that puts a boundary in a repeat body is rejected
    /// by the VM rather than retained as resumable repeat state.
    ///
    /// The parser rejects this at build time, but the handler decodes protocols
    /// it did not build (FFI, WASM, cached artifacts), so the continuation
    /// defends the invariant it relies on.
    #[test]
    fn boundary_discovered_inside_a_repeat_is_rejected() {
        let fragments = HashMap::from([
            (
                "index.html".to_string(),
                FragmentList {
                    fragments: vec![
                        WebUIFragment::signal("$structural:head_start", true),
                        WebUIFragment {
                            fragment: Some(Fragment::ForLoop(WebUIFragmentFor {
                                item: "item".to_string(),
                                collection: "items".to_string(),
                                fragment_id: "for-1".to_string(),
                            })),
                        },
                        WebUIFragment::signal("$structural:body_end", true),
                    ],
                    contains_boundary: true,
                },
            ),
            (
                "for-1".to_string(),
                FragmentList {
                    fragments: vec![WebUIFragment {
                        fragment: Some(Fragment::Boundary(WebUIFragmentBoundary {
                            declaration_id: 0,
                            owner_fragment_id: "index.html".to_string(),
                            name: "row".to_string(),
                            key: None,
                            may_repeat: false,
                            phase: BoundaryPhase::Start as i32,
                        })),
                    }],
                    contains_boundary: true,
                },
            ),
        ]);
        let protocol = crate::Protocol::new(WebUIProtocol::new(fragments));
        let handler = WebUIHandler::new();
        let mut sink = super::super::BufferSink::default();
        let options = crate::RenderOptions::new("index.html", "/");
        let mut response = match handler.stream_response(&protocol, &options, &mut sink) {
            Ok(response) => response,
            Err(error) => panic!("building the response failed: {error}"),
        };
        let error = response
            .start(&serde_json::json!({ "items": [1, 2] }))
            .expect_err("a boundary inside a repeat must be rejected");
        assert!(
            error.to_string().contains("<for> repeat body"),
            "unexpected error: {error}"
        );
    }
}

// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

//! Iterative continuation VM for runtime boundary discovery.

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
use crate::route_matcher::RouteMatch;
use crate::{
    structural_signal_value, HandlerError, Result, WebUIHandler, WebUIProcessContext,
    STATE_INJECT_KEY,
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
/// Records kept resolved while one semantic step walks the graph.
///
/// A step touches the record it entered, the parent it returns to, and at most
/// a couple of enclosing hosts, so four entries cover the common continuation
/// without turning the probe into a search.
const RECORD_CACHE_SIZE: usize = 4;

/// Bounded slot→record cache scoped to a single [`ContinuationVm::advance`].
///
/// Resolving a slot costs a dense-vector read plus a hash of the compiled
/// record ID, and a step re-resolves the same few records every time it
/// descends into a child and unwinds back to the parked parent. Caching the
/// borrow for the duration of one step collapses those repeats to a handful of
/// integer comparisons while keeping the VM itself lifetime-free between calls.
struct RecordCache<'data> {
    entries: [Option<(u32, &'data webui_protocol::FragmentList)>; RECORD_CACHE_SIZE],
    next: usize,
}

impl<'data> RecordCache<'data> {
    const fn new() -> Self {
        Self {
            entries: [None; RECORD_CACHE_SIZE],
            next: 0,
        }
    }

    /// Borrow the record for `slot`, resolving and retaining it on a miss.
    fn record(
        &mut self,
        protocol: &'data crate::Protocol,
        slot: u32,
    ) -> Result<&'data webui_protocol::FragmentList> {
        for (cached, list) in self.entries.iter().flatten() {
            if *cached == slot {
                return Ok(list);
            }
        }
        let list = slot_fragment(protocol, slot)?;
        self.entries[self.next] = Some((slot, list));
        self.next = (self.next + 1) % RECORD_CACHE_SIZE;
        Ok(list)
    }
}

pub(crate) struct ContinuationVm {
    frames: Vec<Frame>,
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
    /// Repeats currently being walked by this step.
    ///
    /// A boundary can never execute inside a repeat, so this is zero at every
    /// point the VM hands control back to the host. Tracking it as a counter
    /// makes that an O(1) checked invariant rather than an assumption about a
    /// protocol the handler did not build.
    open_repeats: usize,
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
    EnterFragment(u32),
    Fragment(FragmentFrame),
    ComponentEnd(ComponentEndFrame),
    IfEnd {
        slot: u32,
    },
    Repeat(RepeatFrame),
    GeneratedComponentStart {
        tag: Box<str>,
    },
    GeneratedComponentEnd {
        tag: Box<str>,
        spanning: bool,
    },
    RouteEnd {
        saved_route_base: Box<str>,
        saved_route_children: Vec<WebUiFragmentRoute>,
    },
    Outlet(OutletFrame),
}

struct ComponentEndFrame {
    saved_local_vars: HashMap<String, Value>,
    component_slot: u32,
    owns_css_tree: bool,
}

struct FragmentFrame {
    slot: u32,
    /// Prepared render slot for the same fragment list, used to read the
    /// per-fragment metadata prepared when the protocol was loaded.
    render_slot: usize,
    index: usize,
    best_route: Option<(String, RouteMatch)>,
}

/// One in-flight `<for>` repeat.
///
/// A repeat can never contain a boundary — the build rejects that with
/// `boundary-in-repeat` and [`ContinuationVm::discover_boundary`] rejects a
/// hand-built protocol that tries — so this frame is drained inside the step
/// that created it and is never retained across a host call. Closing the
/// previous item and opening the next share one frame, so a repeat costs two
/// pushes per item instead of three without widening the frame: `index` is both
/// the next item to open and, when non-zero, one past the item still open.
struct RepeatFrame {
    slot: u32,
    item_name: Box<str>,
    items: std::vec::IntoIter<Value>,
    index: usize,
    saved_value: Option<Value>,
}

struct OutletFrame {
    routes: Vec<WebUiFragmentRoute>,
    index: usize,
    best: Option<(usize, RouteMatch)>,
}

/// What the current [`ContinuationVm::advance`] call is walking toward.
#[derive(Clone, Copy, Eq, PartialEq)]
pub(crate) enum StepGoal {
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
}

impl ContinuationVm {
    pub(crate) fn new(entry_id: &str, protocol: &crate::Protocol) -> Result<Self> {
        protocol_fragment(protocol.protocol(), entry_id)?;
        let entry_slot = protocol
            .fragment_slot(entry_id)
            .ok_or_else(|| HandlerError::MissingFragment(entry_id.to_string()))?;
        let mut frames = Vec::with_capacity(INITIAL_FRAME_CAPACITY);
        frames.push(Frame::EnterFragment(entry_slot));
        Ok(Self {
            frames,
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
            component_count: protocol.component_index().len(),
            pending_span_candidate: None,
            open_repeats: 0,
        })
    }

    pub(crate) fn validate_resume(&self, instance_id: BoundaryInstanceId) -> Result<()> {
        let Some(pending) = self.pending.as_ref() else {
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
        if mode == BoundaryMode::Updatable && self.updatable_count >= MAX_UPDATABLE_OCCURRENCES {
            return Err(updatable_limit_error(MAX_UPDATABLE_OCCURRENCES));
        }
        Ok(())
    }

    pub(crate) fn validate_update(&self, instance_id: BoundaryInstanceId) -> Result<usize> {
        let index = instance_id.index()?;
        let Some(mode) = self.committed_modes.get(index) else {
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
        let Some(pending) = self.pending.take() else {
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
        self.active = Some(ActiveBoundary {
            instance_id: pending.instance_id,
            declaration_id: pending.declaration_id,
            mode,
        });
        Ok(())
    }

    /// Walk the continuation until `goal` is met.
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
        let mut records = RecordCache::new();
        let runtime = StepRuntime {
            goal,
            handler,
            protocol,
        };
        while let Some(frame) = self.frames.pop() {
            match frame {
                Frame::EnterFragment(slot) => {
                    let list = records.record(protocol, slot)?;
                    let frame = open_fragment(slot, list, context);
                    if let Some(status) = self.run_fragment(frame, list, runtime, context)? {
                        return Ok(status);
                    }
                }
                Frame::Fragment(frame) => {
                    let list = records.record(protocol, frame.slot)?;
                    if let Some(status) = self.run_fragment(frame, list, runtime, context)? {
                        return Ok(status);
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
                Frame::Repeat(frame) => self.step_repeat(frame, protocol, context)?,
                Frame::GeneratedComponentStart { tag } => {
                    self.start_generated_component(tag, handler, protocol, context)?;
                }
                Frame::GeneratedComponentEnd { tag, spanning } => {
                    context.writer.write("</")?;
                    context.writer.write(&tag)?;
                    context.writer.write(">")?;
                    if spanning {
                        self.finish_span(&tag, handler, context)?;
                    }
                }
                Frame::RouteEnd {
                    saved_route_base,
                    saved_route_children,
                } => {
                    context.writer.write("</webui-route>")?;
                    context.route_base = Cow::Owned(saved_route_base.into_string());
                    context.route_children = saved_route_children;
                }
                Frame::Outlet(frame) => self.step_outlet(frame, handler, protocol, context)?,
            }
        }

        if self.pending.is_some() || self.active.is_some() {
            return Err(HandlerError::Invariant(
                "pending boundary lost its continuation".to_string(),
            ));
        }
        if self.open_repeats != 0 {
            return Err(HandlerError::Invariant(
                "traversal completed while a repeat was still open".to_string(),
            ));
        }
        if !self.open_spans.is_empty() {
            return Err(malformed_span_signal_error(
                "component span",
                "traversal completed before every component span closed",
            ));
        }
        if self.pending_span_candidate.is_some() {
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
        increment_streaming_record_sequence("terminal", streaming_state(context)?)?;
        context.writer.end()?;
        Ok(StreamStatus {
            boundary: None,
            done: true,
        })
    }

    /// Walk one fragment record until it descends, suspends, commits, or ends.
    ///
    /// The record is resolved by the caller — once per step for the whole
    /// descend/unwind cycle — and inert fragments never touch the frame stack,
    /// so a boundary body costs no record lookups at all. Only a construct that
    /// owns a child record (component, condition, loop, route, outlet), a
    /// discovered boundary, or the checkpoint that ends a committed boundary
    /// parks the frame and returns to the caller.
    fn run_fragment<'data>(
        &mut self,
        mut frame: FragmentFrame,
        list: &'data webui_protocol::FragmentList,
        runtime: StepRuntime<'_, 'data>,
        context: &mut WebUIProcessContext<'data, '_, '_>,
    ) -> Result<Option<StreamStatus>> {
        let StepRuntime {
            goal,
            handler,
            protocol,
        } = runtime;
        loop {
            let index = frame.index;
            let Some(fragment) = list.fragments.get(index) else {
                super::ensure_no_pending_streaming_root(
                    context,
                    "the end of the containing fragment",
                )?;
                return Ok(None);
            };
            super::validate_pending_streaming_root(fragment, context)?;
            super::validate_streaming_root_opening(&list.fragments[..index], fragment)?;
            frame.index = index + 1;

            match fragment.fragment.as_ref() {
                Some(Fragment::Raw(raw)) => context.writer.write(&raw.value)?,
                Some(Fragment::Signal(signal)) => {
                    self.process_signal(signal, handler, context)?;
                }
                Some(Fragment::Attribute(attribute)) => {
                    let prepared = context.render_fragments.list(frame.render_slot);
                    handler.process_attribute(
                        attribute,
                        prepared.and_then(|prepared| prepared.target(index)),
                        prepared.and_then(|prepared| prepared.component_attr_name(index)),
                        context,
                    )?;
                }
                Some(Fragment::Plugin(plugin)) => {
                    if let Some(active) = context.plugin.as_mut() {
                        active.on_element_data(&plugin.data, context.writer)?;
                    }
                }
                Some(Fragment::Boundary(boundary)) => {
                    if boundary.phase() == BoundaryPhase::End {
                        self.finish_boundary(boundary, handler, context)?;
                        if goal == StepGoal::CommitBoundary {
                            // The checkpoint just flushed, so the committed
                            // occurrence ends this step: the parent bytes that
                            // follow belong to the caller's next `advance`.
                            self.push(Frame::Fragment(frame))?;
                            return Ok(Some(StreamStatus {
                                boundary: None,
                                done: false,
                            }));
                        }
                        continue;
                    }
                    let descriptor =
                        self.discover_boundary(boundary, handler, protocol, context)?;
                    self.push(Frame::Fragment(frame))?;
                    return Ok(Some(StreamStatus {
                        boundary: Some(descriptor),
                        done: false,
                    }));
                }
                Some(Fragment::Component(component)) => {
                    self.push(Frame::Fragment(frame))?;
                    self.begin_component(
                        component,
                        ComponentHostOrigin::ParserProduced,
                        (handler, protocol),
                        context,
                    )?;
                    return Ok(None);
                }
                Some(Fragment::IfCond(if_cond)) => {
                    self.push(Frame::Fragment(frame))?;
                    self.begin_if(if_cond, handler, protocol, context)?;
                    return Ok(None);
                }
                Some(Fragment::ForLoop(for_loop)) => {
                    self.push(Frame::Fragment(frame))?;
                    self.begin_repeat(for_loop, handler, protocol, context)?;
                    return Ok(None);
                }
                Some(Fragment::Route(route)) => {
                    let route_match = frame
                        .best_route
                        .as_ref()
                        .filter(|(key, _)| *key == route.fragment_id)
                        .map(|(_, route_match)| route_match.clone());
                    self.push(Frame::Fragment(frame))?;
                    self.render_route(route, route_match, protocol, context)?;
                    return Ok(None);
                }
                Some(Fragment::Outlet(_)) => {
                    self.push(Frame::Fragment(frame))?;
                    self.begin_outlet(context)?;
                    return Ok(None);
                }
                None => {}
            }
        }
    }

    fn process_signal<'data>(
        &mut self,
        signal: &'data webui_protocol::WebUIFragmentSignal,
        handler: &WebUIHandler,
        context: &mut WebUIProcessContext<'data, '_, '_>,
    ) -> Result<()> {
        if let Some(value) = structural_signal_value(signal) {
            if let Some(tag) = value.strip_prefix(SPAN_START_PREFIX) {
                if self.pending_span_candidate.is_some() {
                    return Err(malformed_span_signal_error(
                        tag,
                        "a component span start arrived before the previous host opening completed",
                    ));
                }
                context.writer.stream_begin_component()?;
                self.pending_span_candidate = Some(tag.into());
                return Ok(());
            }
            if let Some(tag) = value.strip_prefix(SPAN_END_PREFIX) {
                return self.finish_span(tag, handler, context);
            }
            if let Some(tag) = value.strip_prefix(super::root::STREAMING_ROOT_PREFIX) {
                if let Some(candidate) = self.pending_span_candidate.as_ref() {
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
        component: &WebUIFragmentComponent,
        origin: ComponentHostOrigin,
        runtime: (&WebUIHandler, &'data crate::Protocol),
        context: &mut WebUIProcessContext<'data, '_, '_>,
    ) -> Result<()> {
        let (handler, protocol) = runtime;
        if let Some(candidate) = self.pending_span_candidate.take() {
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
                let Some(open) = self.open_spans.last() else {
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
            }
            self.record_component(&component.fragment_id, context)?;
        }

        if !context.rendered_components.contains(&component.fragment_id) {
            handler.emit_css_module(component, context)?;
            context
                .rendered_components
                .insert(component.fragment_id.clone());
        }
        let slot = fragment_slot(protocol, &component.fragment_id)?;
        let owns_css_tree =
            WebUIHandler::component_owns_css_tree(&component.fragment_id, protocol.protocol());
        if owns_css_tree {
            WebUIHandler::push_shadow_style_root(&component.fragment_id, context)?;
        }
        let saved_local_vars = std::mem::take(&mut context.local_vars);
        let saved_component_attrs = std::mem::replace(
            &mut context.component_attrs,
            crate::take_scope_map(&mut context.scope_pool),
        );
        context.local_vars = saved_component_attrs;
        context.collecting_component_attrs = false;
        if let Some(plugin) = context.plugin.as_mut() {
            plugin.push_scope();
        }
        self.push(Frame::ComponentEnd(ComponentEndFrame {
            saved_local_vars,
            component_slot: slot,
            owns_css_tree,
        }))?;
        self.push(Frame::EnterFragment(slot))?;
        Ok(())
    }

    fn end_component(
        &mut self,
        frame: ComponentEndFrame,
        protocol: &crate::Protocol,
        context: &mut WebUIProcessContext<'_, '_, '_>,
    ) -> Result<()> {
        if let Some(plugin) = context.plugin.as_mut() {
            plugin.pop_scope();
        }
        let used_locals = std::mem::replace(&mut context.local_vars, frame.saved_local_vars);
        crate::recycle_scope_map(&mut context.scope_pool, used_locals);
        context.component_attrs.clear();
        context.collecting_component_attrs = false;
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
        if_cond: &WebUIFragmentIf,
        handler: &WebUIHandler,
        protocol: &crate::Protocol,
        context: &mut WebUIProcessContext<'_, '_, '_>,
    ) -> Result<()> {
        let condition = if_cond
            .condition
            .as_ref()
            .ok_or_else(missing_if_condition_error)?;
        let condition_met = handler.evaluate_condition(condition, context)?;
        if let Some(plugin) = context.plugin.as_mut() {
            plugin.on_if_start(&if_cond.fragment_id, context.writer)?;
        }
        if condition_met {
            if let Some(plugin) = context.plugin.as_mut() {
                plugin.push_scope();
            }
            let slot = fragment_slot(protocol, &if_cond.fragment_id)?;
            self.push(Frame::IfEnd { slot })?;
            self.push(Frame::EnterFragment(slot))?;
        } else if let Some(plugin) = context.plugin.as_mut() {
            plugin.on_if_end(&if_cond.fragment_id, context.writer)?;
        }
        Ok(())
    }

    /// Open a `<for>` repeat.
    ///
    /// The whole repeat is atomic: it can carry no boundary, so every frame it
    /// pushes is drained before this step returns to the host and the repeat
    /// never becomes resumable continuation state.
    fn begin_repeat(
        &mut self,
        for_loop: &WebUIFragmentFor,
        handler: &WebUIHandler,
        protocol: &crate::Protocol,
        context: &mut WebUIProcessContext<'_, '_, '_>,
    ) -> Result<()> {
        let items = match handler.resolve_value(&for_loop.collection, context) {
            Some(Value::Array(items)) => items,
            Some(_) => return Err(non_array_collection_error(&for_loop.collection)),
            None => Vec::new(),
        };
        if let Some(plugin) = context.plugin.as_mut() {
            plugin.on_for_start(&for_loop.fragment_id, context.writer)?;
        }
        let saved_value = context.local_vars.remove(&for_loop.item);
        if !items.is_empty() {
            context
                .local_vars
                .insert(for_loop.item.clone(), Value::Null);
        }
        self.open_repeats = self
            .open_repeats
            .checked_add(1)
            .ok_or_else(|| continuation_limit_error(MAX_CONTINUATION_DEPTH))?;
        self.push(Frame::Repeat(RepeatFrame {
            slot: fragment_slot(protocol, &for_loop.fragment_id)?,
            item_name: for_loop.item.clone().into(),
            items: items.into_iter(),
            index: 0,
            saved_value,
        }))
    }

    /// Close the item the repeat just rendered, then open the next one.
    fn step_repeat(
        &mut self,
        mut frame: RepeatFrame,
        protocol: &crate::Protocol,
        context: &mut WebUIProcessContext<'_, '_, '_>,
    ) -> Result<()> {
        // The frame is only re-pushed after an item opens, so a non-zero index
        // means the previous item's body has just finished.
        if let Some(open) = frame.index.checked_sub(1) {
            if let Some(plugin) = context.plugin.as_mut() {
                plugin.pop_scope();
                plugin.on_repeat_item_end(open, context.writer)?;
            }
        }
        if let Some(item) = frame.items.next() {
            let index = frame.index;
            frame.index = index.wrapping_add(1);
            if let Some(plugin) = context.plugin.as_mut() {
                plugin.on_repeat_item_start(index, context.writer)?;
                plugin.push_scope();
            }
            if let Some(entry) = context.local_vars.get_mut(frame.item_name.as_ref()) {
                *entry = item;
            }
            let slot = frame.slot;
            self.push(Frame::Repeat(frame))?;
            self.push(Frame::EnterFragment(slot))?;
            return Ok(());
        }
        match frame.saved_value {
            Some(value) => {
                context.local_vars.insert(frame.item_name.into(), value);
            }
            None => {
                context.local_vars.remove(frame.item_name.as_ref());
            }
        }
        self.open_repeats = self.open_repeats.saturating_sub(1);
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
        if self.pending.is_some()
            || self.active.is_some()
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
        if self.open_repeats != 0 {
            return Err(boundary_in_repeat_error(&boundary.name));
        }
        let index = usize::try_from(self.next_boundary_id)
            .map_err(|_| boundary_limit_error(MAX_BOUNDARY_OCCURRENCES))?;
        if index >= MAX_BOUNDARY_OCCURRENCES {
            return Err(boundary_limit_error(MAX_BOUNDARY_OCCURRENCES));
        }
        let instance_id = BoundaryInstanceId::from_raw(self.next_boundary_id);
        self.next_boundary_id = self
            .next_boundary_id
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
            if self.keyed_instance_count >= MAX_KEYED_INSTANCES {
                return Err(keyed_instance_limit_error(MAX_KEYED_INSTANCES));
            }
            let keys = self
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
            self.keyed_instance_count += 1;
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
        self.pending = Some(PendingBoundary {
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
        let Some(value) = handler.resolve_value(path, context) else {
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
        let Some(active) = self.active.take() else {
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
        increment_streaming_record_sequence("boundary", streaming_state(context)?)?;
        streaming_state(context)?.active_boundary = None;
        let expected = self.committed_modes.len();
        if active.instance_id.index()? != expected {
            return Err(HandlerError::Invariant(
                "committed boundary IDs are not gapless".to_string(),
            ));
        }
        // Counted here, not at resume: only an occurrence whose checkpoint
        // actually reached the client consumes the browser's retention budget.
        if active.mode == BoundaryMode::Updatable {
            self.updatable_count = self.updatable_count.saturating_add(1);
        }
        self.committed_modes.push(active.mode);
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
        if self.open_spans.len() >= MAX_SPAN_NESTING {
            return Err(span_nesting_error(MAX_SPAN_NESTING));
        }
        let id = SpanInstanceId::new(self.next_span_id);
        self.next_span_id = self
            .next_span_id
            .checked_add(1)
            .ok_or_else(span_id_overflow_error)?;
        if write_marker {
            super::write_range_marker(context.writer, "<!--ws:", id.raw())?;
        }
        let mut capture = self
            .capture_pool
            .pop()
            .unwrap_or_else(|| RecordCapture::new(self.component_count));
        capture.clear();
        self.open_spans.push(OpenSpan {
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
        let Some(mut span) = self.open_spans.pop() else {
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
            },
            context,
        );
        streaming_state(context)?.swap_capture(&mut span.capture);
        result?;
        increment_streaming_record_sequence("span completion", streaming_state(context)?)?;
        streaming_state(context)?.current_span = self.open_spans.last().map(|open| open.id.raw());
        streaming_state(context)?.pending_span_host = None;
        if self.capture_pool.len() < CAPTURE_POOL_LIMIT {
            span.capture.clear();
            self.capture_pool.push(span.capture);
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
        let Some(span) = self.open_spans.last_mut() else {
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
        handler: &WebUIHandler,
        protocol: &'data crate::Protocol,
        context: &mut WebUIProcessContext<'data, '_, '_>,
    ) -> Result<()> {
        let spanning = protocol_fragment(protocol.protocol(), &tag)?.contains_boundary;
        let enclosed = context.streaming.as_ref().is_some_and(|streaming| {
            streaming.active_boundary.is_some() || streaming.current_span.is_some()
        });
        if !spanning && !enclosed {
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
        prepare_generated_streaming_root(&tag, context)?;
        context.writer.write(">")?;
        self.push(Frame::GeneratedComponentEnd { tag, spanning })?;
        self.begin_component(
            &component,
            ComponentHostOrigin::HandlerGenerated,
            (handler, protocol),
            context,
        )
    }

    fn render_route(
        &mut self,
        route: &WebUiFragmentRoute,
        route_match: Option<RouteMatch>,
        protocol: &crate::Protocol,
        context: &mut WebUIProcessContext<'_, '_, '_>,
    ) -> Result<()> {
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
        let Some(route_match) = route_match else {
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

        let saved_route_base = std::mem::replace(
            &mut context.route_base,
            Cow::Owned(crate::route_matcher::compute_route_base(
                context.request_path,
                route_match.consumed_segments,
            )),
        )
        .into_owned()
        .into_boxed_str();
        let saved_route_children =
            std::mem::replace(&mut context.route_children, route.children.clone());
        self.push(Frame::RouteEnd {
            saved_route_base,
            saved_route_children,
        })?;
        if !route.fragment_id.is_empty() {
            self.push(Frame::GeneratedComponentStart {
                tag: route.fragment_id.clone().into(),
            })?;
        }
        if !route.content_fragment_id.is_empty() {
            let slot = fragment_slot(protocol, &route.content_fragment_id)?;
            self.push(Frame::EnterFragment(slot))?;
        }
        Ok(())
    }

    fn begin_outlet(&mut self, context: &mut WebUIProcessContext<'_, '_, '_>) -> Result<()> {
        let routes = std::mem::take(&mut context.route_children);
        if routes.is_empty() {
            return Ok(());
        }
        let request_segments = crate::route_matcher::split_request_path(context.request_path);
        let mut best: Option<(usize, RouteMatch)> = None;
        for (index, route) in routes.iter().enumerate() {
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
        self.push(Frame::Outlet(OutletFrame {
            routes,
            index: 0,
            best,
        }))
    }

    fn step_outlet(
        &mut self,
        mut frame: OutletFrame,
        handler: &WebUIHandler,
        protocol: &crate::Protocol,
        context: &mut WebUIProcessContext<'_, '_, '_>,
    ) -> Result<()> {
        if frame.index >= frame.routes.len() {
            return Ok(());
        }
        let index = frame.index;
        frame.index += 1;
        let route = std::mem::take(&mut frame.routes[index]);
        let route_match = frame
            .best
            .as_ref()
            .and_then(|(selected, route_match)| (*selected == index).then(|| route_match.clone()));
        self.push(Frame::Outlet(frame))?;
        let _ = (handler, protocol);
        self.render_route(&route, route_match, protocol, context)
    }

    fn push(&mut self, frame: Frame) -> Result<()> {
        if self.frames.len() >= MAX_CONTINUATION_DEPTH {
            return Err(continuation_limit_error(MAX_CONTINUATION_DEPTH));
        }
        self.frames.push(frame);
        Ok(())
    }

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
        let mut pending = vec![entry_id];
        let mut visited = HashSet::new();
        let mut requires_full_state =
            protocol.initial_state_strategy != InitialStateStrategy::Components as i32;
        while let Some(id) = pending.pop() {
            if !visited.insert(id) {
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
                    &mut keys,
                    &mut pending,
                    &mut requires_full_state,
                );
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

/// Park a freshly entered record, pre-selecting its best route match.
///
/// The record is already resolved by the caller's step-local cache, so entering
/// a child costs no additional lookup.
fn open_fragment(
    slot: u32,
    list: &webui_protocol::FragmentList,
    context: &WebUIProcessContext<'_, '_, '_>,
) -> FragmentFrame {
    let best_route = crate::route_renderer::find_best_route_match(
        &list.fragments,
        context.request_path,
        &context.route_base,
        context.route_index,
    );
    FragmentFrame {
        slot,
        // Render slots and continuation slots are the same numbering: both index
        // the protocol's sorted fragment IDs. Reading prepared metadata therefore
        // costs no ID lookup. `render_slots_match_continuation_slots` pins this.
        render_slot: slot as usize,
        index: 0,
        best_route,
    }
}

/// Borrow the record a continuation frame is walking.
fn slot_fragment(protocol: &crate::Protocol, slot: u32) -> Result<&webui_protocol::FragmentList> {
    let id = protocol
        .fragment_id(slot)
        .ok_or_else(|| unknown_fragment_slot_error(slot))?;
    protocol_fragment(protocol.protocol(), id)
}

#[cold]
#[inline(never)]
fn unknown_fragment_slot_error(slot: u32) -> HandlerError {
    HandlerError::Invariant(format!("continuation frame references unknown slot {slot}"))
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

#[cfg(test)]
mod tests {
    use super::*;
    use webui_protocol::{ComponentData, FragmentList};

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

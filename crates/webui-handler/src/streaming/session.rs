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
use super::vm::ContinuationVm;
use super::StreamingSink;
use crate::plugin::HandlerPlugin;
use crate::route_handler::Protocol;
use crate::{
    FlushWriter, HandlerError, RenderOptions, ResponseWriter, Result, WebUIHandler,
    WebUIProcessContext,
};

/// Maximum continuation frames retained by one response.
pub const MAX_CONTINUATION_DEPTH: usize = 256;
/// Maximum unfinished component spans retained by one response.
pub const MAX_OPEN_SPANS: usize = 128;
/// Maximum nested unfinished component spans.
pub const MAX_SPAN_NESTING: usize = 32;
/// Maximum runtime boundary occurrences in one response.
pub const MAX_BOUNDARY_OCCURRENCES: usize = 512;
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
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StreamStatus {
    /// The next occurrence waiting for [`StreamingResponse::resume`].
    pub boundary: Option<BoundaryDescriptor>,
    /// True after the terminal record and writer end completed.
    pub done: bool,
}

/// A progressive response that writes directly through a [`FlushWriter`].
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
    pub fn start(&mut self, state: &Value) -> Result<StreamStatus> {
        self.core.start(
            SessionCall {
                handler: self.handler,
                protocol: self.protocol,
                options: &self.options,
                writer: &mut self.sink,
            },
            state,
        )
    }

    /// Commit the pending occurrence, then advance to the next occurrence or terminal.
    pub fn resume(
        &mut self,
        instance_id: BoundaryInstanceId,
        state: &Value,
        mode: BoundaryMode,
    ) -> Result<StreamStatus> {
        self.core.resume(
            SessionCall {
                handler: self.handler,
                protocol: self.protocol,
                options: &self.options,
                writer: &mut self.sink,
            },
            instance_id,
            state,
            mode,
        )
    }

    /// Emit one projected state update for a committed updatable occurrence.
    pub fn update(&mut self, instance_id: BoundaryInstanceId, patch: &Value) -> Result<()> {
        self.core.update(
            SessionCall {
                handler: self.handler,
                protocol: self.protocol,
                options: &self.options,
                writer: &mut self.sink,
            },
            instance_id,
            patch,
        )
    }

    /// Whether the response has emitted its terminal and ended its writer.
    #[must_use]
    pub fn is_done(&self) -> bool {
        self.core.done
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
        let prepared = protocol.continuation_state_plan(entry_id);
        let state_plan = prepared.resolve()?;
        Ok(Self {
            vm: ContinuationVm::new(entry_id, protocol)?,
            frozen_keys: Arc::clone(&state_plan.keys),
            requires_full_state: state_plan.requires_full_state,
            frozen_state: Value::Object(serde_json::Map::new()),
            started: false,
            done: false,
            failed: false,
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
        self.run_advance(call, None)
    }

    pub(crate) fn resume(
        &mut self,
        call: SessionCall<'_, '_>,
        instance_id: BoundaryInstanceId,
        state: &Value,
        mode: BoundaryMode,
    ) -> Result<StreamStatus> {
        self.require_usable("resume")?;
        self.require_started("resume")?;
        self.vm.validate_resume(instance_id)?;
        if self.requires_full_state {
            overlay_full_state(&mut self.frozen_state, state);
        } else {
            overlay_selected_state(&mut self.frozen_state, state, &self.frozen_keys);
        }
        self.run_advance(call, Some((instance_id, mode)))
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

    fn run_advance(
        &mut self,
        call: SessionCall<'_, '_>,
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
            let status = vm.advance(handler, protocol, context)?;
            if !status.done {
                context.writer.stream_flush()?;
            }
            Ok(status)
        });
        self.frozen_state = state;
        match result {
            Ok(status) => {
                self.done = status.done;
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
}

#[cold]
#[inline(never)]
fn missing_progress_error() -> HandlerError {
    HandlerError::Invariant("streaming progress is unavailable".to_string())
}

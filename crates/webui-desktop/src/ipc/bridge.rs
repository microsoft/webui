// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

use super::{
    credentials::constant_equal,
    decode_frame,
    engine::{Core, Counters, Incoming, State},
    error::fail,
    executor::{lock, Budget, Permit, TimerKey},
    session::SubscriptionSlot,
    wire::{ipc_frame::Body, IpcFrame, Kind},
    Cancellation, Endpoint, IpcError, IpcErrorCode, IpcFuture, IpcLimits, IpcOptions, IpcRegistry,
    IpcSession, IpcWindow, MethodKind, NotificationContext, RequestContext,
};
use crate::protocol::{DesktopHttpMethod, DesktopProtocolResponse, DesktopResponseBody};
use futures_channel::oneshot;
use futures_util::{
    future::{AbortHandle, Abortable},
    FutureExt,
};
use prost::Message;
use std::{
    sync::{atomic::Ordering, Arc, Mutex, OnceLock, Weak},
    time::{Duration, Instant},
};

/// Host provenance is independent of development permissions.
#[derive(Clone, Debug)]
pub enum IpcHost {
    /// A source-backed application.
    Source {
        /// Canonical fixed application origin, supplied by the adapter.
        origin: String,
    },
    /// A packaged application; development-only methods are always denied.
    Packaged {
        /// Canonical fixed application origin, supplied by the adapter.
        origin: String,
    },
}
impl IpcHost {
    pub(super) fn is_source(&self) -> bool {
        matches!(self, Self::Source { .. })
    }
    pub(super) fn origin(&self) -> &str {
        match self {
            Self::Source { origin } | Self::Packaged { origin } => origin,
        }
    }
}

/// Sole non-cloneable owner of a frame's mutable IPC state.
pub struct IpcWindowOwner {
    core: Arc<Core>,
}
impl IpcWindowOwner {
    /// Validate immutable policy without starting workers or timer threads.
    pub fn new(
        registry: Arc<IpcRegistry>,
        options: IpcOptions,
        host: IpcHost,
    ) -> Result<Self, IpcError> {
        registry.validate()?;
        options.validate()?;
        if registry.notifications.len() > options.limits.max_callbacks_per_document {
            return Err(IpcError::new(IpcErrorCode::InvalidPayload, "startup notification definitions exceed the document callback limit", "raise max_callbacks_per_document within its SDK ceiling or register fewer startup receivers"));
        }
        if !matches!(host.origin(), "webui://app" | "https://app.webui.localhost") {
            return Err(fail(IpcErrorCode::PermissionDenied));
        }
        let budget = Budget::new(
            options
                .limits
                .max_worker_tasks_per_frame_including_retired_documents,
            options.limits.max_retained_bytes_per_frame,
        );
        let input_budget = Budget::new(0, options.limits.max_admitted_input_bytes_per_frame);
        let control_budget = Budget::new(
            0,
            options.limits.reserved_control_frames_per_direction
                * (options.limits.max_error_text_bytes_total + 128)
                * 3,
        );
        Ok(Self {
            core: Arc::new(Core {
                registry,
                options,
                host,
                budget,
                input_budget,
                control_budget,
                state: Mutex::new(State {
                    closed: false,
                    navigation: 0,
                    admitted: false,
                    generation: 0,
                    document: None,
                    waiters: Vec::new(),
                    waker: None,
                    wake_pending: false,
                    closed_control: None,
                    activation: None,
                    activation_started: false,
                }),
                workers: OnceLock::new(),
                timer: OnceLock::new(),
                counters: Counters::default(),
            }),
        })
    }
    /// Weak application handle.
    pub fn window(&self) -> IpcWindow {
        IpcWindow {
            core: Arc::downgrade(&self.core),
        }
    }
    /// Weak native adapter handle.
    pub fn bridge(&self) -> IpcBridge {
        IpcBridge {
            core: Arc::downgrade(&self.core),
        }
    }
    /// Close without waiting for uncooperative application code.
    pub fn close(&self) {
        self.core.retire(IpcErrorCode::Closed, true);
    }
}
impl Drop for IpcWindowOwner {
    fn drop(&mut self) {
        self.close();
    }
}

/// Main-document capability principal, constructed only from trusted native
/// top-level commit tracking and canonical origin, never a message callback.
#[derive(Clone, Debug)]
pub struct CommittedMainDocument {
    /// Strictly increasing top-level navigation identity.
    pub navigation: u64,
    /// Platform-canonicalized fixed app origin.
    pub origin: String,
}
/// Generated renderer schema handshake.
#[derive(Clone, Debug, serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Hello {
    /// Must be exactly two.
    pub wire_version: u32,
    /// Exact generated contract name.
    pub contract_name: String,
    /// Exact breaking schema version.
    pub contract_major: u32,
    /// Exact normalized lowercase SHA-256 digest.
    pub schema_hash: String,
}
/// Bounded adapter admission metadata.
pub struct Admission {
    /// Renderer schema identity.
    pub hello: Hello,
    /// Echoed activation, compared atomically with native-owned proof.
    pub proof: super::DocumentActivation,
}
/// Successful document admission. Tokens must not appear in URLs or logs.
#[derive(Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionInfo {
    /// Serialize as a canonical decimal string at the native JSON boundary.
    #[serde(serialize_with = "super::admission::decimal")]
    pub generation: u64,
    /// Cryptographic 128-bit lowercase hexadecimal credential.
    pub token: String,
    /// Validated numeric budgets.
    pub limits: IpcLimits,
}
/// Payload-free native event-loop wake source.
pub trait IpcWake: Send + Sync + 'static {
    /// Schedule a native control drain; never poll application code here.
    fn wake(&self) -> Result<(), IpcError>;
}
/// Bounded native push metadata.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum NativeControl {
    /// Outbound frames are available.
    Ready {
        /// Canonical decimal string on native JSON.
        generation: u64,
    },
    /// A document connection was revoked.
    Closed {
        /// Revoked generation.
        generation: u64,
        /// Stable terminal category.
        code: IpcErrorCode,
    },
}
/// Owned native protocol request. Adapters must cap growth before constructing it.
pub struct OwnedIpcHttpRequest {
    /// Trusted native navigation identity.
    pub navigation: u64,
    /// Only POST ingress and GET drain are supported.
    pub method: DesktopHttpMethod,
    /// Reserved IPC route without query strings.
    pub path: String,
    /// Session header, never a query parameter.
    pub token: String,
    /// Complete bounded protobuf frame.
    pub body: Vec<u8>,
    /// Reservation obtained before body allocation/copy; transferred into work.
    pub input_permit: IpcInputPermit,
}
/// Non-cloneable reservation for native-owned body capacity. The SDK transfers
/// it into admitted work; a 204 does not release a blocked handler's credit.
pub struct IpcInputPermit {
    core: Weak<Core>,
    input: Permit,
    retained: Permit,
    emergency: bool,
    max: usize,
}
impl IpcInputPermit {
    /// Reserve additional credit before growing the body buffer.
    pub fn try_grow(&mut self, additional_bytes: usize) -> Result<(), IpcError> {
        let core = self
            .core
            .upgrade()
            .ok_or_else(|| fail(IpcErrorCode::Closed))?;
        if lock(&core.state).closed {
            return Err(fail(IpcErrorCode::Closed));
        }
        if additional_bytes > self.max.saturating_sub(self.input.bytes()) {
            return Err(fail(IpcErrorCode::PayloadTooLarge));
        }
        let result = self.grow_existing(additional_bytes);
        if result.is_err() && !self.emergency {
            let size = self.input.bytes() + additional_bytes;
            let control_max = core.options.limits.max_error_text_bytes_total + 128;
            if size <= control_max {
                let input = core.control_budget.reserve(0, size)?;
                let retained = core.control_budget.reserve(0, size)?;
                self.input = input;
                self.retained = retained;
                self.emergency = true;
                self.max = control_max;
                return Ok(());
            }
        }
        result
    }
    fn grow_existing(&mut self, additional_bytes: usize) -> Result<(), IpcError> {
        self.input.grow(additional_bytes)?;
        if let Err(error) = self.retained.grow(additional_bytes) {
            self.input.shrink(additional_bytes);
            return Err(error);
        }
        Ok(())
    }
    fn require_data(&mut self, core: &Arc<Core>) -> Result<(), IpcError> {
        if self.emergency {
            let input = core.input_budget.reserve(0, self.input.bytes())?;
            let retained = core.budget.reserve(0, self.input.bytes())?;
            self.input = input;
            self.retained = retained;
            self.emergency = false;
        }
        Ok(())
    }
}
pub(super) struct InputCredits {
    pub _input: IpcInputPermit,
    pub _copy: Permit,
}
/// Per-frame weak native facade; its futures contain response completions only.
#[derive(Clone)]
pub struct IpcBridge {
    pub(super) core: Weak<Core>,
}
impl IpcBridge {
    /// Whether this live frame has an explicitly configured application schema.
    /// Disabled/default frames need no native bootstrap or wake-source setup.
    pub fn is_enabled(&self) -> bool {
        self.core
            .upgrade()
            .is_some_and(|core| core.registry.is_enabled() && !lock(&core.state).closed)
    }

    /// Install the single coalesced native wake source.
    pub fn attach_waker(&self, wake: Arc<dyn IpcWake>) -> Result<(), IpcError> {
        let core = self
            .core
            .upgrade()
            .ok_or_else(|| fail(IpcErrorCode::Closed))?;
        {
            let mut state = lock(&core.state);
            if state.closed {
                return Err(fail(IpcErrorCode::Closed));
            }
            if state.waker.is_some() {
                return Err(fail(IpcErrorCode::InvalidFrame));
            }
            state.waker = Some(wake);
        }
        core.signal();
        Ok(())
    }
    /// Revoke the old document before replacement. Same-document routing must
    /// not call this method. A backwards/repeated identity is ignored.
    pub fn navigate(&self, navigation: u64) {
        let Some(core) = self.core.upgrade() else {
            return;
        };
        core.navigate(navigation);
    }
    /// Authenticate and close only this document connection. A generation alone
    /// grants no authority; stale credentials cannot retire a replacement.
    /// Trusted native teardown must use `close` or `navigate` instead.
    pub fn disconnect_authenticated(&self, generation: u64, token: &str) -> Result<(), IpcError> {
        let core = self
            .core
            .upgrade()
            .ok_or_else(|| fail(IpcErrorCode::Closed))?;
        core.disconnect_authenticated(generation, token)
    }
    /// Terminal shutdown without joining application workers.
    pub fn close(&self) {
        if let Some(core) = self.core.upgrade() {
            core.retire(IpcErrorCode::Closed, true);
        }
    }
    /// Charge complete input capacity before adapters allocate or copy bytes.
    /// Small control ingress has separately bounded emergency storage, so a
    /// saturated application-byte budget cannot prevent cancellation/completion.
    pub fn reserve_input(&self, bytes: usize) -> Result<IpcInputPermit, IpcError> {
        let core = self
            .core
            .upgrade()
            .ok_or_else(|| fail(IpcErrorCode::Closed))?;
        if lock(&core.state).closed {
            return Err(fail(IpcErrorCode::Closed));
        }
        if bytes > core.options.limits.max_frame_bytes {
            return Err(fail(IpcErrorCode::PayloadTooLarge));
        }
        let normal = (|| {
            Ok((
                core.input_budget.reserve(0, bytes)?,
                core.budget.reserve(0, bytes)?,
            ))
        })();
        let (input, retained, emergency, max) = match normal {
            Ok((input, retained)) => (input, retained, false, core.options.limits.max_frame_bytes),
            Err(_) if bytes <= core.options.limits.max_error_text_bytes_total + 128 => (
                core.control_budget.reserve(0, bytes)?,
                core.control_budget.reserve(0, bytes)?,
                true,
                core.options.limits.max_error_text_bytes_total + 128,
            ),
            Err(error) => return Err(error),
        };
        Ok(IpcInputPermit {
            core: self.core.clone(),
            input,
            retained,
            emergency,
            max,
        })
    }
    /// Drain coalesced metadata on the native thread. No application bytes.
    pub fn take_control(&self) -> Result<Option<NativeControl>, IpcError> {
        let core = self
            .core
            .upgrade()
            .ok_or_else(|| fail(IpcErrorCode::Closed))?;
        let mut state = lock(&core.state);
        if state.closed {
            return Ok(None);
        }
        state.wake_pending = false;
        if let Some(control) = state.closed_control.take() {
            return Ok(Some(control));
        }
        if let Some(doc) = &mut state.document {
            if doc.ready {
                doc.ready = false;
                return Ok(Some(NativeControl::Ready {
                    generation: doc.generation,
                }));
            }
        }
        Ok(None)
    }
    /// Validate bounded envelope/control metadata and reserve capacity immediately.
    /// Returned futures never decode application payloads or run user handlers.
    /// Adapters add `Cache-Control: no-store` to every response.
    pub fn submit(&self, request: OwnedIpcHttpRequest) -> IpcFuture<DesktopProtocolResponse> {
        let navigation = request.navigation;
        let weak = self.core.clone();
        let result = self
            .core
            .upgrade()
            .ok_or_else(|| fail(IpcErrorCode::Closed))
            .and_then(|core| {
                let result = submit(&core, request);
                if result.is_err() {
                    core.counters.rejected.fetch_add(1, Ordering::Relaxed);
                }
                result
            });
        Box::pin(async move {
            Ok(match result {
                Ok((response, _drain)) => {
                    let core = weak.upgrade().ok_or_else(|| fail(IpcErrorCode::Closed))?;
                    let state = lock(&core.state);
                    if state.closed {
                        return Err(fail(IpcErrorCode::Closed));
                    }
                    if state.navigation != navigation {
                        return Err(fail(IpcErrorCode::Navigated));
                    }
                    if state.document.is_none() {
                        return Err(fail(IpcErrorCode::Closed));
                    }
                    response
                }
                Err(error) => http_error(error),
            })
        })
    }
}

struct DrainGuard {
    core: Weak<Core>,
    generation: u64,
}
impl Drop for DrainGuard {
    fn drop(&mut self) {
        if let Some(core) = self.core.upgrade() {
            let mut state = lock(&core.state);
            if let Ok(doc) = Core::document(&mut state, self.generation) {
                doc.drain_inflight = false;
            }
        }
    }
}

fn submit(
    core: &Arc<Core>,
    mut request: OwnedIpcHttpRequest,
) -> Result<(DesktopProtocolResponse, Option<DrainGuard>), IpcError> {
    if request.body.len() > core.options.limits.max_frame_bytes {
        return Err(fail(IpcErrorCode::PayloadTooLarge));
    }
    if !Weak::ptr_eq(&request.input_permit.core, &Arc::downgrade(core))
        || request.input_permit.input.bytes() < request.body.capacity()
    {
        return Err(fail(IpcErrorCode::InvalidFrame));
    }
    if request.path.len() > 64 || request.token.len() != 32 {
        return Err(fail(IpcErrorCode::InvalidFrame));
    }
    let generation = {
        let state = lock(&core.state);
        if state.closed {
            return Err(fail(IpcErrorCode::Closed));
        }
        let doc = state
            .document
            .as_ref()
            .ok_or_else(|| fail(IpcErrorCode::NotReady))?;
        if doc.navigation != request.navigation
            || !constant_equal(doc.token.as_bytes(), request.token.as_bytes())
        {
            return Err(fail(IpcErrorCode::PermissionDenied));
        }
        doc.generation
    };
    match (&request.method, request.path.as_str()) {
        (DesktopHttpMethod::Get, "/_webui/ipc/outbound") if request.body.is_empty() => {
            let mut state = lock(&core.state);
            let doc = Core::document(&mut state, generation)?;
            if doc.drain_inflight {
                return Err(fail(IpcErrorCode::Overloaded));
            }
            doc.drain_inflight = true;
            let drain = DrainGuard {
                core: Arc::downgrade(core),
                generation,
            };
            let Some(queued) = doc.queue.pop_front() else {
                return Ok((empty(), Some(drain)));
            };
            if queued.control {
                doc.control_slots -= 1;
                if queued.bytes.len() > core.options.limits.max_error_text_bytes_total + 128 {
                    doc.data_bytes -= queued.bytes.len();
                }
            } else {
                doc.data_frames -= 1;
                doc.data_bytes -= queued.bytes.len();
            }
            Ok((
                DesktopProtocolResponse::new(
                    200,
                    "application/x-protobuf",
                    DesktopResponseBody::with_guard(queued.bytes, queued._memory),
                ),
                Some(drain),
            ))
        }
        (DesktopHttpMethod::Post, "/_webui/ipc") => {
            let received = Instant::now();
            let (mut copy, emergency_copy) = match core.budget.reserve(0, request.body.len()) {
                Ok(copy) => (copy, false),
                Err(_)
                    if request.body.len()
                        <= core.options.limits.max_error_text_bytes_total + 128 =>
                {
                    (core.control_budget.reserve(0, request.body.len())?, true)
                }
                Err(error) => return Err(error),
            };
            let frame = decode_frame(&request.body, &core.options.limits)?;
            if frame.generation != generation {
                return Err(fail(IpcErrorCode::Navigated));
            }
            if matches!(Kind::try_from(frame.kind), Ok(Kind::Request | Kind::Notify)) {
                request.input_permit.require_data(core)?;
                if emergency_copy {
                    copy = core.budget.reserve(0, request.body.len())?;
                }
            }
            let credits = InputCredits {
                _input: request.input_permit,
                _copy: copy,
            };
            dispatch(core, frame, received, credits)?;
            Ok((empty(), None))
        }
        _ => Err(fail(IpcErrorCode::InvalidFrame)),
    }
}
fn empty() -> DesktopProtocolResponse {
    DesktopProtocolResponse::new(204, "application/x-protobuf", Vec::new())
}
fn http_error(error: IpcError) -> DesktopProtocolResponse {
    let status = match error.code {
        IpcErrorCode::PermissionDenied => 401,
        IpcErrorCode::Navigated | IpcErrorCode::NotReady | IpcErrorCode::SchemaMismatch => 409,
        IpcErrorCode::PayloadTooLarge => 413,
        IpcErrorCode::Overloaded => 429,
        IpcErrorCode::Closed | IpcErrorCode::Transport => 503,
        _ => 400,
    };
    DesktopProtocolResponse::new(
        status,
        "application/x-protobuf",
        error.wire(2048).encode_to_vec(),
    )
}
fn dispatch(
    core: &Arc<Core>,
    frame: IpcFrame,
    received: Instant,
    credits: InputCredits,
) -> Result<(), IpcError> {
    match Kind::try_from(frame.kind).map_err(|_| fail(IpcErrorCode::InvalidFrame))? {
        Kind::Request | Kind::Notify => invoke(core, frame, received, credits),
        Kind::Result | Kind::Error | Kind::Accept => reply(core, frame, credits),
        Kind::Cancel => {
            let incoming = {
                let mut state = lock(&core.state);
                let doc = Core::document(&mut state, frame.generation)?;
                let incoming = doc.incoming.remove(&frame.id);
                if incoming.is_some() {
                    doc.control_slots -= 1;
                }
                incoming
            };
            if let Some(incoming) = incoming {
                core.remove_timer(
                    incoming.deadline,
                    TimerKey {
                        generation: frame.generation,
                        id: frame.id,
                        local: false,
                    },
                );
                incoming.cancellation.cancel();
                incoming.abort.abort();
            } else {
                core.counters.stale_replies.fetch_add(1, Ordering::Relaxed);
            }
            Ok(())
        }
        Kind::Unspecified => Err(fail(IpcErrorCode::InvalidFrame)),
    }
}
fn reply(core: &Core, frame: IpcFrame, credits: InputCredits) -> Result<(), IpcError> {
    let (sender, result) = {
        let mut state = lock(&core.state);
        let doc = Core::document(&mut state, frame.generation)?;
        let Some(pending) = doc.pending.get_mut(&frame.id) else {
            core.counters.stale_replies.fetch_add(1, Ordering::Relaxed);
            return Ok(());
        };
        if !pending.sent
            || (frame.kind == Kind::Accept as i32 && pending.kind != MethodKind::Notification)
            || (frame.kind == Kind::Result as i32 && pending.kind != MethodKind::Rpc)
        {
            return Err(fail(IpcErrorCode::InvalidFrame));
        }
        let result = match frame.body {
            Some(Body::Payload(bytes)) => Ok(super::engine::ReceivedPayload {
                bytes,
                _credits: credits,
            }),
            Some(Body::Error(error)) => Err(IpcError::from_wire(
                error,
                core.options.limits.max_error_text_bytes_total,
            )),
            None => Ok(super::engine::ReceivedPayload {
                bytes: Vec::new(),
                _credits: credits,
            }),
        };
        (pending.sender.take(), result)
    };
    if let Some(sender) = sender {
        let _ = sender.send(result);
    } else {
        core.counters.stale_replies.fetch_add(1, Ordering::Relaxed);
    }
    Ok(())
}

fn invoke(
    core: &Arc<Core>,
    frame: IpcFrame,
    received: Instant,
    credits: InputCredits,
) -> Result<(), IpcError> {
    let kind = if frame.kind == Kind::Request as i32 {
        MethodKind::Rpc
    } else {
        MethodKind::Notification
    };
    let method = core.allowed(frame.method_id, Endpoint::Host, kind);
    {
        let mut state = lock(&core.state);
        let doc = Core::document(&mut state, frame.generation)?;
        if frame.id <= doc.last_peer_id {
            return Err(fail(IpcErrorCode::InvalidFrame));
        }
        doc.last_peer_id = frame.id;
        if let Err(error) = method {
            let reserve = core.reservation(doc)?;
            core.enqueue_control(
                doc,
                frame.id,
                (
                    Kind::Error,
                    Some(Body::Error(
                        error.wire(core.options.limits.max_error_text_bytes_total),
                    )),
                ),
                reserve,
            );
            drop(state);
            core.signal();
            return Ok(());
        }
    }
    if kind == MethodKind::Notification {
        return notify(core, frame, credits);
    }
    request(core, frame, received, credits)
}
fn request(
    core: &Arc<Core>,
    frame: IpcFrame,
    received: Instant,
    credits: InputCredits,
) -> Result<(), IpcError> {
    let permit = core
        .budget
        .reserve(1, core.options.limits.max_frame_bytes)?;
    let workers = core.workers()?;
    let timer = core.timer()?;
    let deadline = received
        .checked_add(Duration::from_millis(u64::from(frame.timeout_ms)))
        .ok_or_else(|| fail(IpcErrorCode::InvalidFrame))?;
    let (abort, registration) = AbortHandle::new_pair();
    let cancellation = Cancellation::default();
    {
        let mut state = lock(&core.state);
        let doc = Core::document(&mut state, frame.generation)?;
        if doc.incoming.len() >= core.options.limits.max_pending_calls_per_direction {
            return Err(fail(IpcErrorCode::Overloaded));
        }
        let reserve = core.reservation(doc)?;
        doc.incoming.insert(
            frame.id,
            Incoming {
                cancellation: cancellation.clone(),
                abort,
                deadline,
                reserve,
            },
        );
        timer.insert(
            deadline,
            TimerKey {
                generation: frame.generation,
                id: frame.id,
                local: false,
            },
        );
    }
    let weak = Arc::downgrade(core);
    let handler = core.registry.handlers.get(&frame.method_id).cloned();
    let limits = core.options.limits.clone();
    let generation = frame.generation;
    let id = frame.id;
    let task = async move {
        let _permit = permit;
        let _credits = credits;
        let result = Abortable::new(
            async {
                if cancellation.is_cancelled() || Instant::now() >= deadline {
                    return Err(fail(IpcErrorCode::DeadlineExceeded));
                }
                let Some(handler) = handler else {
                    return Err(fail(IpcErrorCode::ReceiverUnavailable));
                };
                let Some(Body::Payload(bytes)) = frame.body else {
                    return Err(fail(IpcErrorCode::InvalidFrame));
                };
                let context = RequestContext {
                    session: IpcSession {
                        core: weak.clone(),
                        generation,
                    },
                    cancellation,
                    deadline,
                };
                handler(context, bytes, limits).await
            },
            registration,
        )
        .await;
        if let (Ok(result), Some(core)) = (result, weak.upgrade()) {
            core.complete_incoming(generation, id, result);
        }
    };
    if let Err(error) = workers.spawn(task) {
        core.complete_incoming(generation, id, Err(error));
    }
    core.counters.accepted.fetch_add(1, Ordering::Relaxed);
    Ok(())
}

struct Delivery {
    slot: Arc<SubscriptionSlot>,
    previous: Option<oneshot::Receiver<()>>,
    next: oneshot::Sender<()>,
}
fn notify(core: &Arc<Core>, frame: IpcFrame, credits: InputCredits) -> Result<(), IpcError> {
    let workers = core.workers()?;
    let method = core.allowed(frame.method_id, Endpoint::Host, MethodKind::Notification)?;
    let (deliveries, reserve, permit, callback_permit, notification_permit) = {
        let mut state = lock(&core.state);
        let doc = Core::document(&mut state, frame.generation)?;
        let count = doc
            .subscriptions
            .values()
            .filter(|slot| slot.method == frame.method_id && slot.active.load(Ordering::Acquire))
            .count();
        let permit = core.budget.reserve(
            count + 1,
            frame
                .encoded_len()
                .checked_mul(count)
                .ok_or_else(|| fail(IpcErrorCode::Overloaded))?,
        )?;
        let callback_permit = doc.callback_budget.reserve(count, 0)?;
        let notification_permit = doc.notification_budget.reserve(1, 0)?;
        let reserve = core.reservation(doc)?;
        let mut deliveries = Vec::with_capacity(count);
        for slot in doc
            .subscriptions
            .values()
            .filter(|slot| slot.method == frame.method_id && slot.active.load(Ordering::Acquire))
        {
            let (next, tail) = oneshot::channel();
            let previous = lock(&slot.tail).replace(tail);
            deliveries.push(Delivery {
                slot: Arc::clone(slot),
                previous,
                next,
            });
        }
        (
            deliveries,
            reserve,
            permit,
            callback_permit,
            notification_permit,
        )
    };
    let weak = Arc::downgrade(core);
    let limits = core.options.limits.clone();
    let generation = frame.generation;
    let id = frame.id;
    let task = async move {
        // The shared fanout permit survives until every callback actually exits.
        let permit = Arc::new((permit, callback_permit, notification_permit, credits));
        let Some(Body::Payload(bytes)) = frame.body else {
            return;
        };
        let validation = (method.validate_request)(&bytes, &limits);
        let invoke_callbacks = validation.is_ok();
        let Some(core) = weak.upgrade() else {
            return;
        };
        {
            let mut state = lock(&core.state);
            let Ok(doc) = Core::document(&mut state, generation) else {
                return;
            };
            match validation {
                Ok(()) => core.enqueue_control(doc, id, (Kind::Accept, None), reserve),
                Err(error) => {
                    core.enqueue_control(
                        doc,
                        id,
                        (
                            Kind::Error,
                            Some(Body::Error(error.wire(limits.max_error_text_bytes_total))),
                        ),
                        reserve,
                    );
                }
            }
        }
        core.signal();
        let bytes = Arc::new(bytes);
        for delivery in deliveries {
            let weak = weak.clone();
            let bytes = Arc::clone(&bytes);
            let limits = limits.clone();
            let permit = Arc::clone(&permit);
            let work = async move {
                let _permit = permit;
                let _next = delivery.next;
                // Even a rejected notification is an ordering node. Waiting
                // outside callback cancellation prevents a skipped middle node
                // from releasing successors past an unfinished predecessor.
                if let Some(previous) = delivery.previous {
                    let _ = previous.await;
                }
                if !invoke_callbacks {
                    return;
                }
                let slot = delivery.slot;
                let cancellation = slot.cancellation.cancelled();
                let callback = async {
                    if !slot.active.load(Ordering::Acquire) {
                        return;
                    }
                    let callback = lock(&slot.callback).as_ref().map(Arc::clone);
                    if let Some(callback) = callback {
                        if !slot.active.load(Ordering::Acquire) {
                            return;
                        }
                        let context = NotificationContext {
                            session: IpcSession {
                                core: weak.clone(),
                                generation,
                            },
                        };
                        if callback(context, bytes, limits).await.is_err() {
                            if let Some(core) = weak.upgrade() {
                                core.counters
                                    .callback_errors
                                    .fetch_add(1, Ordering::Relaxed);
                            }
                        }
                    }
                }
                .boxed();
                let _ = futures_util::future::select(callback, cancellation).await;
            };
            if let Err(error) = core.workers().and_then(|pool| pool.spawn(work)) {
                core.retire(error.code, true);
                return;
            }
        }
    };
    workers.spawn(task)?;
    core.counters.accepted.fetch_add(1, Ordering::Relaxed);
    Ok(())
}

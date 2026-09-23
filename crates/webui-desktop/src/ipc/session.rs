// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

pub use super::executor::Cancellation;
use super::{
    engine::{envelope, Core, Pending},
    error::fail,
    executor::{lock, TimerKey},
    wire::{ipc_frame::Body, Kind},
    Endpoint, Event, Host, IpcCodec, IpcError, IpcErrorCode, IpcFuture, IpcLimits, MethodKind,
    Renderer, Rpc,
};
use futures_channel::oneshot;
use futures_util::future::{AbortHandle, Abortable};
use std::{
    future::Future,
    pin::Pin,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex, Weak,
    },
    task::{Context, Poll},
    time::{Duration, Instant},
};

/// Local monotonic deadline policy.
#[derive(Clone, Copy, Debug)]
pub struct CallOptions {
    /// Includes worker and transport queue time; zero is invalid.
    pub timeout: Duration,
}
impl Default for CallOptions {
    fn default() -> Self {
        Self {
            timeout: Duration::from_secs(30),
        }
    }
}
/// Context for a Rust RPC receiver.
#[derive(Clone)]
pub struct RequestContext {
    /// The calling document, never silently retargeted.
    pub session: IpcSession,
    /// Cooperative cancellation signal.
    pub cancellation: Cancellation,
    /// Receiver-local monotonic deadline.
    pub deadline: Instant,
}
/// Context for a notification subscriber.
#[derive(Clone)]
pub struct NotificationContext {
    /// Document that sent the notification.
    pub session: IpcSession,
}

/// Weak handle to an owning desktop frame.
#[derive(Clone)]
pub struct IpcWindow {
    pub(super) core: Weak<Core>,
}
impl IpcWindow {
    /// Return the currently admitted document, without waiting.
    pub fn current_session(&self) -> Result<IpcSession, IpcError> {
        let core = self
            .core
            .upgrade()
            .ok_or_else(|| fail(IpcErrorCode::Closed))?;
        let state = lock(&core.state);
        if state.closed {
            return Err(fail(IpcErrorCode::Closed));
        }
        let doc = state
            .document
            .as_ref()
            .ok_or_else(|| fail(IpcErrorCode::NotReady))?;
        Ok(IpcSession {
            core: self.core.clone(),
            generation: doc.generation,
        })
    }
    /// Await the current or next admission. Cancelled waiters release their slot.
    pub fn ready(&self) -> IpcFuture<IpcSession> {
        let result = (|| {
            let core = self
                .core
                .upgrade()
                .ok_or_else(|| fail(IpcErrorCode::Closed))?;
            let mut state = lock(&core.state);
            if state.closed {
                return Err(fail(IpcErrorCode::Closed));
            }
            if !core.registry.is_enabled() {
                return Err(fail(IpcErrorCode::NotReady));
            }
            if let Some(doc) = &state.document {
                return Ok(Ready::Now(IpcSession {
                    core: self.core.clone(),
                    generation: doc.generation,
                }));
            }
            state.waiters.retain(|waiter| !waiter.is_canceled());
            if state.waiters.len() >= core.options.limits.max_readiness_waiters {
                return Err(fail(IpcErrorCode::Overloaded));
            }
            let (sender, receiver) = oneshot::channel();
            state.waiters.push(sender);
            Ok(Ready::Wait(receiver))
        })();
        Box::pin(async move {
            match result? {
                Ready::Now(session) => Ok(session),
                Ready::Wait(receiver) => receiver.await.map_err(|_| fail(IpcErrorCode::Closed))?,
            }
        })
    }
    /// Current bounded resource usage and cumulative transport counters.
    pub fn stats(&self) -> IpcStats {
        let Some(core) = self.core.upgrade() else {
            return IpcStats::default();
        };
        let state = lock(&core.state);
        let doc = state.document.as_ref();
        IpcStats {
            accepted: core.counters.accepted.load(Ordering::Relaxed),
            rejected: core.counters.rejected.load(Ordering::Relaxed),
            pending: doc.map_or(0, |d| d.pending.len() + d.incoming.len()),
            queued_bytes: doc.map_or(0, |d| d.queue.iter().map(|q| q.bytes.len()).sum()),
            worker_tasks: core.budget.tasks.load(Ordering::Acquire),
            retained_bytes: core.budget.bytes.load(Ordering::Acquire),
            admitted_input_bytes: core.input_budget.bytes.load(Ordering::Acquire),
            timeouts: core.counters.timeouts.load(Ordering::Relaxed),
            cancellations: core.counters.cancellations.load(Ordering::Relaxed),
            stale_replies: core.counters.stale_replies.load(Ordering::Relaxed),
            callback_errors: core.counters.callback_errors.load(Ordering::Relaxed),
            wakeups: core.counters.wakeups.load(Ordering::Relaxed),
            deadline_wakeups: core
                .timer
                .get()
                .and_then(|timer| timer.as_ref().ok())
                .map_or(0, |timer| timer.wakes.load(Ordering::Relaxed)),
            workers_started: core.workers.get().is_some_and(Result::is_ok),
            timer_started: core.timer.get().is_some_and(Result::is_ok),
        }
    }
}
enum Ready {
    Now(IpcSession),
    Wait(oneshot::Receiver<Result<IpcSession, IpcError>>),
}
/// Payload-free observability snapshot.
#[derive(Clone, Debug, Default)]
pub struct IpcStats {
    /// Accepted ingress invocations.
    pub accepted: u64,
    /// Rejected ingress submissions.
    pub rejected: u64,
    /// Live local and remote calls.
    pub pending: usize,
    /// Bytes awaiting transport drain.
    pub queued_bytes: usize,
    /// Active and retired worker permits.
    pub worker_tasks: usize,
    /// Active and retired accounted transport bytes.
    pub retained_bytes: usize,
    /// Input credits held by native buffers, active and retired handlers.
    pub admitted_input_bytes: usize,
    /// Expired deadlines.
    pub timeouts: u64,
    /// Explicit cancellation count.
    pub cancellations: u64,
    /// Ignored late completion IDs.
    pub stale_replies: u64,
    /// Failed notification callbacks.
    pub callback_errors: u64,
    /// Native wake invocations.
    pub wakeups: u64,
    /// Actual deadline firings (not idle polling).
    pub deadline_wakeups: usize,
    /// Whether a worker pool was activated.
    pub workers_started: bool,
    /// Whether deadline supervision was activated.
    pub timer_started: bool,
}

/// Weak handle pinned to exactly one admitted document.
#[derive(Clone)]
pub struct IpcSession {
    pub(super) core: Weak<Core>,
    pub(super) generation: u64,
}
impl IpcSession {
    /// Document generation, useful for correlation but not authorization.
    pub fn generation(&self) -> u64 {
        self.generation
    }
    /// Whether this document has been revoked or the frame closed.
    pub fn is_closed(&self) -> bool {
        let Some(core) = self.core.upgrade() else {
            return true;
        };
        let state = lock(&core.state);
        state.closed
            || !state
                .document
                .as_ref()
                .is_some_and(|doc| doc.generation == self.generation)
    }
    /// Call a generated renderer RPC. Dropping the unfinished call cancels it.
    pub fn call<M: Rpc<Receiver = Renderer>>(
        &self,
        request: M::Request,
        options: CallOptions,
    ) -> IpcCall<M::Response> {
        self.start::<M::Request, M::Response>(M::ID, MethodKind::Rpc, request, options)
    }
    /// Emit a generated renderer notification. Dropping before first poll
    /// suppresses sending; after sending it abandons acceptance, not peer work.
    pub fn notify<E: Event<Receiver = Renderer>>(&self, payload: E::Payload) -> IpcFuture<()> {
        let session = self.clone();
        Box::pin(async move {
            let timeout = session
                .core
                .upgrade()
                .map(|c| c.options.limits.notification_accept_timeout_ms)
                .ok_or_else(|| fail(IpcErrorCode::Closed))?;
            session
                .start::<E::Payload, ()>(
                    E::ID,
                    MethodKind::Notification,
                    payload,
                    CallOptions {
                        timeout: Duration::from_millis(timeout as u64),
                    },
                )
                .await
        })
    }
    fn start<Req, Resp>(
        &self,
        id: u32,
        kind: MethodKind,
        request: Req,
        options: CallOptions,
    ) -> IpcCall<Resp>
    where
        Req: IpcCodec,
        Resp: IpcCodec,
    {
        let (sender, receiver) = oneshot::channel();
        let output = Arc::new(Mutex::new(Some(sender)));
        let fail_output = Arc::clone(&output);
        let failure: Arc<dyn Fn(IpcError) + Send + Sync> = Arc::new(move |error| {
            let sender = lock(&fail_output).take();
            if let Some(sender) = sender {
                let _ = sender.send(Err(error));
            }
        });
        let result = self.prepare(id, kind, options, Arc::clone(&failure));
        let mut call = IpcCall {
            receiver,
            core: self.core.clone(),
            generation: self.generation,
            id: 0,
            done: false,
        };
        match result {
            Err(error) => failure(error),
            Ok(work) => {
                call.id = work.id;
                let core = Arc::clone(&work.core);
                let weak = Arc::downgrade(&core);
                let generation = self.generation;
                let request_id = work.id;
                let abort = work.abort;
                let task = async move {
                    let _permit = work.permit;
                    // Cancellation may skip this invocation, but cannot unlink
                    // its ordering gate while its predecessor is unfinished.
                    // Keep the admitted permit until this sequencing node exits.
                    if let Some(previous) = work.previous {
                        let _ = previous.await;
                    }
                    let outcome = Abortable::new(
                        async move {
                            let gate = work.next;
                            let result = send_request(
                                &work.core,
                                (generation, request_id),
                                id,
                                kind,
                                request,
                            );
                            drop(gate);
                            result?;
                            let payload = work
                                .receiver
                                .await
                                .map_err(|_| fail(IpcErrorCode::Closed))??;
                            let bytes = &payload.bytes;
                            let method = work.core.allowed(id, Endpoint::Renderer, kind)?;
                            if let Some(validate) = method.validate_response {
                                validate(bytes, &work.core.options.limits)?;
                            }
                            Resp::decode_ipc(bytes.as_slice())
                        },
                        abort,
                    )
                    .await;
                    if let Ok(mut result) = outcome {
                        let Some(core) = weak.upgrade() else {
                            return;
                        };
                        // Removing the pending entry wins settlement ownership.
                        // Cancellation/navigation may have removed it already;
                        // their explicit error must not race a dropped raw channel.
                        let Some(expired) = finish_local(&core, generation, request_id) else {
                            return;
                        };
                        if expired {
                            result = Err(fail(IpcErrorCode::DeadlineExceeded));
                        }
                        let sender = lock(&output).take();
                        if let Some(sender) = sender {
                            let _ = sender.send(result);
                        }
                    }
                };
                match core.workers().and_then(|workers| workers.spawn(task)) {
                    Ok(()) => {}
                    Err(error) => core.cancel_local(self.generation, call.id, error.code),
                }
            }
        }
        call
    }
    fn prepare(
        &self,
        method: u32,
        kind: MethodKind,
        options: CallOptions,
        failure: Arc<dyn Fn(IpcError) + Send + Sync>,
    ) -> Result<LocalWork, IpcError> {
        let started = Instant::now();
        let core = self
            .core
            .upgrade()
            .ok_or_else(|| fail(IpcErrorCode::Closed))?;
        core.allowed(method, Endpoint::Renderer, kind)?;
        if options.timeout.is_zero()
            || options.timeout > Duration::from_millis(core.options.limits.max_timeout_ms as u64)
        {
            return Err(fail(IpcErrorCode::InvalidPayload));
        }
        let deadline = started
            .checked_add(options.timeout)
            .ok_or_else(|| fail(IpcErrorCode::InvalidPayload))?;
        let permit = core
            .budget
            .reserve(1, core.options.limits.max_frame_bytes)?;
        core.workers()?;
        let timer = core.timer()?;
        let (sender, receiver) = oneshot::channel();
        let (abort, registration) = AbortHandle::new_pair();
        let (next, tail) = oneshot::channel();
        let id;
        let previous;
        {
            let mut state = lock(&core.state);
            let doc = Core::document(&mut state, self.generation)?;
            let count = doc
                .pending
                .values()
                .filter(|pending| pending.kind == kind)
                .count();
            let cap = match kind {
                MethodKind::Rpc => core.options.limits.max_pending_calls_per_direction,
                MethodKind::Notification => {
                    core.options
                        .limits
                        .max_outstanding_notifications_per_direction
                }
            };
            if doc.next_id == u64::MAX {
                drop(state);
                core.retire(IpcErrorCode::Closed, false);
                return Err(fail(IpcErrorCode::Closed));
            }
            if count >= cap {
                return Err(fail(IpcErrorCode::Overloaded));
            }
            let reserve = core.reservation(doc)?;
            id = doc.next_id;
            doc.next_id += 1;
            previous = doc.local_tail.take();
            doc.local_tail = Some(tail);
            doc.pending.insert(
                id,
                Pending {
                    sender: Some(sender),
                    failure,
                    abort,
                    sent: false,
                    kind,
                    deadline,
                    cancellation: Cancellation::default(),
                    reserve,
                },
            );
            timer.insert(
                deadline,
                TimerKey {
                    generation: self.generation,
                    id,
                    local: true,
                },
            );
        }
        Ok(LocalWork {
            core,
            id,
            receiver,
            abort: registration,
            previous,
            next,
            permit,
        })
    }
    /// Subscribe to a generated host notification. Each slot runs in receipt
    /// order and dropping its guard releases callbacks and skips queued work.
    pub fn subscribe<E, F, Fut>(&self, callback: F) -> Result<IpcSubscription, IpcError>
    where
        E: Event<Receiver = Host>,
        F: Fn(NotificationContext, E::Payload) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<(), IpcError>> + Send + 'static,
    {
        let core = self
            .core
            .upgrade()
            .ok_or_else(|| fail(IpcErrorCode::Closed))?;
        let method = core.allowed(E::ID, Endpoint::Host, MethodKind::Notification)?;
        let slot = Arc::new(SubscriptionSlot::new(
            E::ID,
            notification_callback::<E, F, Fut>(method, callback),
        ));
        let id;
        {
            let mut state = lock(&core.state);
            let doc = Core::document(&mut state, self.generation)?;
            if doc.subscriptions.len() >= core.options.limits.max_callbacks_per_document
                || doc
                    .subscriptions
                    .values()
                    .filter(|slot| slot.method == E::ID)
                    .count()
                    >= core.options.limits.max_callbacks_per_event
            {
                return Err(fail(IpcErrorCode::Overloaded));
            }
            id = doc.next_subscription;
            doc.next_subscription = id
                .checked_add(1)
                .ok_or_else(|| fail(IpcErrorCode::Overloaded))?;
            doc.subscriptions.insert(id, Arc::clone(&slot));
        }
        Ok(IpcSubscription {
            core: self.core.clone(),
            generation: self.generation,
            id,
            slot: Some(slot),
        })
    }
}

struct LocalWork {
    core: Arc<Core>,
    id: u64,
    receiver: oneshot::Receiver<Result<super::engine::ReceivedPayload, IpcError>>,
    abort: futures_util::future::AbortRegistration,
    previous: Option<oneshot::Receiver<()>>,
    next: oneshot::Sender<()>,
    permit: super::executor::Permit,
}
fn send_request<Req: IpcCodec>(
    core: &Core,
    address: (u64, u64),
    method_id: u32,
    kind: MethodKind,
    request: Req,
) -> Result<(), IpcError> {
    let (generation, id) = address;
    let bytes = request.encode_ipc();
    if bytes.len() > core.options.limits.max_frame_bytes.saturating_sub(64) {
        return Err(fail(IpcErrorCode::PayloadTooLarge));
    }
    let method = core.allowed(method_id, Endpoint::Renderer, kind)?;
    (method.validate_request)(&bytes, &core.options.limits)?;
    {
        let mut state = lock(&core.state);
        let doc = Core::document(&mut state, generation)?;
        let pending = doc
            .pending
            .get(&id)
            .ok_or_else(|| fail(IpcErrorCode::Cancelled))?;
        let remaining = pending
            .deadline
            .checked_duration_since(Instant::now())
            .ok_or_else(|| fail(IpcErrorCode::DeadlineExceeded))?;
        let mut frame = envelope(
            generation,
            id,
            if kind == MethodKind::Rpc {
                Kind::Request
            } else {
                Kind::Notify
            },
            Some(Body::Payload(bytes)),
        );
        frame.method_id = method_id;
        frame.timeout_ms = if kind == MethodKind::Rpc {
            u32::try_from(remaining.as_millis().max(1))
                .map_err(|_| fail(IpcErrorCode::InvalidPayload))?
        } else {
            0
        };
        core.enqueue_data(doc, frame)?;
        if let Some(pending) = doc.pending.get_mut(&id) {
            pending.sent = true;
        }
    }
    core.signal();
    Ok(())
}
fn finish_local(core: &Core, generation: u64, id: u64) -> Option<bool> {
    let pending = {
        let mut state = lock(&core.state);
        let Ok(doc) = Core::document(&mut state, generation) else {
            return None;
        };
        let pending = doc.pending.remove(&id);
        if pending.is_some() {
            doc.control_slots -= 1;
        }
        pending
    };
    if let Some(pending) = pending {
        core.remove_timer(
            pending.deadline,
            TimerKey {
                generation,
                id,
                local: true,
            },
        );
        return Some(Instant::now() >= pending.deadline);
    }
    None
}

/// An owned acknowledged call. Completion is decoded on a worker.
#[must_use = "dropping an unfinished call cancels it"]
pub struct IpcCall<T> {
    receiver: oneshot::Receiver<Result<T, IpcError>>,
    core: Weak<Core>,
    generation: u64,
    id: u64,
    done: bool,
}
impl<T> IpcCall<T> {
    /// Settle locally first, then send a best-effort cancellation.
    pub fn cancel(&self) {
        if let Some(core) = self.core.upgrade() {
            core.counters.cancellations.fetch_add(1, Ordering::Relaxed);
            core.cancel_local(self.generation, self.id, IpcErrorCode::Cancelled);
        }
    }
}
impl<T> Future for IpcCall<T> {
    type Output = Result<T, IpcError>;
    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.get_mut();
        match Pin::new(&mut this.receiver).poll(cx) {
            Poll::Pending => Poll::Pending,
            Poll::Ready(result) => {
                this.done = true;
                Poll::Ready(
                    result
                        .map_err(|_| fail(IpcErrorCode::Closed))
                        .and_then(|r| r),
                )
            }
        }
    }
}
impl<T> Drop for IpcCall<T> {
    fn drop(&mut self) {
        if !self.done {
            self.cancel();
        }
    }
}

pub(super) type Callback =
    dyn Fn(NotificationContext, Arc<Vec<u8>>, IpcLimits) -> IpcFuture<()> + Send + Sync;
pub(super) fn notification_callback<E, F, Fut>(
    method: &'static super::MethodDescriptor,
    callback: F,
) -> Arc<Callback>
where
    E: Event<Receiver = Host>,
    F: Fn(NotificationContext, E::Payload) -> Fut + Send + Sync + 'static,
    Fut: Future<Output = Result<(), IpcError>> + Send + 'static,
{
    let callback = Arc::new(callback);
    Arc::new(move |ctx, bytes, limits| {
        let callback = Arc::clone(&callback);
        Box::pin(async move {
            (method.validate_request)(&bytes, &limits)?;
            let payload = E::Payload::decode_ipc(bytes.as_slice())?;
            callback(ctx, payload).await
        })
    })
}
pub(super) struct SubscriptionSlot {
    pub method: u32,
    pub active: AtomicBool,
    pub callback: Mutex<Option<Arc<Callback>>>,
    pub tail: Mutex<Option<oneshot::Receiver<()>>>,
    pub cancellation: Cancellation,
}
impl SubscriptionSlot {
    pub fn new(method: u32, callback: Arc<Callback>) -> Self {
        Self {
            method,
            active: AtomicBool::new(true),
            callback: Mutex::new(Some(callback)),
            tail: Mutex::new(None),
            cancellation: Cancellation::default(),
        }
    }
    pub fn close(&self) {
        self.active.store(false, Ordering::Release);
        let callback = lock(&self.callback).take();
        self.cancellation.cancel();
        drop(callback);
    }
}
/// Non-cloneable document-scoped subscription registration.
#[must_use = "keep this guard alive to receive notifications"]
pub struct IpcSubscription {
    core: Weak<Core>,
    generation: u64,
    id: u64,
    slot: Option<Arc<SubscriptionSlot>>,
}
impl IpcSubscription {
    /// Remove this callback, idempotently.
    pub fn close(&mut self) {
        let Some(slot) = self.slot.take() else {
            return;
        };
        slot.close();
        let removed = self.core.upgrade().and_then(|core| {
            let mut state = lock(&core.state);
            Core::document(&mut state, self.generation)
                .ok()
                .and_then(|doc| doc.subscriptions.remove(&self.id))
        });
        drop(removed);
    }
}
impl Drop for IpcSubscription {
    fn drop(&mut self) {
        self.close();
    }
}

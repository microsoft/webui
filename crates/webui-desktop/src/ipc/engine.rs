// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

use super::{
    error::fail,
    executor::{lock, Budget, Cancellation, Permit, Timer, TimerKey, Workers},
    session::{IpcSession, SubscriptionSlot},
    wire::{ipc_frame::Body, IpcFrame, Kind},
    Endpoint, IpcError, IpcErrorCode, IpcHost, IpcLimits, IpcOptions, IpcRegistry, IpcWake,
    MethodDescriptor, MethodKind, NativeControl, IPC_VERSION,
};
use futures_channel::oneshot;
use futures_util::future::AbortHandle;
use std::{
    collections::{HashMap, VecDeque},
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc, Mutex, OnceLock,
    },
    time::Instant,
};

pub(super) struct ReceivedPayload {
    pub bytes: Vec<u8>,
    pub _credits: super::bridge::InputCredits,
}
pub(super) type ReplySender = oneshot::Sender<Result<ReceivedPayload, IpcError>>;
pub(super) struct Pending {
    pub sender: Option<ReplySender>,
    pub failure: Arc<dyn Fn(IpcError) + Send + Sync>,
    pub abort: AbortHandle,
    pub sent: bool,
    pub kind: MethodKind,
    pub deadline: Instant,
    pub cancellation: Cancellation,
    pub reserve: Reservation,
}
pub(super) struct Incoming {
    pub cancellation: Cancellation,
    pub abort: AbortHandle,
    pub deadline: Instant,
    pub reserve: Reservation,
}
pub(super) struct Reservation {
    pub memory: Permit,
}
pub(super) struct Queued {
    pub bytes: Vec<u8>,
    pub control: bool,
    pub _memory: Permit,
}
pub(super) struct Document {
    pub navigation: u64,
    pub generation: u64,
    pub token: String,
    pub next_id: u64,
    pub last_peer_id: u64,
    pub pending: HashMap<u64, Pending>,
    pub incoming: HashMap<u64, Incoming>,
    pub subscriptions: HashMap<u64, Arc<SubscriptionSlot>>,
    pub next_subscription: u64,
    pub queue: VecDeque<Queued>,
    pub data_frames: usize,
    pub data_bytes: usize,
    pub control_slots: usize,
    pub ready: bool,
    pub drain_inflight: bool,
    pub local_tail: Option<oneshot::Receiver<()>>,
    pub callback_budget: Arc<Budget>,
    pub notification_budget: Arc<Budget>,
}
impl Document {
    pub fn new(navigation: u64, generation: u64, token: String, limits: &IpcLimits) -> Self {
        Self {
            navigation,
            generation,
            token,
            next_id: 1,
            last_peer_id: 0,
            pending: HashMap::new(),
            incoming: HashMap::new(),
            subscriptions: HashMap::new(),
            next_subscription: 1,
            queue: VecDeque::new(),
            data_frames: 0,
            data_bytes: 0,
            control_slots: 0,
            ready: false,
            drain_inflight: false,
            local_tail: None,
            callback_budget: Budget::new(limits.max_callback_tasks_per_document, usize::MAX),
            notification_budget: Budget::new(
                limits.max_outstanding_notifications_per_direction,
                usize::MAX,
            ),
        }
    }
}
pub(super) struct State {
    pub closed: bool,
    pub navigation: u64,
    pub admitted: bool,
    pub generation: u64,
    pub document: Option<Document>,
    pub waiters: Vec<oneshot::Sender<Result<IpcSession, IpcError>>>,
    pub waker: Option<Arc<dyn IpcWake>>,
    pub wake_pending: bool,
    pub closed_control: Option<NativeControl>,
    pub activation: Option<super::admission::PendingActivation>,
    pub activation_started: bool,
}
#[derive(Default)]
pub(super) struct Counters {
    pub accepted: AtomicU64,
    pub rejected: AtomicU64,
    pub timeouts: AtomicU64,
    pub cancellations: AtomicU64,
    pub stale_replies: AtomicU64,
    pub callback_errors: AtomicU64,
    pub wakeups: AtomicU64,
}
pub(super) struct Core {
    pub registry: Arc<IpcRegistry>,
    pub options: IpcOptions,
    pub host: IpcHost,
    pub state: Mutex<State>,
    pub budget: Arc<Budget>,
    pub input_budget: Arc<Budget>,
    pub control_budget: Arc<Budget>,
    pub workers: OnceLock<Result<Workers, IpcError>>,
    pub timer: OnceLock<Result<Arc<Timer>, IpcError>>,
    pub counters: Counters,
}
enum DocumentChange<'a> {
    Navigate(u64),
    Disconnect { generation: u64, token: &'a str },
}
impl Core {
    pub fn document(state: &mut State, generation: u64) -> Result<&mut Document, IpcError> {
        if state.closed {
            return Err(fail(IpcErrorCode::Closed));
        }
        state
            .document
            .as_mut()
            .filter(|doc| doc.generation == generation)
            .ok_or_else(|| fail(IpcErrorCode::Navigated))
    }
    pub fn workers(&self) -> Result<&Workers, IpcError> {
        let state = lock(&self.state);
        if state.closed {
            return Err(fail(IpcErrorCode::Closed));
        }
        self.workers
            .get_or_init(|| Workers::new(self.options.worker_threads))
            .as_ref()
            .map_err(Clone::clone)
    }
    pub fn timer(self: &Arc<Self>) -> Result<&Arc<Timer>, IpcError> {
        let state = lock(&self.state);
        if state.closed {
            return Err(fail(IpcErrorCode::Closed));
        }
        self.timer
            .get_or_init(|| Timer::new(Arc::downgrade(self)))
            .as_ref()
            .map_err(Clone::clone)
    }
    pub fn remove_timer(&self, deadline: Instant, key: TimerKey) {
        if let Some(Ok(timer)) = self.timer.get() {
            timer.remove(deadline, key);
        }
    }
    pub fn allowed(
        &self,
        id: u32,
        receiver: Endpoint,
        kind: MethodKind,
    ) -> Result<&'static MethodDescriptor, IpcError> {
        let method = self.registry.method(id, receiver, kind)?;
        let granted = match receiver {
            Endpoint::Host => &self.options.allow_host,
            Endpoint::Renderer => &self.options.allow_renderer,
        };
        if !granted.contains(&id)
            || (method.development_only && (!self.options.development || !self.host.is_source()))
        {
            return Err(fail(IpcErrorCode::PermissionDenied));
        }
        Ok(method)
    }
    pub fn reservation(&self, doc: &mut Document) -> Result<Reservation, IpcError> {
        if doc.control_slots >= self.options.limits.reserved_control_frames_per_direction {
            return Err(fail(IpcErrorCode::Overloaded));
        }
        let memory = self
            .control_budget
            .reserve(0, self.options.limits.max_error_text_bytes_total + 128)?;
        doc.control_slots += 1;
        Ok(Reservation { memory })
    }
    pub fn enqueue_data(&self, doc: &mut Document, frame: IpcFrame) -> Result<(), IpcError> {
        let size = frame.encoded_len();
        let limits = &self.options.limits;
        if size > limits.max_frame_bytes {
            return Err(fail(IpcErrorCode::PayloadTooLarge));
        }
        if doc.data_frames >= limits.max_queued_frames_per_direction
            || size
                > limits
                    .max_queued_bytes_per_direction
                    .saturating_sub(doc.data_bytes)
        {
            return Err(fail(IpcErrorCode::Overloaded));
        }
        let memory = self.budget.reserve(0, size)?;
        doc.data_frames += 1;
        doc.data_bytes += size;
        doc.ready |= doc.queue.is_empty();
        doc.queue.push_back(Queued {
            bytes: frame.encode_to_vec(),
            control: false,
            _memory: memory,
        });
        Ok(())
    }
    pub fn enqueue_control(
        &self,
        doc: &mut Document,
        id: u64,
        result: (Kind, Option<Body>),
        reserve: Reservation,
    ) {
        let mut frame = envelope(doc.generation, id, result.0, result.1);
        let size = frame.encoded_len();
        let memory = if size <= self.options.limits.max_error_text_bytes_total + 128 {
            reserve.memory
        } else {
            // Large successful replies share the data byte budget, but use their
            // already-reserved completion slot. Errors always fit the reserve.
            let capacity = self
                .options
                .limits
                .max_queued_bytes_per_direction
                .saturating_sub(doc.data_bytes);
            match (size <= capacity && size <= self.options.limits.max_frame_bytes)
                .then(|| self.budget.reserve(0, size))
            {
                Some(Ok(memory)) => {
                    doc.data_bytes += size;
                    memory
                }
                _ => {
                    frame = envelope(
                        doc.generation,
                        id,
                        Kind::Error,
                        Some(Body::Error(
                            fail(IpcErrorCode::PayloadTooLarge)
                                .wire(self.options.limits.max_error_text_bytes_total),
                        )),
                    );
                    reserve.memory
                }
            }
        };
        doc.ready |= doc.queue.is_empty();
        doc.queue.push_back(Queued {
            bytes: frame.encode_to_vec(),
            control: true,
            _memory: memory,
        });
    }
    pub fn signal(&self) {
        let waker = {
            let mut state = lock(&self.state);
            if state.closed
                || state.wake_pending
                || (state.closed_control.is_none()
                    && !state.document.as_ref().is_some_and(|d| d.ready))
            {
                return;
            }
            let Some(waker) = state.waker.as_ref().map(Arc::clone) else {
                return;
            };
            state.wake_pending = true;
            waker
        };
        self.counters.wakeups.fetch_add(1, Ordering::Relaxed);
        if waker.wake().is_err() {
            self.retire(IpcErrorCode::Transport, true);
        }
    }
    pub fn complete_incoming(&self, generation: u64, id: u64, result: Result<Vec<u8>, IpcError>) {
        let pending = {
            let mut state = lock(&self.state);
            let Ok(doc) = Self::document(&mut state, generation) else {
                return;
            };
            let Some(pending) = doc.incoming.remove(&id) else {
                return;
            };
            let result = if Instant::now() >= pending.deadline {
                Err(fail(IpcErrorCode::DeadlineExceeded))
            } else {
                result
            };
            let (kind, body) = match result {
                Ok(payload) => (Kind::Result, Body::Payload(payload)),
                Err(error) => (
                    Kind::Error,
                    Body::Error(error.wire(self.options.limits.max_error_text_bytes_total)),
                ),
            };
            self.enqueue_control(doc, id, (kind, Some(body)), pending.reserve);
            (pending.deadline, pending.cancellation)
        };
        self.remove_timer(
            pending.0,
            TimerKey {
                generation,
                id,
                local: false,
            },
        );
        self.signal();
    }
    pub fn cancel_local(&self, generation: u64, id: u64, code: IpcErrorCode) {
        let pending = {
            let mut state = lock(&self.state);
            let Ok(doc) = Self::document(&mut state, generation) else {
                return;
            };
            let Some(pending) = doc.pending.remove(&id) else {
                return;
            };
            if pending.sent {
                self.enqueue_control(doc, id, (Kind::Cancel, None), pending.reserve);
            } else {
                doc.control_slots -= 1;
            }
            (
                pending.failure,
                pending.cancellation,
                pending.deadline,
                pending.abort,
            )
        };
        self.remove_timer(
            pending.2,
            TimerKey {
                generation,
                id,
                local: true,
            },
        );
        pending.1.cancel();
        pending.3.abort();
        (pending.0)(fail(code));
        self.signal();
    }
    pub fn timeout(&self, key: TimerKey) {
        self.counters.timeouts.fetch_add(1, Ordering::Relaxed);
        if key.local {
            self.cancel_local(key.generation, key.id, IpcErrorCode::DeadlineExceeded);
            return;
        }
        let pending = {
            let mut state = lock(&self.state);
            let Ok(doc) = Self::document(&mut state, key.generation) else {
                return;
            };
            let Some(pending) = doc.incoming.remove(&key.id) else {
                return;
            };
            self.enqueue_control(
                doc,
                key.id,
                (
                    Kind::Error,
                    Some(Body::Error(
                        fail(IpcErrorCode::DeadlineExceeded)
                            .wire(self.options.limits.max_error_text_bytes_total),
                    )),
                ),
                pending.reserve,
            );
            (pending.cancellation, pending.abort)
        };
        pending.0.cancel();
        pending.1.abort();
        self.signal();
    }
    pub fn retire(&self, code: IpcErrorCode, terminal: bool) {
        let _ = self.retire_matching(code, terminal, None);
    }
    pub fn navigate(&self, navigation: u64) {
        let _ = self.retire_matching(
            IpcErrorCode::Navigated,
            false,
            Some(DocumentChange::Navigate(navigation)),
        );
    }
    pub fn disconnect_authenticated(&self, generation: u64, token: &str) -> Result<(), IpcError> {
        self.retire_matching(
            IpcErrorCode::Closed,
            false,
            Some(DocumentChange::Disconnect { generation, token }),
        )
    }
    fn retire_matching(
        &self,
        code: IpcErrorCode,
        terminal: bool,
        change: Option<DocumentChange<'_>>,
    ) -> Result<(), IpcError> {
        let (document, waiters, waker) = {
            let mut state = lock(&self.state);
            if state.closed {
                return if matches!(change, Some(DocumentChange::Disconnect { .. })) {
                    Err(fail(IpcErrorCode::Closed))
                } else {
                    Ok(())
                };
            }
            match change {
                Some(DocumentChange::Navigate(navigation)) => {
                    if navigation <= state.navigation {
                        return Ok(());
                    }
                    state.navigation = navigation;
                    state.admitted = false;
                    state.activation_started = false;
                }
                Some(DocumentChange::Disconnect { generation, token }) => {
                    let doc = state
                        .document
                        .as_ref()
                        .ok_or_else(|| fail(IpcErrorCode::Closed))?;
                    let credential_matches =
                        super::credentials::constant_equal(doc.token.as_bytes(), token.as_bytes());
                    if !(credential_matches & (generation == doc.generation)) {
                        return Err(fail(IpcErrorCode::PermissionDenied));
                    }
                }
                _ => {}
            }
            state.closed = terminal;
            state.activation = None;
            let document = state.document.take();
            if let Some(doc) = &document {
                state.closed_control = Some(NativeControl::Closed {
                    generation: doc.generation,
                    code,
                });
            }
            state.wake_pending = false;
            let waiters = if terminal {
                std::mem::take(&mut state.waiters)
            } else {
                Vec::new()
            };
            let waker = if terminal { state.waker.take() } else { None };
            (document, waiters, waker)
        };
        if terminal {
            if let Some(Ok(timer)) = self.timer.get() {
                timer.stop();
            }
        }
        if let Some(document) = document {
            for (id, pending) in document.pending {
                self.remove_timer(
                    pending.deadline,
                    TimerKey {
                        generation: document.generation,
                        id,
                        local: true,
                    },
                );
                pending.cancellation.cancel();
                pending.abort.abort();
                (pending.failure)(fail(code));
            }
            for (id, pending) in document.incoming {
                self.remove_timer(
                    pending.deadline,
                    TimerKey {
                        generation: document.generation,
                        id,
                        local: false,
                    },
                );
                pending.cancellation.cancel();
                pending.abort.abort();
            }
            for (_, slot) in document.subscriptions {
                slot.close();
            }
        }
        for waiter in waiters {
            let _ = waiter.send(Err(fail(code)));
        }
        drop(waker);
        if !terminal {
            self.signal();
        }
        Ok(())
    }
}
pub(super) fn envelope(generation: u64, id: u64, kind: Kind, body: Option<Body>) -> IpcFrame {
    IpcFrame {
        version: IPC_VERSION,
        generation,
        id,
        kind: kind as i32,
        method_id: 0,
        timeout_ms: 0,
        body,
    }
}

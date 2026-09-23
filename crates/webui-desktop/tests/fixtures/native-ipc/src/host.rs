// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

use futures_channel::oneshot;
use webui_desktop::ipc::{
    CallOptions, Cancellation, IpcError, IpcErrorCode, IpcFuture, NotificationContext,
    RequestContext,
};
use webui_desktop::WindowHandle;

use crate::generated::messages::fixture::native::{Generation, Item, Report};
use crate::generated::{HostHandler, RendererClient};

fn failure(message: &str) -> IpcError {
    IpcError::new(
        IpcErrorCode::Handler,
        message,
        "inspect the native fixture assertion",
    )
}

fn validate(item: &Item) -> Result<(), IpcError> {
    if item.id != u64::MAX || ![0, 16384, 262144].contains(&item.image.len()) {
        return Err(failure("uint64 maximum or byte length mismatch"));
    }
    if item
        .image
        .iter()
        .enumerate()
        .any(|(i, byte)| usize::from(*byte) != (i * 31 + 7) % 256)
    {
        return Err(failure("binary payload mismatch"));
    }
    Ok(())
}

pub struct Host {
    lifecycle: crate::lifecycle::Lifecycle,
    saved: Arc<AtomicUsize>,
    selected: Arc<AtomicUsize>,
    cancelled: Arc<AtomicBool>,
    release: Mutex<Option<oneshot::Sender<()>>>,
    gate: Mutex<Option<oneshot::Receiver<()>>>,
    cancel_sender: Mutex<Option<oneshot::Sender<bool>>>,
    cancel_receiver: Mutex<Option<oneshot::Receiver<bool>>>,
    report: Mutex<Option<Report>>,
    done: AtomicBool,
    reported: AtomicBool,
    window: Arc<OnceLock<WindowHandle>>,
}

impl Host {
    pub fn new(window: Arc<OnceLock<WindowHandle>>) -> Self {
        let (release, gate) = oneshot::channel();
        let (cancel_sender, cancel_receiver) = oneshot::channel();
        Self {
            lifecycle: crate::lifecycle::Lifecycle::default(),
            saved: Arc::new(AtomicUsize::new(0)),
            selected: Arc::new(AtomicUsize::new(0)),
            cancelled: Arc::new(AtomicBool::new(false)),
            release: Mutex::new(Some(release)),
            gate: Mutex::new(Some(gate)),
            cancel_sender: Mutex::new(Some(cancel_sender)),
            cancel_receiver: Mutex::new(Some(cancel_receiver)),
            report: Mutex::new(None),
            done: AtomicBool::new(false),
            reported: AtomicBool::new(false),
            window,
        }
    }

    pub fn report_closed(&self, mode: &str) {
        let Ok(report) = self.report.lock() else {
            return;
        };
        let Some(report) = report.as_ref() else {
            return;
        };
        if !self.done.load(Ordering::SeqCst) {
            return;
        }
        if self.reported.swap(true, Ordering::SeqCst) {
            return;
        }
        println!(
            "NATIVE_IPC_RESULT {}",
            webui_test_utils::test_json!({
                "scope": "native-ipc-four-flow",
                "status": "pass",
                "mode": mode,
                "native_backend": crate::platform::metadata().1,
                "platform": crate::platform::metadata().0,
                "binary_sha256": std::env::var("NATIVE_IPC_BINARY_SHA256").unwrap_or_default(),
                "schema_hash": crate::generated::SCHEMA_HASH,
                "visibility": report.visibility,
                "user_agent": report.user_agent,
                "saves": self.saved.load(Ordering::SeqCst),
                "renderer_async_labels": report.labels,
                "changes": report.changes,
                "startup_notifications": self.selected.load(Ordering::SeqCst),
                "confirmations": report.confirmations,
                "payload_sizes": [0, 16384, 262144],
                "uint64": u64::MAX.to_string(),
                "invalid_input": report.invalid_rejected,
                "handler_failure_recovery": report.handler_rejected,
                "cancellation_recovery": report.cancelled,
                "unsubscribe": report.unsubscribed,
                "void_rpc_completed": report.void_completed,
                "notification_acceptance_not_completion": report.notification_accepted,
                "native_window_closed": true,
                "full_document_navigation": self.lifecycle.complete(),
                "connection_close_and_recovery": self.lifecycle.complete(),
                "retired_session_rejected": self.lifecycle.complete(),
                "same_document_generation": self.lifecycle.same_document_complete(),
                "history_fresh_admission": self.lifecycle.complete(),
                "native_disconnect_before_navigation": self.lifecycle.disconnected(),
                "persisted_restores": self.lifecycle.persisted_restores(),
                "history_renderer_callbacks_verified": self.lifecycle.complete(),
                "latency_claim": false
            })
        );
    }

    pub fn disconnect_observed(&self) -> bool {
        self.lifecycle.observe_disconnect()
    }
}

impl HostHandler for Host {
    fn save(&self, context: RequestContext, mut request: Item) -> IpcFuture<()> {
        eprintln!(
            "NATIVE_IPC_SAVE bytes={} phase={}",
            request.image.len(),
            request.phase
        );
        let saved = Arc::clone(&self.saved);
        Box::pin(async move {
            validate(&request)?;
            if request.phase == "reject" {
                return Err(failure("intentional handler rejection"));
            }
            let expected = format!("label:{}:{}", request.id, request.image.len());
            let client = RendererClient::new(context.session);
            let label = client
                .label_for(request.clone(), CallOptions::default())
                .await?;
            if label.text != expected {
                return Err(failure("renderer async label mismatch"));
            }
            request.phase = "changed".into();
            client.changed(request).await?;
            saved.fetch_add(1, Ordering::SeqCst);
            Ok(())
        })
    }

    fn selected(&self, context: NotificationContext, mut request: Item) -> IpcFuture<()> {
        eprintln!("NATIVE_IPC_SELECTED");
        if request.phase == "after-navigation" {
            let selected = Arc::clone(&self.selected);
            return Box::pin(async move {
                validate(&request)?;
                if selected.fetch_add(1, Ordering::SeqCst) != 1 {
                    return Err(failure(
                        "startup notification not isolated across navigation",
                    ));
                }
                request.phase = "confirmed-after-navigation".into();
                RendererClient::new(context.session).changed(request).await
            });
        }
        let gate = self
            .gate
            .lock()
            .map_err(|_| failure("gate poisoned"))
            .and_then(|mut gate| gate.take().ok_or_else(|| failure("duplicate selected")));
        let selected = Arc::clone(&self.selected);
        Box::pin(async move {
            validate(&request)?;
            gate?.await.map_err(|_| failure("gate dropped"))?;
            request.phase = "confirmed".into();
            selected.fetch_add(1, Ordering::SeqCst);
            RendererClient::new(context.session).changed(request).await
        })
    }

    fn release(&self, _: RequestContext, _: ()) -> IpcFuture<()> {
        let result = self
            .release
            .lock()
            .map_err(|_| failure("release poisoned"))
            .and_then(|mut release| release.take().ok_or_else(|| failure("duplicate release")))
            .and_then(|release| {
                release
                    .send(())
                    .map_err(|_| failure("selected receiver dropped"))
            });
        Box::pin(async move { result })
    }

    fn wait(&self, context: RequestContext, mut request: Item) -> IpcFuture<()> {
        let sender = self
            .cancel_sender
            .lock()
            .map_err(|_| failure("cancel sender poisoned"))
            .and_then(|mut sender| sender.take().ok_or_else(|| failure("duplicate wait")));
        Box::pin(async move {
            let _drop_notice = CancelDropNotice {
                cancellation: context.cancellation.clone(),
                sender: Some(sender?),
            };
            validate(&request)?;
            let client = RendererClient::new(context.session);
            request.phase = "waiting".into();
            client.changed(request).await?;
            context.cancellation.cancelled().await;
            Err(IpcError::new(
                IpcErrorCode::Cancelled,
                "cancelled",
                "start another request",
            ))
        })
    }

    fn cancellation_observed(&self, _: RequestContext, _: ()) -> IpcFuture<()> {
        let receiver = self
            .cancel_receiver
            .lock()
            .map_err(|_| failure("cancel receiver poisoned"))
            .and_then(|mut receiver| {
                receiver
                    .take()
                    .ok_or_else(|| failure("duplicate cancellation observation"))
            });
        let cancelled = Arc::clone(&self.cancelled);
        Box::pin(async move {
            if !receiver?
                .await
                .map_err(|_| failure("cancellation guard missing"))?
            {
                return Err(failure("handler dropped without cancellation"));
            }
            cancelled.store(true, Ordering::SeqCst);
            Ok(())
        })
    }

    fn finish(&self, _: RequestContext, request: Report) -> IpcFuture<()> {
        eprintln!("NATIVE_IPC_FINISH");
        let valid = self.saved.load(Ordering::SeqCst) == 4
            && self.selected.load(Ordering::SeqCst) == 1
            && self.cancelled.load(Ordering::SeqCst)
            && request.labels == 4
            && request.changes == 4
            && request.confirmations == 1
            && request.invalid_rejected
            && request.cancelled
            && request.unsubscribed
            && request.void_completed
            && request.notification_accepted
            && request.handler_rejected
            && matches!(request.visibility.as_str(), "visible" | "hidden")
            && !request.user_agent.is_empty();
        let result = if valid {
            self.report
                .lock()
                .map_err(|_| failure("report poisoned"))
                .map(|mut report| *report = Some(request))
        } else {
            Err(failure("final four-flow assertions failed"))
        };
        Box::pin(async move { result })
    }

    fn done(&self, _: NotificationContext, _: ()) -> IpcFuture<()> {
        let result = self
            .report
            .lock()
            .map_err(|_| failure("report poisoned"))
            .and_then(|report| {
                if report.is_none()
                    || !self.lifecycle.complete()
                    || self.selected.load(Ordering::SeqCst) != 2
                    || self.saved.load(Ordering::SeqCst) != 5
                {
                    return Err(failure(
                        "done before full protocol and lifecycle assertions",
                    ));
                }
                self.done.store(true, Ordering::SeqCst);
                self.window
                    .get()
                    .ok_or_else(|| failure("window missing"))?
                    .request_close()
                    .map_err(|_| failure("native close rejected"))
            });
        Box::pin(async move { result })
    }

    fn lifecycle_hold(&self, context: RequestContext, request: Item) -> IpcFuture<()> {
        self.lifecycle.hold(context, request)
    }

    fn lifecycle_check(&self, context: RequestContext, request: Item) -> IpcFuture<()> {
        self.lifecycle.check(context, request)
    }

    fn session_generation(&self, context: RequestContext, _: ()) -> IpcFuture<Generation> {
        Box::pin(async move {
            Ok(Generation {
                value: context.session.generation(),
                phase: String::new(),
            })
        })
    }

    fn same_document(&self, context: RequestContext, request: Generation) -> IpcFuture<()> {
        self.lifecycle.same_document(context, request)
    }

    fn history_probe(&self, context: RequestContext, proof: Generation) -> IpcFuture<()> {
        self.lifecycle.history_probe(context, proof)
    }

    fn history_verified(&self, context: RequestContext, proof: Generation) -> IpcFuture<()> {
        self.lifecycle.history_verified(context, proof)
    }
}

struct CancelDropNotice {
    cancellation: Cancellation,
    sender: Option<oneshot::Sender<bool>>,
}

impl Drop for CancelDropNotice {
    fn drop(&mut self) {
        if let Some(sender) = self.sender.take() {
            if sender.send(self.cancellation.is_cancelled()).is_err() {
                eprintln!("NATIVE_IPC_FAILURE cancellation observation receiver dropped");
            }
        }
    }
}

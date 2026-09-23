// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use futures_channel::oneshot;
use webui_desktop::ipc::{
    CallOptions, Cancellation, IpcError, IpcErrorCode, IpcFuture, IpcSession, RequestContext,
};

use crate::generated::{
    messages::fixture::native::{Generation, Item},
    RendererClient,
};

const PHASES: [&str; 5] = [
    "navigation",
    "close",
    "history-forward",
    "history-back",
    "history-return",
];

#[derive(Default)]
pub struct Lifecycle {
    retired: Mutex<Option<Retired>>,
    completed: Arc<Mutex<Vec<String>>>,
    same_document: Mutex<Vec<String>>,
    disconnected: AtomicBool,
    history_checks: AtomicUsize,
    persisted_checks: AtomicUsize,
}

struct Retired {
    session: IpcSession,
    dropped: oneshot::Receiver<bool>,
    phase: String,
}

struct DropNotice {
    cancellation: Cancellation,
    sender: Option<oneshot::Sender<bool>>,
}

fn error(message: &str) -> IpcError {
    IpcError::new(
        IpcErrorCode::Handler,
        message,
        "inspect fixture lifecycle assertions",
    )
}

impl Lifecycle {
    pub fn hold(&self, context: RequestContext, request: Item) -> IpcFuture<()> {
        let (sender, dropped) = oneshot::channel();
        let result = self
            .retired
            .lock()
            .map_err(|_| error("retired session poisoned"))
            .and_then(|mut retired| {
                if retired.is_some() || !PHASES.contains(&request.phase.as_str()) {
                    return Err(error("unexpected lifecycle probe"));
                }
                *retired = Some(Retired {
                    session: context.session.clone(),
                    dropped,
                    phase: request.phase.clone(),
                });
                Ok(())
            });
        Box::pin(async move {
            result?;
            let _notice = DropNotice {
                cancellation: context.cancellation.clone(),
                sender: Some(sender),
            };
            RendererClient::new(context.session)
                .changed(request)
                .await?;
            context.cancellation.cancelled().await;
            Err(error("lifecycle hold must be retired"))
        })
    }

    pub fn check(&self, context: RequestContext, request: Item) -> IpcFuture<()> {
        let retired = self
            .retired
            .lock()
            .map_err(|_| error("retired session poisoned"))
            .and_then(|mut retired| retired.take().ok_or_else(|| error("no retired session")));
        let completed = Arc::clone(&self.completed);
        Box::pin(async move {
            let retired = retired?;
            if request.phase != retired.phase
                || !retired.session.is_closed()
                || retired.session.generation() == context.session.generation()
                || !retired
                    .dropped
                    .await
                    .map_err(|_| error("retirement guard missing"))?
            {
                return Err(error("old document not cancelled and revoked"));
            }
            let stale = RendererClient::new(retired.session)
                .label_for(request, CallOptions::default())
                .await;
            if !matches!(
                stale,
                Err(IpcError {
                    code: IpcErrorCode::Navigated,
                    ..
                })
            ) {
                return Err(error(
                    "old session did not reject renderer request as navigated",
                ));
            }
            let mut completed = completed
                .lock()
                .map_err(|_| error("completion list poisoned"))?;
            if PHASES.get(completed.len()).copied() != Some(retired.phase.as_str()) {
                return Err(error("lifecycle phases out of order"));
            }
            eprintln!("NATIVE_IPC_LIFECYCLE_ASSERTED {}", retired.phase);
            completed.push(retired.phase);
            Ok(())
        })
    }

    pub fn complete(&self) -> bool {
        self.completed
            .lock()
            .is_ok_and(|completed| completed.as_slice() == PHASES)
            && self.same_document_complete()
            && self.disconnected.load(Ordering::SeqCst)
            && self.history_checks.load(Ordering::SeqCst) == 2
    }

    pub fn same_document(&self, context: RequestContext, request: Generation) -> IpcFuture<()> {
        let result = self
            .same_document
            .lock()
            .map_err(|_| error("same document state poisoned"))
            .and_then(|mut phases| {
                if request.value != context.session.generation()
                    || ["hash", "spa", "spa-back", "spa-forward"]
                        .get(phases.len())
                        .copied()
                        != Some(request.phase.as_str())
                {
                    return Err(error(
                        "same-document navigation changed generation or phase order",
                    ));
                }
                phases.push(request.phase);
                Ok(())
            });
        Box::pin(async move { result })
    }

    pub fn same_document_complete(&self) -> bool {
        self.same_document
            .lock()
            .is_ok_and(|phases| phases.as_slice() == ["hash", "spa", "spa-back", "spa-forward"])
    }

    pub fn observe_disconnect(&self) -> bool {
        let observed = self.retired.lock().is_ok_and(|retired| {
            retired
                .as_ref()
                .is_some_and(|retired| retired.phase == "close" && retired.session.is_closed())
        });
        if observed {
            self.disconnected.store(true, Ordering::SeqCst);
            eprintln!("NATIVE_IPC_NATIVE_DISCONNECT_ASSERTED before navigation");
        } else {
            eprintln!("NATIVE_IPC_FAILURE native disconnect has not settled before observation");
        }
        observed
    }

    pub fn disconnected(&self) -> bool {
        self.disconnected.load(Ordering::SeqCst)
    }

    pub fn history_probe(&self, context: RequestContext, proof: Generation) -> IpcFuture<()> {
        Box::pin(async move {
            validate_history_proof(&context, &proof)?;
            let client = RendererClient::new(context.session);
            let request = Item {
                id: u64::MAX,
                image: Vec::new(),
                phase: "history-probe".into(),
            };
            let label = client
                .label_for(request.clone(), CallOptions::default())
                .await?;
            if label.text != "label:18446744073709551615:0" {
                return Err(error("restored renderer RPC response mismatch"));
            }
            client.changed(request).await
        })
    }

    pub fn history_verified(&self, context: RequestContext, proof: Generation) -> IpcFuture<()> {
        let result = validate_history_proof(&context, &proof).and_then(|()| {
            let count = self.history_checks.load(Ordering::SeqCst);
            let suffix = if count == 0 { "-back" } else { "-forward" };
            if count >= 2 || !proof.phase.ends_with(suffix) {
                return Err(error("history renderer proofs out of order"));
            }
            self.history_checks.fetch_add(1, Ordering::SeqCst);
            if proof.phase.starts_with("persisted-") {
                self.persisted_checks.fetch_add(1, Ordering::SeqCst);
            }
            eprintln!(
                "NATIVE_IPC_HISTORY_RENDERER_ASSERTED {} old={} new={}",
                proof.phase,
                proof.value,
                context.session.generation()
            );
            Ok(())
        });
        Box::pin(async move { result })
    }

    pub fn persisted_restores(&self) -> usize {
        self.persisted_checks.load(Ordering::SeqCst)
    }
}

fn validate_history_proof(context: &RequestContext, proof: &Generation) -> Result<(), IpcError> {
    let valid = match proof.phase.as_str() {
        "persisted-back" | "persisted-forward" => {
            proof.value > 0 && proof.value < context.session.generation()
        }
        "fresh-back" | "fresh-forward" => proof.value == 0,
        _ => false,
    };
    if valid {
        Ok(())
    } else {
        Err(error("invalid history generation proof"))
    }
}

impl Drop for DropNotice {
    fn drop(&mut self) {
        if let Some(sender) = self.sender.take() {
            if sender.send(self.cancellation.is_cancelled()).is_err() {
                eprintln!("NATIVE_IPC_FAILURE lifecycle observer dropped");
            }
        }
    }
}

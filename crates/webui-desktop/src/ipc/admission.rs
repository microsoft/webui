// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

use super::{
    credentials::constant_equal,
    engine::{Core, Document, State},
    error::fail,
    executor::lock,
    Admission, CommittedMainDocument, IpcBridge, IpcError, IpcErrorCode, IpcFuture, IpcSession,
    SessionInfo, IPC_VERSION,
};
use futures_channel::oneshot;
use std::{
    sync::Arc,
    time::{Duration, Instant},
};

/// One native-owned post-commit challenge. Never embed in persistent scripts,
/// URLs or logs. Its nonce comes only from probing the committed main document.
#[derive(Clone, Eq, PartialEq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DocumentActivation {
    /// Trusted native navigation identity.
    #[serde(serialize_with = "decimal")]
    pub navigation: u64,
    /// Browser bootstrap's per-document cryptographic nonce.
    #[serde(serialize_with = "hex_bytes")]
    pub document_nonce: [u8; 16],
    /// Independent OS-random proof, consumed by exactly one successful hello.
    #[serde(serialize_with = "hex_bytes")]
    pub challenge: [u8; 16],
}
impl std::fmt::Debug for DocumentActivation {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("DocumentActivation")
            .field("navigation", &self.navigation)
            .finish_non_exhaustive()
    }
}
pub(super) struct PendingActivation {
    pub document: CommittedMainDocument,
    pub proof: DocumentActivation,
    pub deadline: Instant,
}
pub(super) fn decimal<S: serde::Serializer>(value: &u64, serializer: S) -> Result<S::Ok, S::Error> {
    serializer.serialize_str(&value.to_string())
}
fn hex_bytes<S: serde::Serializer>(value: &[u8; 16], serializer: S) -> Result<S::Ok, S::Error> {
    serializer.serialize_str(&hex(value))
}
fn hex(value: &[u8; 16]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut text = String::with_capacity(32);
    for byte in value {
        text.push(char::from(DIGITS[usize::from(byte >> 4)]));
        text.push(char::from(DIGITS[usize::from(byte & 15)]));
    }
    text
}
#[cfg(not(target_arch = "wasm32"))]
fn random() -> Result<[u8; 16], IpcError> {
    let mut value = [0; 16];
    getrandom::fill(&mut value).map_err(|_| {
        IpcError::new(
            IpcErrorCode::Transport,
            "OS randomness is unavailable",
            "restore the operating system random source before admitting desktop documents",
        )
    })?;
    Ok(value)
}
#[cfg(target_arch = "wasm32")]
fn random() -> Result<[u8; 16], IpcError> {
    Err(IpcError::new(
        IpcErrorCode::Transport,
        "desktop IPC admission is unavailable on wasm32",
        "run the desktop host on a native target",
    ))
}
impl IpcBridge {
    /// Begin exactly one post-commit activation for the current navigation.
    /// Call only after a successful, navigation-guarded native bootstrap probe.
    /// This does not start application workers or a timer.
    pub fn begin_document(
        &self,
        document: CommittedMainDocument,
        document_nonce: [u8; 16],
    ) -> Result<DocumentActivation, IpcError> {
        let core = self
            .core
            .upgrade()
            .ok_or_else(|| fail(IpcErrorCode::Closed))?;
        check_identity(&core, &document)?;
        if !core.registry.is_enabled() {
            return Err(fail(IpcErrorCode::NotReady));
        }
        let challenge = random()?;
        let mut state = lock(&core.state);
        if state.closed {
            return Err(fail(IpcErrorCode::Closed));
        }
        if document.navigation != state.navigation {
            return Err(fail(IpcErrorCode::Navigated));
        }
        if state.activation_started || state.admitted {
            return Err(fail(IpcErrorCode::NotReady));
        }
        let proof = DocumentActivation {
            navigation: document.navigation,
            document_nonce,
            challenge,
        };
        state.activation = Some(PendingActivation {
            document,
            proof: proof.clone(),
            deadline: Instant::now()
                + Duration::from_millis(core.options.limits.handshake_timeout_ms as u64),
        });
        state.activation_started = true;
        Ok(proof)
    }
    /// Atomically consume valid proof and create one session. Invalid or stale
    /// proof never consumes a newer activation. The completion runs off-thread.
    pub fn admit(&self, admission: Admission) -> IpcFuture<SessionInfo> {
        let weak = self.core.clone();
        let setup = (|| {
            let core = self
                .core
                .upgrade()
                .ok_or_else(|| fail(IpcErrorCode::Closed))?;
            check_admission(&core, &admission)?;
            let permit = core.budget.reserve(1, 0)?;
            core.workers()?;
            let (sender, receiver) = oneshot::channel();
            let worker_core = Arc::clone(&core);
            core.workers()?.spawn(async move {
                let _permit = permit;
                // Cancellation before dispatch must not consume the challenge.
                if sender.is_canceled() {
                    return;
                }
                let result = admit(&worker_core, admission).map(|info| AdmissionDelivery {
                    core: Arc::downgrade(&worker_core),
                    info: Some(info),
                });
                let _ = sender.send(result);
            })?;
            Ok(receiver)
        })();
        Box::pin(async move {
            let delivery = setup?.await.map_err(|_| fail(IpcErrorCode::Closed))??;
            let info = delivery
                .info
                .as_ref()
                .ok_or_else(|| fail(IpcErrorCode::Closed))?;
            let core = weak.upgrade().ok_or_else(|| fail(IpcErrorCode::Closed))?;
            Core::document(&mut lock(&core.state), info.generation)?;
            delivery.into_info()
        })
    }
}
struct AdmissionDelivery {
    core: std::sync::Weak<Core>,
    info: Option<SessionInfo>,
}
impl AdmissionDelivery {
    fn into_info(mut self) -> Result<SessionInfo, IpcError> {
        self.info.take().ok_or_else(|| fail(IpcErrorCode::Closed))
    }
}
impl Drop for AdmissionDelivery {
    fn drop(&mut self) {
        if let (Some(core), Some(info)) = (self.core.upgrade(), self.info.as_ref()) {
            let _ = core.disconnect_authenticated(info.generation, &info.token);
        }
    }
}
fn check_identity(core: &Core, document: &CommittedMainDocument) -> Result<(), IpcError> {
    if document.navigation == 0
        || document.origin.len() > 256
        || document.origin != core.host.origin()
    {
        return Err(fail(IpcErrorCode::PermissionDenied));
    }
    Ok(())
}
fn check_admission(core: &Core, admission: &Admission) -> Result<(), IpcError> {
    let schema = core
        .registry
        .schema
        .ok_or_else(|| fail(IpcErrorCode::NotReady))?;
    let hello = &admission.hello;
    if hello.wire_version != IPC_VERSION {
        return Err(fail(IpcErrorCode::UnsupportedVersion));
    }
    if hello.contract_name.len() > 256
        || hello.schema_hash.len() != 64
        || hello.contract_name != schema.name
        || hello.contract_major != schema.major
        || hello.schema_hash != schema.hash
    {
        return Err(fail(IpcErrorCode::SchemaMismatch));
    }
    let state = lock(&core.state);
    check_activation(core, &state, &admission.proof)
}
fn check_activation(
    core: &Core,
    state: &State,
    proof: &DocumentActivation,
) -> Result<(), IpcError> {
    if state.closed {
        return Err(fail(IpcErrorCode::Closed));
    }
    if state.admitted {
        return Err(fail(IpcErrorCode::NotReady));
    }
    let pending = state
        .activation
        .as_ref()
        .ok_or_else(|| fail(IpcErrorCode::NotReady))?;
    check_identity(core, &pending.document)?;
    if state.navigation != pending.document.navigation {
        return Err(fail(IpcErrorCode::Navigated));
    }
    // Evaluate both secret comparisons, irrespective of the first result.
    let nonce_matches = constant_equal(&pending.proof.document_nonce, &proof.document_nonce);
    let challenge_matches = constant_equal(&pending.proof.challenge, &proof.challenge);
    if !(nonce_matches & challenge_matches & (pending.document.navigation == proof.navigation)) {
        return Err(fail(IpcErrorCode::PermissionDenied));
    }
    if Instant::now() >= pending.deadline {
        return Err(fail(IpcErrorCode::DeadlineExceeded));
    }
    Ok(())
}
fn admit(core: &Arc<Core>, admission: Admission) -> Result<SessionInfo, IpcError> {
    check_admission(core, &admission)?;
    let token = hex(&random()?);
    let (generation, waiters) = {
        let mut state = lock(&core.state);
        check_activation(core, &state, &admission.proof)?;
        let generation = state
            .generation
            .checked_add(1)
            .ok_or_else(|| fail(IpcErrorCode::Closed))?;
        state.activation = None;
        state.admitted = true;
        state.generation = generation;
        let mut document = Document::new(
            state.navigation,
            generation,
            token.clone(),
            &core.options.limits,
        );
        for (&method, callback) in &core.registry.notifications {
            let id = document.next_subscription;
            document.next_subscription += 1;
            document.subscriptions.insert(
                id,
                Arc::new(super::session::SubscriptionSlot::new(
                    method,
                    Arc::clone(callback),
                )),
            );
        }
        state.document = Some(document);
        (generation, std::mem::take(&mut state.waiters))
    };
    let session = IpcSession {
        core: Arc::downgrade(core),
        generation,
    };
    for waiter in waiters {
        let _ = waiter.send(Ok(session.clone()));
    }
    Ok(SessionInfo {
        generation,
        token,
        limits: core.options.limits.clone(),
    })
}

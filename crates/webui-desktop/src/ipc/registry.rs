// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

use super::{
    error::fail,
    session::{notification_callback, Callback},
    IpcCodec, IpcError, IpcErrorCode, IpcFuture, IpcLimits, NotificationContext, RequestContext,
};
use std::{collections::HashMap, future::Future, sync::Arc};

/// Rust receiver marker.
pub struct Host;
/// JavaScript receiver marker.
pub struct Renderer;

/// A generated acknowledged RPC (including an ordinary `()` response).
///
/// Rust cannot call a host receiver:
/// ```compile_fail
/// use webui_desktop::ipc::*;
/// struct Save;
/// impl Rpc for Save { type Request = (); type Response = (); type Receiver = Host; const ID: u32 = 1101; }
/// fn invalid(session: &IpcSession) { session.call::<Save>((), CallOptions::default()); }
/// ```
/// An RPC cannot be sent as an event:
/// ```compile_fail
/// use webui_desktop::ipc::*;
/// struct Label;
/// impl Rpc for Label { type Request = (); type Response = (); type Receiver = Renderer; const ID: u32 = 2001; }
/// fn invalid(session: &IpcSession) { session.notify::<Label>(()); }
/// ```
/// A renderer receiver cannot be installed in a Rust registry:
/// ```compile_fail
/// use webui_desktop::ipc::*;
/// struct Label;
/// impl Rpc for Label { type Request = (); type Response = (); type Receiver = Renderer; const ID: u32 = 2001; }
/// fn invalid(registry: &mut IpcRegistry) { registry.register::<Label, _, _>(|_, _| async { Ok(()) }); }
/// ```
/// Requests retain the generated type:
/// ```compile_fail
/// use webui_desktop::ipc::*;
/// struct Label;
/// impl Rpc for Label { type Request = (); type Response = (); type Receiver = Renderer; const ID: u32 = 2001; }
/// fn invalid(session: &IpcSession) { session.call::<Label>("not a request", CallOptions::default()); }
/// ```
pub trait Rpc: Send + Sync + 'static {
    /// Generated request payload type.
    type Request: IpcCodec;
    /// Generated response payload type.
    type Response: IpcCodec;
    /// Endpoint implementing this method.
    type Receiver: Send + Sync + 'static;
    /// Stable application method ID.
    const ID: u32;
}
/// A generated notification, never an RPC.
pub trait Event: Send + Sync + 'static {
    /// Generated event payload type.
    type Payload: IpcCodec + Clone;
    /// Endpoint receiving this event.
    type Receiver: Send + Sync + 'static;
    /// Stable application event ID.
    const ID: u32;
}
/// Runtime endpoint metadata.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Endpoint {
    /// Rust.
    Host,
    /// JavaScript.
    Renderer,
}
/// Whether a method waits for completion or only dispatch admission.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MethodKind {
    /// Acknowledged call.
    Rpc,
    /// One-way delivery with bounded acceptance.
    Notification,
}
/// Generated iterative binary validator.
pub type PayloadValidator = fn(&[u8], &IpcLimits) -> Result<(), IpcError>;
/// Generated schema method metadata.
pub struct MethodDescriptor {
    /// Stable application ID (greater than 1023).
    pub id: u32,
    /// Fully qualified schema name.
    pub name: &'static str,
    /// Receiver, not sender.
    pub receiver: Endpoint,
    /// RPC or notification.
    pub kind: MethodKind,
    /// Requires development permission and a source host.
    pub development_only: bool,
    /// Runs before application decoding.
    pub validate_request: PayloadValidator,
    /// Present only for RPCs.
    pub validate_response: Option<PayloadValidator>,
}
/// Exact schema identity embedded by code generation.
pub struct IpcSchema {
    /// Contract name.
    pub name: &'static str,
    /// Breaking application version.
    pub major: u32,
    /// Normalized SHA-256 lowercase hex digest.
    pub hash: &'static str,
    /// Generated methods, unique by ID.
    pub methods: &'static [MethodDescriptor],
}

pub(super) type Handler =
    dyn Fn(RequestContext, Vec<u8>, IpcLimits) -> IpcFuture<Vec<u8>> + Send + Sync;
/// Immutable handler definitions shared by frames, never mutable document state.
#[derive(Default)]
pub struct IpcRegistry {
    pub(super) schema: Option<&'static IpcSchema>,
    pub(super) handlers: HashMap<u32, Arc<Handler>>,
    pub(super) notifications: HashMap<u32, Arc<Callback>>,
}
impl IpcRegistry {
    /// Enable one schema. The default registry instead disables IPC entirely.
    pub fn new(schema: &'static IpcSchema) -> Self {
        Self {
            schema: Some(schema),
            handlers: HashMap::new(),
            notifications: HashMap::new(),
        }
    }
    /// Whether a schema was explicitly configured.
    pub fn is_enabled(&self) -> bool {
        self.schema.is_some()
    }
    /// Configured schema, if IPC is enabled.
    pub fn schema(&self) -> Option<&'static IpcSchema> {
        self.schema
    }
    /// Check service IDs before a generated registration transaction mutates it.
    pub fn validate_registration(&self, ids: &[u32]) -> Result<(), IpcError> {
        for (index, id) in ids.iter().enumerate() {
            let method = self
                .schema
                .and_then(|schema| schema.methods.iter().find(|method| method.id == *id))
                .ok_or_else(|| fail(IpcErrorCode::UnknownMethod))?;
            if method.receiver != Endpoint::Host {
                return Err(fail(IpcErrorCode::InvalidFrame));
            }
            if self.handlers.contains_key(id)
                || self.notifications.contains_key(id)
                || ids[..index].contains(id)
            {
                return Err(fail(IpcErrorCode::InvalidPayload));
            }
        }
        Ok(())
    }
    /// Install one immutable startup receiver for a generated host notification.
    /// It is activated in every admitted document before readiness and shares
    /// that document's callback budgets with dynamic RAII subscriptions.
    pub fn register_notification<E, F, Fut>(&mut self, callback: F) -> Result<(), IpcError>
    where
        E: Event<Receiver = Host>,
        F: Fn(NotificationContext, E::Payload) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<(), IpcError>> + Send + 'static,
    {
        self.validate_registration(&[E::ID])?;
        let method = self.method(E::ID, Endpoint::Host, MethodKind::Notification)?;
        self.notifications
            .insert(E::ID, notification_callback::<E, F, Fut>(method, callback));
        Ok(())
    }
    /// Register an asynchronous Rust receiver. Duplicates never overwrite.
    pub fn register<M, F, Fut>(&mut self, handler: F) -> Result<(), IpcError>
    where
        M: Rpc<Receiver = Host>,
        F: Fn(RequestContext, M::Request) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<M::Response, IpcError>> + Send + 'static,
    {
        self.validate_registration(&[M::ID])?;
        let method = self.method(M::ID, Endpoint::Host, MethodKind::Rpc)?;
        let handler = Arc::new(handler);
        self.handlers.insert(
            M::ID,
            Arc::new(move |ctx, bytes, limits| {
                let handler = Arc::clone(&handler);
                Box::pin(async move {
                    (method.validate_request)(&bytes, &limits)?;
                    let request = M::Request::decode_ipc(bytes.as_slice())?;
                    let response = handler(ctx, request).await?;
                    let payload = response.encode_ipc();
                    if payload.len() > limits.max_frame_bytes.saturating_sub(64) {
                        return Err(fail(IpcErrorCode::PayloadTooLarge));
                    }
                    let validate = method
                        .validate_response
                        .ok_or_else(|| fail(IpcErrorCode::InvalidPayload))?;
                    validate(&payload, &limits)?;
                    Ok(payload)
                })
            }),
        );
        Ok(())
    }
    pub(super) fn method(
        &self,
        id: u32,
        endpoint: Endpoint,
        kind: MethodKind,
    ) -> Result<&'static MethodDescriptor, IpcError> {
        let method = self
            .schema
            .and_then(|schema| schema.methods.iter().find(|m| m.id == id))
            .ok_or_else(|| fail(IpcErrorCode::UnknownMethod))?;
        if method.receiver != endpoint || method.kind != kind {
            return Err(fail(IpcErrorCode::InvalidFrame));
        }
        Ok(method)
    }
    pub(super) fn validate(&self) -> Result<(), IpcError> {
        let Some(schema) = self.schema else {
            return Ok(());
        };
        if schema.name.is_empty()
            || schema.name.len() > 256
            || schema.major == 0
            || schema.hash.len() != 64
            || !schema
                .hash
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        {
            return Err(fail(IpcErrorCode::SchemaMismatch));
        }
        for (index, method) in schema.methods.iter().enumerate() {
            if method.id <= 1023
                || schema.methods[..index].iter().any(|m| m.id == method.id)
                || (method.kind == MethodKind::Rpc) != method.validate_response.is_some()
            {
                return Err(fail(IpcErrorCode::SchemaMismatch));
            }
        }
        Ok(())
    }
}

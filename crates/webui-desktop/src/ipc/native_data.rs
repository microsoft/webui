// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

//! Private native binary lane for owned local-server documents. The existing
//! IPC bridge still owns admission, frame decoding, grants, work and queues;
//! these cursors only marshal bounded base64 across the webview API.

use std::{
    cell::RefCell,
    rc::Rc,
    time::{Duration, Instant},
};

use base64::{engine::general_purpose::STANDARD, Engine as _};
use serde::{Deserialize, Serialize};

use super::{
    wire::WireError, IpcBridge, IpcError, IpcErrorCode, IpcInputPermit, OwnedIpcHttpRequest,
    SessionInfo,
};
use crate::{DesktopHttpMethod, DesktopProtocolResponse, DesktopResponseBody};

pub(crate) const CHUNK_BYTES: usize = 24 * 1024;
pub(crate) const CHUNK_BASE64: usize = 4 * (CHUNK_BYTES / 3);
pub(crate) const MAX_MESSAGE_UNITS: usize = CHUNK_BASE64 + 1024;
const CURSOR_DEADLINE: Duration = Duration::from_secs(5);

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct DataRequest {
    pub kind: String,
    pub version: u8,
    pub call_id: String,
    pub generation: String,
    pub token: String,
    pub operation: String,
    pub offset: usize,
    pub total_bytes: Option<usize>,
    pub data: Option<String>,
    pub max_bytes: Option<usize>,
}

impl DataRequest {
    pub(crate) fn authenticated(&self, session: &SessionInfo) -> Result<(), IpcError> {
        if self.kind != "ipcData"
            || self.version != 1
            || self.call_id.is_empty()
            || self.call_id.len() > 20
            || self.call_id.starts_with('0')
            || !self.call_id.bytes().all(|b| b.is_ascii_digit())
            || self.generation != session.generation.to_string()
            || !super::credentials::constant_equal(self.token.as_bytes(), session.token.as_bytes())
        {
            return Err(error(IpcErrorCode::PermissionDenied));
        }
        Ok(())
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct DataReply {
    kind: &'static str,
    version: u8,
    call_id: String,
    generation: String,
    operation: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    next_offset: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    total_bytes: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    offset: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    data: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    complete: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    empty: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<DataError>,
}
#[derive(Serialize)]
struct DataError {
    code: &'static str,
}

impl DataReply {
    fn new(request: &DataRequest) -> Self {
        Self {
            kind: "ipcDataResult",
            version: 1,
            call_id: request.call_id.clone(),
            generation: request.generation.clone(),
            operation: request.operation.clone(),
            next_offset: None,
            total_bytes: None,
            offset: None,
            data: None,
            complete: None,
            empty: None,
            error: None,
        }
    }
    pub(crate) fn rejected(request: &DataRequest, code: IpcErrorCode) -> Self {
        let mut reply = Self::new(request);
        reply.error = Some(DataError {
            code: code.as_str(),
        });
        reply
    }
}

struct Incoming {
    bytes: Vec<u8>,
    permit: IpcInputPermit,
    written: usize,
    until: Instant,
}
struct Outgoing {
    bytes: DesktopResponseBody,
    offset: usize,
    until: Instant,
}

#[derive(Default)]
pub(crate) struct NativeData {
    incoming: Option<Incoming>,
    outgoing: Option<Outgoing>,
    receiving: bool,
    submitting: bool,
}

impl NativeData {
    pub(crate) fn reset(&mut self) {
        self.incoming = None;
        self.outgoing = None;
        self.receiving = false;
        self.submitting = false;
    }

    /// Native adapters arm one UI-loop timer for the earliest held cursor.
    /// No timer or background work exists until a partial frame is retained.
    pub(crate) fn next_deadline(&self) -> Option<Instant> {
        self.incoming
            .as_ref()
            .map(|cursor| cursor.until)
            .into_iter()
            .chain(self.outgoing.as_ref().map(|cursor| cursor.until))
            .min()
    }

    /// Release expired input credits and output leases on the native timer
    /// callback, even if the page never sends another message.
    pub(crate) fn expire(&mut self, now: Instant) {
        if self
            .incoming
            .as_ref()
            .is_some_and(|cursor| now >= cursor.until)
        {
            self.incoming = None;
        }
        if self
            .outgoing
            .as_ref()
            .is_some_and(|cursor| now >= cursor.until)
        {
            self.outgoing = None;
        }
    }

    #[cfg(test)]
    pub(crate) fn seed_test_incoming(&mut self, permit: IpcInputPermit, until: Instant) {
        self.incoming = Some(Incoming {
            bytes: vec![0; 4096],
            permit,
            written: 1,
            until,
        });
    }

    #[cfg(test)]
    pub(crate) fn seed_test_outgoing(&mut self, bytes: DesktopResponseBody, until: Instant) {
        self.outgoing = Some(Outgoing {
            bytes,
            offset: 0,
            until,
        });
    }

    /// One outstanding native operation, tied to an authenticated current
    /// document by the adapter. Never contact the application HTTP listener.
    pub(crate) async fn exchange(
        state: &Rc<RefCell<Self>>,
        bridge: IpcBridge,
        navigation: u64,
        session: SessionInfo,
        request: DataRequest,
    ) -> DataReply {
        let result = async {
            request.authenticated(&session)?;
            match request.operation.as_str() {
                "send" => Self::send(state, &bridge, navigation, &session, &request).await,
                "receive" => Self::receive(state, &bridge, navigation, &session, &request).await,
                _ => Err(error(IpcErrorCode::InvalidFrame)),
            }
        }
        .await;
        match result {
            Ok(reply) => reply,
            Err(error) => {
                if request.operation == "send" && recoverable_send_error(error.code) {
                    // JS preserves the native session for these categories.
                    // Release partial bytes and credits before the next
                    // offset-zero send, never replay an incomplete frame.
                    state.borrow_mut().incoming = None;
                }
                DataReply::rejected(&request, error.code)
            }
        }
    }

    async fn send(
        state: &Rc<RefCell<Self>>,
        bridge: &IpcBridge,
        navigation: u64,
        session: &SessionInfo,
        request: &DataRequest,
    ) -> Result<DataReply, IpcError> {
        let total = request
            .total_bytes
            .ok_or_else(|| error(IpcErrorCode::InvalidFrame))?;
        let encoded = request
            .data
            .as_deref()
            .ok_or_else(|| error(IpcErrorCode::InvalidFrame))?;
        if request.max_bytes.is_some()
            || total == 0
            || total > session.limits.max_frame_bytes
            || encoded.is_empty()
            || encoded.len() > CHUNK_BASE64
            || encoded.len() % 4 != 0
        {
            return Err(error(IpcErrorCode::PayloadTooLarge));
        }
        let mut data = vec![0_u8; CHUNK_BYTES];
        let count = STANDARD
            .decode_slice(encoded, &mut data)
            .map_err(|_| error(IpcErrorCode::InvalidFrame))?;
        if count == 0
            || request
                .offset
                .checked_add(count)
                .is_none_or(|end| end > total)
        {
            return Err(error(IpcErrorCode::InvalidFrame));
        }
        let complete = request.offset + count == total;
        let ready = {
            let mut cursor = state.borrow_mut();
            if cursor.submitting {
                return Err(error(IpcErrorCode::Overloaded));
            }
            if cursor
                .incoming
                .as_ref()
                .is_some_and(|incoming| Instant::now() >= incoming.until)
            {
                cursor.incoming = None;
            }
            if request.offset == 0 && cursor.incoming.is_some() {
                return Err(error(IpcErrorCode::Overloaded));
            }
            if request.offset == 0 && cursor.incoming.is_none() {
                let permit = bridge.reserve_input(total)?;
                // Capacity is paid before allocating the entire untrusted frame.
                cursor.incoming = Some(Incoming {
                    bytes: vec![0; total],
                    permit,
                    written: 0,
                    until: Instant::now() + CURSOR_DEADLINE,
                });
            }
            let incoming = cursor
                .incoming
                .as_mut()
                .ok_or_else(|| error(IpcErrorCode::InvalidFrame))?;
            if incoming.bytes.len() != total || request.offset != incoming.written {
                return Err(error(IpcErrorCode::InvalidFrame));
            }
            incoming.bytes[request.offset..request.offset + count].copy_from_slice(&data[..count]);
            incoming.written += count;
            incoming.until = Instant::now() + CURSOR_DEADLINE;
            if complete {
                cursor.submitting = true;
                cursor.incoming.take()
            } else {
                None
            }
        };
        if let Some(incoming) = ready {
            let owned = OwnedIpcHttpRequest {
                navigation,
                method: DesktopHttpMethod::Post,
                path: "/_webui/ipc".into(),
                token: session.token.clone(),
                body: incoming.bytes,
                input_permit: incoming.permit,
            };
            let result = bridge.submit(owned).await;
            state.borrow_mut().submitting = false;
            let response = result?;
            if response.status != 204 {
                return Err(error(response_error(&response)));
            }
        }
        let mut reply = DataReply::new(request);
        reply.next_offset = Some(request.offset + count);
        reply.complete = Some(complete);
        Ok(reply)
    }

    async fn receive(
        state: &Rc<RefCell<Self>>,
        bridge: &IpcBridge,
        navigation: u64,
        session: &SessionInfo,
        request: &DataRequest,
    ) -> Result<DataReply, IpcError> {
        if request.total_bytes.is_some()
            || request.data.is_some()
            || request
                .max_bytes
                .is_none_or(|max| max == 0 || max > CHUNK_BYTES)
        {
            return Err(error(IpcErrorCode::InvalidFrame));
        }
        {
            let mut cursor = state.borrow_mut();
            if cursor.receiving {
                return Err(error(IpcErrorCode::Overloaded));
            }
            cursor.receiving = true;
            if cursor
                .outgoing
                .as_ref()
                .is_some_and(|outgoing| Instant::now() >= outgoing.until)
            {
                cursor.outgoing = None;
            }
        }
        let result = Self::receive_inner(state, bridge, navigation, session, request).await;
        state.borrow_mut().receiving = false;
        result
    }

    async fn receive_inner(
        state: &Rc<RefCell<Self>>,
        bridge: &IpcBridge,
        navigation: u64,
        session: &SessionInfo,
        request: &DataRequest,
    ) -> Result<DataReply, IpcError> {
        if state.borrow().outgoing.is_none() {
            if request.offset != 0 {
                return Err(error(IpcErrorCode::InvalidFrame));
            }
            let input_permit = bridge.reserve_input(0)?;
            let response = bridge
                .submit(OwnedIpcHttpRequest {
                    navigation,
                    method: DesktopHttpMethod::Get,
                    path: "/_webui/ipc/outbound".into(),
                    token: session.token.clone(),
                    body: Vec::new(),
                    input_permit,
                })
                .await?;
            if response.status == 204 {
                let mut reply = DataReply::new(request);
                reply.empty = Some(true);
                return Ok(reply);
            }
            if response.status != 200 {
                return Err(error(response_error(&response)));
            }
            let bytes = response
                .body
                .into_bytes()
                .map_err(|_| error(IpcErrorCode::Transport))?;
            if bytes.is_empty() || bytes.len() > session.limits.max_frame_bytes {
                return Err(error(IpcErrorCode::InvalidFrame));
            }
            state.borrow_mut().outgoing = Some(Outgoing {
                bytes,
                offset: 0,
                until: Instant::now() + CURSOR_DEADLINE,
            });
        }
        let mut cursor = state.borrow_mut();
        let outgoing = cursor
            .outgoing
            .as_mut()
            .ok_or_else(|| error(IpcErrorCode::Closed))?;
        if outgoing.offset != request.offset {
            return Err(error(IpcErrorCode::InvalidFrame));
        }
        let max = request
            .max_bytes
            .ok_or_else(|| error(IpcErrorCode::InvalidFrame))?;
        let end = outgoing.offset + max.min(outgoing.bytes.len() - outgoing.offset);
        let mut reply = DataReply::new(request);
        reply.total_bytes = Some(outgoing.bytes.len());
        reply.offset = Some(outgoing.offset);
        reply.data = Some(STANDARD.encode(&outgoing.bytes.as_slice()[outgoing.offset..end]));
        reply.next_offset = Some(end);
        reply.complete = Some(end == outgoing.bytes.len());
        outgoing.offset = end;
        outgoing.until = Instant::now() + CURSOR_DEADLINE;
        if end == outgoing.bytes.len() {
            cursor.outgoing = None;
        }
        Ok(reply)
    }
}

fn response_error(response: &DesktopProtocolResponse) -> IpcErrorCode {
    response
        .body
        .as_bytes()
        .filter(|body| body.len() <= 2176)
        .and_then(|body| WireError::decode(body.as_slice()).ok())
        .map_or(IpcErrorCode::Transport, |wire| {
            IpcErrorCode::from_wire(&wire.code)
        })
}

fn recoverable_send_error(code: IpcErrorCode) -> bool {
    matches!(
        code,
        IpcErrorCode::Overloaded | IpcErrorCode::PayloadTooLarge | IpcErrorCode::InvalidPayload
    )
}
#[cold]
#[inline(never)]
fn error(code: IpcErrorCode) -> IpcError {
    IpcError::new(
        code,
        "native IPC data rejected",
        "reconnect the current trusted document",
    )
}

#[cfg(test)]
#[allow(clippy::disallowed_methods)]
mod tests {
    use super::*;
    use crate::ipc::{
        wire::{ipc_frame::Body, IpcFrame, Kind},
        Admission, CommittedMainDocument, Hello, IpcOptions, IpcWake, IPC_VERSION,
    };
    use crate::{DesktopApp, HostLifetime, LocalServerOptions, LoopbackOrigin};
    use futures_executor::block_on;
    use std::sync::Arc;

    struct Wake;
    impl IpcWake for Wake {
        fn wake(&self) -> Result<(), IpcError> {
            Ok(())
        }
    }

    #[test]
    fn idle_expiry_releases_input_credit_and_output_lease_without_a_new_message() {
        use std::sync::atomic::{AtomicBool, Ordering};
        struct DropFlag(Arc<AtomicBool>);
        impl Drop for DropFlag {
            fn drop(&mut self) {
                self.0.store(true, Ordering::Release);
            }
        }
        let ipc = crate::ipc::IpcWindowOwner::new(
            Arc::new(crate::ipc::IpcRegistry::default()),
            IpcOptions::default(),
            crate::ipc::IpcHost::Source {
                origin: "webui://app".into(),
            },
        )
        .unwrap();
        let bridge = ipc.bridge();
        let mut held = Vec::new();
        for _ in 0..7 {
            held.push(bridge.reserve_input(1_048_576).unwrap());
        }
        let until = Instant::now() + Duration::from_millis(10);
        let mut cursor = NativeData::default();
        cursor.seed_test_incoming(bridge.reserve_input(1_048_576).unwrap(), until);
        assert_eq!(
            bridge.reserve_input(4096).err().unwrap().code,
            IpcErrorCode::Overloaded
        );
        let dropped = Arc::new(AtomicBool::new(false));
        cursor.outgoing = Some(Outgoing {
            bytes: DesktopResponseBody::with_guard(vec![1; 1024], DropFlag(Arc::clone(&dropped))),
            offset: 0,
            until: until + Duration::from_millis(10),
        });
        assert_eq!(cursor.next_deadline(), Some(until));
        cursor.expire(until);
        assert!(cursor.incoming.is_none());
        assert!(!dropped.load(Ordering::Acquire));
        assert!(bridge.reserve_input(4096).is_ok());
        cursor.expire(until + Duration::from_millis(10));
        assert!(dropped.load(Ordering::Acquire));
        assert!(cursor.next_deadline().is_none());
        drop(held);
    }

    #[test]
    fn late_read_chunk_after_idle_deadline_cannot_reclaim_retired_output() {
        let session = SessionInfo {
            generation: 1,
            token: "a".repeat(32),
            limits: IpcOptions::default().limits,
        };
        let state = Rc::new(RefCell::new(NativeData::default()));
        let until = Instant::now() + CURSOR_DEADLINE;
        state
            .borrow_mut()
            .seed_test_outgoing(DesktopResponseBody::from(vec![3; CHUNK_BYTES + 1]), until);
        let bridge = IpcBridge {
            core: std::sync::Weak::new(),
        };
        let mut read = request(&session, "receive");
        read.max_bytes = Some(CHUNK_BYTES);
        let first = block_on(NativeData::exchange(
            &state,
            bridge.clone(),
            1,
            session.clone(),
            read,
        ));
        assert_eq!(first.next_offset, Some(CHUNK_BYTES));
        assert_eq!(first.complete, Some(false));
        let deadline = state.borrow().next_deadline().unwrap();
        state.borrow_mut().expire(deadline);
        let mut late = request(&session, "receive");
        late.offset = CHUNK_BYTES;
        late.max_bytes = Some(CHUNK_BYTES);
        let reply = block_on(NativeData::exchange(&state, bridge, 1, session, late));
        assert_eq!(
            reply.error.unwrap().code,
            IpcErrorCode::InvalidFrame.as_str()
        );
        assert!(state.borrow().outgoing.is_none());
    }

    #[test]
    fn every_browser_recoverable_send_error_discards_partial_ingress() {
        for code in [
            IpcErrorCode::Overloaded,
            IpcErrorCode::PayloadTooLarge,
            IpcErrorCode::InvalidPayload,
        ] {
            assert!(recoverable_send_error(code));
        }
        for code in [
            IpcErrorCode::InvalidFrame,
            IpcErrorCode::Closed,
            IpcErrorCode::Navigated,
        ] {
            assert!(!recoverable_send_error(code));
        }
    }

    fn request(session: &SessionInfo, operation: &str) -> DataRequest {
        DataRequest {
            kind: "ipcData".into(),
            version: 1,
            call_id: "1".into(),
            generation: session.generation.to_string(),
            token: session.token.clone(),
            operation: operation.into(),
            offset: 0,
            total_bytes: None,
            data: None,
            max_bytes: None,
        }
    }

    #[cfg(any(target_os = "macos", target_os = "windows"))]
    #[test]
    fn native_chunks_use_the_existing_v3_engine_not_the_http_listener() {
        let listener = crate::bind_owned_local_server("127.0.0.1:0".parse().unwrap()).unwrap();
        listener.set_nonblocking(true).unwrap();
        let origin = LoopbackOrigin::from_socket_addr(listener.local_addr().unwrap()).unwrap();
        let (owner, lifetime) = HostLifetime::new();
        let frame =
            DesktopApp::from_local_server(LocalServerOptions::new(origin.clone(), lifetime))
                .application_ipc(
                    &listener,
                    crate::ipc_test_support::registry(),
                    crate::ipc_test_support::options(),
                )
                .unwrap()
                .build()
                .unwrap();
        let bridge = frame.ipc_bridge().unwrap();
        bridge.attach_waker(Arc::new(Wake)).unwrap();
        bridge.navigate(1);
        let proof = bridge
            .begin_document(
                CommittedMainDocument {
                    navigation: 1,
                    origin: origin.as_str().into(),
                },
                [1; 16],
            )
            .unwrap();
        let session = block_on(bridge.admit(Admission {
            proof,
            hello: Hello {
                wire_version: IPC_VERSION,
                contract_name: "test.frame.echo".into(),
                contract_major: 1,
                schema_hash: "0123456789abcdef".repeat(4),
            },
        }))
        .unwrap();
        let data = Rc::new(RefCell::new(NativeData::default()));
        let frame_bytes = IpcFrame {
            version: IPC_VERSION,
            generation: session.generation,
            id: 1,
            kind: Kind::Request as i32,
            method_id: 1101,
            timeout_ms: 3000,
            body: Some(Body::Payload(vec![10, 4, b'p', b'i', b'n', b'g'])),
        }
        .encode_to_vec();
        let mut partial = request(&session, "send");
        partial.total_bytes = Some(CHUNK_BYTES + 1);
        partial.data = Some(STANDARD.encode(vec![1; CHUNK_BYTES]));
        let first = block_on(NativeData::exchange(
            &data,
            bridge.clone(),
            1,
            session.clone(),
            partial,
        ));
        assert_eq!(first.next_offset, Some(CHUNK_BYTES));
        let mut competing = request(&session, "send");
        competing.total_bytes = Some(frame_bytes.len());
        competing.data = Some(STANDARD.encode(&frame_bytes));
        let overloaded = block_on(NativeData::exchange(
            &data,
            bridge.clone(),
            1,
            session.clone(),
            competing,
        ));
        assert_eq!(
            overloaded.error.unwrap().code,
            IpcErrorCode::Overloaded.as_str()
        );
        assert!(
            data.borrow().incoming.is_none(),
            "overload must release the old partial permit"
        );
        let mut partial_again = request(&session, "send");
        partial_again.total_bytes = Some(CHUNK_BYTES + 1);
        partial_again.data = Some(STANDARD.encode(vec![1; CHUNK_BYTES]));
        assert_eq!(
            block_on(NativeData::exchange(
                &data,
                bridge.clone(),
                1,
                session.clone(),
                partial_again
            ))
            .next_offset,
            Some(CHUNK_BYTES)
        );
        let mut oversize = request(&session, "send");
        oversize.offset = CHUNK_BYTES;
        oversize.total_bytes = Some(CHUNK_BYTES + 1);
        oversize.data = Some("a".repeat(CHUNK_BASE64 + 4));
        let rejected = block_on(NativeData::exchange(
            &data,
            bridge.clone(),
            1,
            session.clone(),
            oversize,
        ));
        assert_eq!(
            rejected.error.unwrap().code,
            IpcErrorCode::PayloadTooLarge.as_str()
        );
        assert!(data.borrow().incoming.is_none());
        let mut timed_out = request(&session, "send");
        timed_out.total_bytes = Some(CHUNK_BYTES + 1);
        timed_out.data = Some(STANDARD.encode(vec![1; CHUNK_BYTES]));
        assert_eq!(
            block_on(NativeData::exchange(
                &data,
                bridge.clone(),
                1,
                session.clone(),
                timed_out
            ))
            .next_offset,
            Some(CHUNK_BYTES)
        );
        let deadline = data.borrow().next_deadline().unwrap();
        data.borrow_mut().expire(deadline);
        let mut late = request(&session, "send");
        late.offset = CHUNK_BYTES;
        late.total_bytes = Some(CHUNK_BYTES + 1);
        late.data = Some(STANDARD.encode([1]));
        let rejected = block_on(NativeData::exchange(
            &data,
            bridge.clone(),
            1,
            session.clone(),
            late,
        ));
        assert_eq!(
            rejected.error.unwrap().code,
            IpcErrorCode::InvalidFrame.as_str()
        );
        let mut input = request(&session, "send");
        input.total_bytes = Some(frame_bytes.len());
        input.data = Some(STANDARD.encode(&frame_bytes));
        let result = block_on(NativeData::exchange(
            &data,
            bridge.clone(),
            1,
            session.clone(),
            input,
        ));
        assert_eq!(result.next_offset, Some(frame_bytes.len()));
        assert_eq!(result.complete, Some(true));
        assert!(
            listener.accept().is_err(),
            "session credential or payload reached the HTTP listener"
        );
        let deadline = Instant::now() + Duration::from_secs(3);
        let reply = loop {
            let mut read = request(&session, "receive");
            read.max_bytes = Some(CHUNK_BYTES);
            let result = block_on(NativeData::exchange(
                &data,
                bridge.clone(),
                1,
                session.clone(),
                read,
            ));
            if result.empty != Some(true) {
                break result;
            }
            assert!(
                Instant::now() < deadline,
                "host IPC reply did not reach native cursor"
            );
            std::thread::sleep(Duration::from_millis(5));
        };
        let bytes = STANDARD.decode(reply.data.unwrap()).unwrap();
        let decoded = IpcFrame::decode(&bytes).unwrap();
        assert_eq!(decoded.kind, Kind::Result as i32);
        assert_eq!(
            decoded.body,
            Some(Body::Payload(vec![10, 4, b'p', b'i', b'n', b'g']))
        );
        assert!(listener.accept().is_err());
        owner.revoke().unwrap();
        let mut after = request(&session, "send");
        after.total_bytes = Some(frame_bytes.len());
        after.data = Some(STANDARD.encode(frame_bytes));
        let denied = block_on(NativeData::exchange(&data, bridge, 1, session, after));
        assert_eq!(denied.error.unwrap().code, IpcErrorCode::Closed.as_str());
    }

    #[test]
    fn malformed_or_overlarge_chunks_are_rejected_before_frame_allocation() {
        let session = SessionInfo {
            generation: 1,
            token: "a".repeat(32),
            limits: IpcOptions::default().limits,
        };
        let mut request = request(&session, "send");
        request.total_bytes = Some(3);
        request.data = Some("YWJj".into());
        assert!(request.authenticated(&session).is_ok());
        request.token.replace_range(..1, "b");
        assert_eq!(
            request.authenticated(&session).unwrap_err().code,
            IpcErrorCode::PermissionDenied
        );
        request.token = session.token.clone();
        request.data = Some("a".repeat(CHUNK_BASE64 + 4));
        let state = Rc::new(RefCell::new(NativeData::default()));
        let bridge = IpcBridge {
            core: std::sync::Weak::new(),
        };
        let denied = block_on(NativeData::exchange(&state, bridge, 1, session, request));
        assert_eq!(
            denied.error.unwrap().code,
            IpcErrorCode::PayloadTooLarge.as_str()
        );
        assert!(state.borrow().incoming.is_none());
    }

    #[test]
    fn native_data_preserves_stable_core_error_instead_of_guessing_from_status() {
        for code in [
            IpcErrorCode::NotReady,
            IpcErrorCode::Navigated,
            IpcErrorCode::Closed,
            IpcErrorCode::Transport,
        ] {
            let response = DesktopProtocolResponse::new(
                if matches!(code, IpcErrorCode::NotReady | IpcErrorCode::Navigated) {
                    409
                } else {
                    503
                },
                "application/x-protobuf",
                WireError {
                    code: code.as_str().into(),
                    message: String::new(),
                    help: String::new(),
                    application_code: String::new(),
                }
                .encode_to_vec(),
            );
            assert_eq!(response_error(&response), code);
        }
    }
}

// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

#![allow(clippy::disallowed_methods)]
#![cfg(feature = "application-ipc")]

use futures_executor::block_on;
use futures_util::FutureExt;
use std::{
    sync::{
        atomic::{AtomicUsize, Ordering},
        mpsc, Arc, Mutex,
    },
    time::{Duration, Instant},
};
use webui_desktop::{
    ipc::{
        wire::{ipc_frame::Body, IpcFrame, Kind, WireError},
        *,
    },
    DesktopHttpMethod,
};

#[derive(Clone, Debug, Default, PartialEq)]
struct Item {
    id: u64,
    bytes: Vec<u8>,
}
impl IpcCodec for Item {
    fn encode_ipc(&self) -> Vec<u8> {
        let mut writer = PayloadWriter::with_capacity(self.bytes.len() + 16);
        if self.id != 0 {
            writer.uint64(1, self.id);
        }
        if !self.bytes.is_empty() {
            writer.bytes(2, &self.bytes);
        }
        writer.finish()
    }

    fn decode_ipc(bytes: &[u8]) -> Result<Self, IpcError> {
        let mut item = Self::default();
        let mut reader = WireReader::new(bytes);
        while let Some(field) = reader.next_field()? {
            match field.number {
                1 => item.id = field.uint64()?,
                2 => item.bytes = field.bytes()?.to_vec(),
                _ => {}
            }
        }
        Ok(item)
    }
}
static SHAPES: &[MessageShape] = &[
    MessageShape {
        fields: &[
            FieldShape {
                number: 1,
                kind: FieldKind::Uint64,
                repeated: false,
                packed: false,
                oneof: None,
                map_key: false,
            },
            FieldShape {
                number: 2,
                kind: FieldKind::Bytes,
                repeated: false,
                packed: false,
                oneof: None,
                map_key: false,
            },
        ],
    },
    MessageShape { fields: &[] },
];
fn item_guard(bytes: &[u8], limits: &IpcLimits) -> Result<(), IpcError> {
    validate_message(bytes, 0, SHAPES, limits)
}
fn empty_guard(bytes: &[u8], limits: &IpcLimits) -> Result<(), IpcError> {
    validate_message(bytes, 1, SHAPES, limits)
}
struct ValidationGate {
    started: mpsc::Sender<()>,
    release: mpsc::Receiver<()>,
}
static OUTBOUND_VALIDATION_GATE: Mutex<Option<ValidationGate>> = Mutex::new(None);
fn sequencing_guard(bytes: &[u8], limits: &IpcLimits) -> Result<(), IpcError> {
    item_guard(bytes, limits)?;
    let item = Item::decode_ipc(bytes).unwrap();
    if item.id == 9999 {
        let gate = OUTBOUND_VALIDATION_GATE.lock().unwrap().take().unwrap();
        gate.started.send(()).unwrap();
        let _ = gate.release.recv_timeout(Duration::from_secs(2));
    }
    Ok(())
}
static SCHEMA: IpcSchema = IpcSchema {
    name: "core.tests",
    major: 1,
    hash: "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
    methods: &[
        MethodDescriptor {
            id: 1101,
            name: "Host.Save",
            receiver: Endpoint::Host,
            kind: MethodKind::Rpc,
            development_only: false,
            validate_request: item_guard,
            validate_response: Some(empty_guard),
        },
        MethodDescriptor {
            id: 1102,
            name: "Host.Selected",
            receiver: Endpoint::Host,
            kind: MethodKind::Notification,
            development_only: false,
            validate_request: item_guard,
            validate_response: None,
        },
        MethodDescriptor {
            id: 1103,
            name: "Host.Debug",
            receiver: Endpoint::Host,
            kind: MethodKind::Rpc,
            development_only: true,
            validate_request: item_guard,
            validate_response: Some(empty_guard),
        },
        MethodDescriptor {
            id: 1104,
            name: "Host.Echo",
            receiver: Endpoint::Host,
            kind: MethodKind::Rpc,
            development_only: false,
            validate_request: item_guard,
            validate_response: Some(item_guard),
        },
        MethodDescriptor {
            id: 2001,
            name: "Renderer.Label",
            receiver: Endpoint::Renderer,
            kind: MethodKind::Rpc,
            development_only: false,
            validate_request: sequencing_guard,
            validate_response: Some(item_guard),
        },
        MethodDescriptor {
            id: 2002,
            name: "Renderer.Changed",
            receiver: Endpoint::Renderer,
            kind: MethodKind::Notification,
            development_only: false,
            validate_request: item_guard,
            validate_response: None,
        },
    ],
};
struct Save;
impl Rpc for Save {
    type Request = Item;
    type Response = ();
    type Receiver = Host;
    const ID: u32 = 1101;
}
struct DebugCall;
impl Rpc for DebugCall {
    type Request = Item;
    type Response = ();
    type Receiver = Host;
    const ID: u32 = 1103;
}
struct Label;
impl Rpc for Label {
    type Request = Item;
    type Response = Item;
    type Receiver = Renderer;
    const ID: u32 = 2001;
}
struct Echo;
impl Rpc for Echo {
    type Request = Item;
    type Response = Item;
    type Receiver = Host;
    const ID: u32 = 1104;
}
struct Selected;
impl Event for Selected {
    type Payload = Item;
    type Receiver = Host;
    const ID: u32 = 1102;
}
struct Changed;
impl Event for Changed {
    type Payload = Item;
    type Receiver = Renderer;
    const ID: u32 = 2002;
}

#[test]
fn disabled_registry_is_deny_all_and_starts_no_resources() {
    let owner = IpcWindowOwner::new(
        Arc::new(IpcRegistry::default()),
        IpcOptions::default(),
        IpcHost::Packaged {
            origin: "webui://app".into(),
        },
    )
    .unwrap();
    let window = owner.window();
    assert_eq!(
        window.current_session().err().unwrap().code,
        IpcErrorCode::NotReady
    );
    assert_eq!(
        block_on(window.ready()).err().unwrap().code,
        IpcErrorCode::NotReady
    );
    assert!(!window.stats().workers_started);
    assert!(!window.stats().timer_started);
    let bridge = owner.bridge();
    assert!(!bridge.is_enabled());
    drop(owner);
    assert!(!bridge.is_enabled());
    assert_eq!(
        window.current_session().err().unwrap().code,
        IpcErrorCode::Closed
    );
    assert!(bridge.take_control().is_err());
}

#[test]
fn registry_registration_is_transactional_and_never_overwrites() {
    let mut registry = IpcRegistry::new(&SCHEMA);
    registry
        .register::<Save, _, _>(|_, _| async { Ok(()) })
        .unwrap();
    assert_eq!(
        registry
            .register::<Save, _, _>(|_, _| async { Ok(()) })
            .unwrap_err()
            .code,
        IpcErrorCode::InvalidPayload
    );
    assert!(registry.validate_registration(&[1103, 1101]).is_err());
    registry
        .register::<DebugCall, _, _>(|_, _| async { Ok(()) })
        .unwrap();
    assert!(registry.validate_registration(&[2001]).is_err());
    registry
        .register_notification::<Selected, _, _>(|_, _| async { Ok(()) })
        .unwrap();
    assert_eq!(
        registry
            .register_notification::<Selected, _, _>(|_, _| async { Ok(()) })
            .unwrap_err()
            .code,
        IpcErrorCode::InvalidPayload
    );
    assert!(registry.validate_registration(&[1101, 1102]).is_err());
}

#[test]
fn malformed_envelopes_and_wire_versions_fail_closed() {
    let limits = IpcLimits::default();
    for bytes in [
        // Too short to even hold the fixed 30-byte header.
        vec![0x80],
        Vec::new(),
        vec![0u8; 29],
        // Well-formed 30-byte header (accept, no body) but an invalid body tag.
        {
            let mut bytes = IpcFrame {
                version: IPC_VERSION,
                generation: 1,
                id: 1,
                kind: Kind::Accept as i32,
                method_id: 0,
                timeout_ms: 0,
                body: None,
            }
            .encode_to_vec();
            *bytes.last_mut().unwrap() = 9;
            bytes
        },
        // Payload length prefix claiming more bytes than actually remain.
        {
            let mut bytes = IpcFrame {
                version: IPC_VERSION,
                generation: 1,
                id: 1,
                kind: Kind::Request as i32,
                method_id: 1101,
                timeout_ms: 1000,
                body: Some(Body::Payload(vec![1, 2, 3])),
            }
            .encode_to_vec();
            let len_offset = bytes.len() - 3 - 4;
            bytes[len_offset..len_offset + 4].copy_from_slice(&0xffff_ffffu32.to_le_bytes());
            bytes
        },
    ] {
        assert!(decode_frame(&bytes, &limits).is_err());
    }
    let mut frame = IpcFrame {
        version: IPC_VERSION - 1,
        generation: 1,
        id: 1,
        kind: Kind::Request as i32,
        method_id: 1101,
        timeout_ms: 1000,
        body: Some(Body::Payload(Vec::new())),
    };
    assert_eq!(
        decode_frame(&frame.encode_to_vec(), &limits)
            .unwrap_err()
            .code,
        IpcErrorCode::UnsupportedVersion
    );
    frame.version = IPC_VERSION;
    frame.kind = Kind::Accept as i32;
    assert_eq!(
        decode_frame(&frame.encode_to_vec(), &limits)
            .unwrap_err()
            .code,
        IpcErrorCode::InvalidFrame
    );
    frame.kind = Kind::Request as i32;
    // Trailing garbage appended after an otherwise well-formed frame must be
    // rejected: the fixed layout has no tolerance for extra bytes.
    let mut trailing = frame.encode_to_vec();
    trailing.extend([8, 2]);
    assert_eq!(
        decode_frame(&trailing, &limits).unwrap_err().code,
        IpcErrorCode::InvalidFrame
    );
    assert_eq!(
        decode_frame(&vec![0; limits.max_frame_bytes + 1], &limits)
            .unwrap_err()
            .code,
        IpcErrorCode::PayloadTooLarge
    );
}

#[test]
fn generated_validation_checks_collections_oneofs_map_keys_and_numeric_ranges() {
    static SHAPES: &[MessageShape] = &[MessageShape {
        fields: &[
            FieldShape {
                number: 1,
                kind: FieldKind::Uint32,
                repeated: true,
                packed: true,
                oneof: None,
                map_key: false,
            },
            FieldShape {
                number: 2,
                kind: FieldKind::String,
                repeated: false,
                packed: false,
                oneof: Some(0),
                map_key: true,
            },
            FieldShape {
                number: 3,
                kind: FieldKind::String,
                repeated: false,
                packed: false,
                oneof: Some(0),
                map_key: false,
            },
        ],
    }];
    let mut limits = IpcLimits::default();
    assert!(validate_message(&[8, 0xff, 0xff, 0xff, 0xff, 0x1f], 0, SHAPES, &limits).is_err());
    assert!(validate_message(b"\x12\x09__proto__", 0, SHAPES, &limits).is_ok());
    assert!(validate_message(b"\x12\x0bconstructor", 0, SHAPES, &limits).is_ok());
    assert!(validate_message(b"\x12\x09prototype", 0, SHAPES, &limits).is_ok());
    assert!(validate_message(b"\x12\x01a\x1a\x01b", 0, SHAPES, &limits).is_err());
    limits.max_collection_entries_per_message = 3;
    assert!(validate_message(&[10, 4, 1, 2, 3, 4], 0, SHAPES, &limits).is_err());
    assert!(validate_message(&[10, 2, 1, 2], 0, SHAPES, &limits).is_ok());
    assert!(validate_message(&[10, 3, 1, 2, 3], 0, SHAPES, &limits).is_ok());
    let item = Item {
        id: u64::MAX,
        bytes: vec![0xaa; 256 * 1024],
    };
    item_guard(&item.encode_ipc(), &IpcLimits::default()).unwrap();
    assert_eq!(
        Item::decode_ipc(item.encode_ipc().as_slice()).unwrap(),
        item
    );
}

struct Wake(mpsc::Sender<()>);
impl IpcWake for Wake {
    fn wake(&self) -> Result<(), IpcError> {
        self.0.send(()).map_err(|_| {
            IpcError::new(
                IpcErrorCode::Transport,
                "test wake channel closed",
                "retain the test peer",
            )
        })
    }
}
struct Peer {
    owner: IpcWindowOwner,
    bridge: IpcBridge,
    info: SessionInfo,
    navigation: u64,
    wakes: mpsc::Receiver<()>,
}
#[test]
fn removed_transfer_paths_reject_without_disrupting_typed_ipc() {
    let mut registry = IpcRegistry::new(&SCHEMA);
    registry
        .register::<Echo, _, _>(|_, item| async move { Ok(item) })
        .unwrap();
    let peer = Peer::new(registry, IpcOptions::for_schema(&SCHEMA));
    for path in [
        "/_webui/ipc/raw/1/00000000000000000000000000000000/1/0",
        "/_webui/ipc/raw/1/00000000000000000000000000000000/1/cancel",
    ] {
        assert_eq!(
            peer.bridge.max_request_body_bytes(path).unwrap(),
            peer.info.limits.max_frame_bytes
        );
        for method in [DesktopHttpMethod::Get, DesktopHttpMethod::Post] {
            let response = peer.submit(method, path, Vec::new());
            assert_eq!(response.status, 400);
            let error = WireError::decode(response.body.as_bytes().unwrap().as_slice()).unwrap();
            assert_eq!(error.code, "invalid-frame");
        }
    }
    let item = Item {
        id: 42,
        bytes: vec![1, 2, 3],
    };
    assert_eq!(
        peer.post(peer.invocation(1, Echo::ID, Kind::Request, item.clone())),
        204
    );
    let response = peer.next_frame();
    assert_eq!(response.kind, Kind::Result as i32);
    assert_eq!(response.body, Some(Body::Payload(item.encode_ipc())));
}

fn hello() -> Hello {
    Hello {
        wire_version: IPC_VERSION,
        contract_name: SCHEMA.name.into(),
        contract_major: SCHEMA.major,
        schema_hash: SCHEMA.hash.into(),
    }
}
fn identity(navigation: u64) -> CommittedMainDocument {
    CommittedMainDocument {
        navigation,
        origin: "webui://app".into(),
    }
}
impl Peer {
    fn new(registry: IpcRegistry, options: IpcOptions) -> Self {
        Self::with_host(
            registry,
            options,
            IpcHost::Source {
                origin: "webui://app".into(),
            },
        )
    }
    fn with_host(registry: IpcRegistry, options: IpcOptions, host: IpcHost) -> Self {
        let owner = IpcWindowOwner::new(Arc::new(registry), options, host).unwrap();
        let bridge = owner.bridge();
        assert!(bridge.is_enabled());
        let (sender, wakes) = mpsc::channel();
        bridge.attach_waker(Arc::new(Wake(sender))).unwrap();
        bridge.navigate(1);
        let proof = bridge.begin_document(identity(1), [1; 16]).unwrap();
        assert!(!owner.window().stats().workers_started);
        assert!(!owner.window().stats().timer_started);
        let info = block_on(bridge.admit(Admission {
            hello: hello(),
            proof,
        }))
        .unwrap();
        Self {
            owner,
            bridge,
            info,
            navigation: 1,
            wakes,
        }
    }
    fn session(&self) -> IpcSession {
        self.owner.window().current_session().unwrap()
    }
    fn navigate(&mut self) {
        self.navigation += 1;
        self.bridge.navigate(self.navigation);
        let proof = self
            .bridge
            .begin_document(identity(self.navigation), [2; 16])
            .unwrap();
        self.info = block_on(self.bridge.admit(Admission {
            hello: hello(),
            proof,
        }))
        .unwrap();
    }
    fn submit(
        &self,
        method: DesktopHttpMethod,
        path: &str,
        body: Vec<u8>,
    ) -> webui_desktop::DesktopProtocolResponse {
        let permit = self.bridge.reserve_input(body.capacity()).unwrap();
        block_on(self.bridge.submit(OwnedIpcHttpRequest {
            navigation: self.navigation,
            method,
            path: path.into(),
            token: self.info.token.clone(),
            body,
            input_permit: permit,
        }))
        .unwrap()
    }
    fn post(&self, frame: IpcFrame) -> u16 {
        self.submit(
            DesktopHttpMethod::Post,
            "/_webui/ipc",
            frame.encode_to_vec(),
        )
        .status
    }
    fn invocation(&self, id: u64, method: u32, kind: Kind, item: Item) -> IpcFrame {
        IpcFrame {
            version: IPC_VERSION,
            generation: self.info.generation,
            id,
            kind: kind as i32,
            method_id: method,
            timeout_ms: if kind == Kind::Request { 30_000 } else { 0 },
            body: Some(Body::Payload(item.encode_ipc())),
        }
    }
    fn response(&self, id: u64, kind: Kind, body: Option<Body>) -> IpcFrame {
        IpcFrame {
            version: IPC_VERSION,
            generation: self.info.generation,
            id,
            kind: kind as i32,
            method_id: 0,
            timeout_ms: 0,
            body,
        }
    }
    fn next_frame(&self) -> IpcFrame {
        let deadline = Instant::now() + Duration::from_secs(3);
        loop {
            let response = self.submit(DesktopHttpMethod::Get, "/_webui/ipc/outbound", Vec::new());
            if response.status == 200 {
                return IpcFrame::decode(response.body.as_bytes().unwrap().as_slice()).unwrap();
            }
            assert_eq!(response.status, 204);
            let mut ready = false;
            while let Some(control) = self.bridge.take_control().unwrap() {
                ready |= matches!(control, NativeControl::Ready { generation } if generation == self.info.generation);
            }
            // Ready may have coalesced with a previously consumed wake between
            // the final GET and control drain. Process it rather than waiting
            // for a second wake which the transport correctly does not emit.
            if ready {
                assert!(Instant::now() < deadline);
                continue;
            }
            self.wakes
                .recv_timeout(deadline.saturating_duration_since(Instant::now()))
                .unwrap();
        }
    }
}

#[test]
fn startup_notifications_are_installed_before_ready_and_reinstalled_per_document() {
    let native_thread = std::thread::current().id();
    let (sent, received) = mpsc::channel();
    let mut registry = IpcRegistry::new(&SCHEMA);
    registry
        .register_notification::<Selected, _, _>(move |ctx, item| {
            let sent = sent.clone();
            async move {
                assert_ne!(std::thread::current().id(), native_thread);
                sent.send((ctx.session.generation(), item.id)).unwrap();
                Ok(())
            }
        })
        .unwrap();
    let mut options = IpcOptions::for_schema(&SCHEMA);
    options.limits.max_callbacks_per_event = 1;
    let mut peer = Peer::new(registry, options);
    let old = peer.session();
    assert_eq!(
        old.subscribe::<Selected, _, _>(|_, _| async { Ok(()) })
            .err()
            .unwrap()
            .code,
        IpcErrorCode::Overloaded
    );
    assert_eq!(
        peer.post(peer.invocation(
            1,
            1102,
            Kind::Notify,
            Item {
                id: 7,
                bytes: Vec::new()
            }
        )),
        204
    );
    assert_eq!(peer.next_frame().kind, Kind::Accept as i32);
    assert_eq!(
        received.recv_timeout(Duration::from_secs(2)).unwrap(),
        (old.generation(), 7)
    );
    peer.navigate();
    assert!(old.is_closed());
    let current = peer.session();
    assert_ne!(current.generation(), old.generation());
    assert_eq!(
        peer.post(peer.invocation(
            1,
            1102,
            Kind::Notify,
            Item {
                id: 9,
                bytes: Vec::new()
            }
        )),
        204
    );
    assert_eq!(peer.next_frame().kind, Kind::Accept as i32);
    assert_eq!(
        received.recv_timeout(Duration::from_secs(2)).unwrap(),
        (current.generation(), 9)
    );
}

#[test]
fn startup_and_dynamic_callbacks_share_limits_and_dynamic_disposal_keeps_startup() {
    let count = Arc::new(AtomicUsize::new(0));
    let count_handler = Arc::clone(&count);
    let (sent, received) = mpsc::channel();
    let mut registry = IpcRegistry::new(&SCHEMA);
    registry
        .register_notification::<Selected, _, _>(move |_, _| {
            count_handler.fetch_add(1, Ordering::SeqCst);
            let sent = sent.clone();
            async move {
                sent.send(()).unwrap();
                Ok(())
            }
        })
        .unwrap();
    let mut options = IpcOptions::for_schema(&SCHEMA);
    options.limits.max_callbacks_per_event = 2;
    options.limits.max_callbacks_per_document = 2;
    let peer = Peer::new(registry, options);
    let dynamic_count = Arc::new(AtomicUsize::new(0));
    let dynamic_handler = Arc::clone(&dynamic_count);
    let mut guard = peer
        .session()
        .subscribe::<Selected, _, _>(move |_, _| {
            dynamic_handler.fetch_add(1, Ordering::SeqCst);
            async { Ok(()) }
        })
        .unwrap();
    assert!(peer
        .session()
        .subscribe::<Selected, _, _>(|_, _| async { Ok(()) })
        .is_err());
    guard.close();
    guard.close();
    assert_eq!(
        peer.post(peer.invocation(1, 1102, Kind::Notify, Item::default())),
        204
    );
    assert_eq!(peer.next_frame().kind, Kind::Accept as i32);
    received.recv_timeout(Duration::from_secs(2)).unwrap();
    assert_eq!(count.load(Ordering::SeqCst), 1);
    assert_eq!(dynamic_count.load(Ordering::SeqCst), 0);
}

#[test]
fn acknowledged_void_rpc_waits_for_handler_while_notification_waits_only_for_acceptance() {
    let (release, wait) = futures_channel::oneshot::channel::<()>();
    let wait = Mutex::new(Some(wait));
    let (started, observed) = mpsc::channel();
    let mut registry = IpcRegistry::new(&SCHEMA);
    registry
        .register::<Save, _, _>(move |_, _| {
            let wait = wait.lock().unwrap().take().unwrap();
            started.send(()).unwrap();
            async move {
                wait.await.unwrap();
                Ok(())
            }
        })
        .unwrap();
    let peer = Peer::new(registry, IpcOptions::for_schema(&SCHEMA));
    assert_eq!(
        peer.post(peer.invocation(1, 1101, Kind::Request, Item::default())),
        204
    );
    observed.recv_timeout(Duration::from_secs(2)).unwrap();
    assert_eq!(
        peer.submit(DesktopHttpMethod::Get, "/_webui/ipc/outbound", Vec::new())
            .status,
        204
    );
    release.send(()).unwrap();
    let response = peer.next_frame();
    assert_eq!(response.id, 1);
    assert_eq!(response.kind, Kind::Result as i32);
    assert_eq!(response.body, Some(Body::Payload(Vec::new())));

    let payload = Item {
        id: u64::MAX,
        bytes: vec![0xa5; 256 * 1024],
    };
    let mut notification = peer.session().notify::<Changed>(payload.clone());
    assert!(notification.as_mut().now_or_never().is_none());
    let outgoing = peer.next_frame();
    assert_eq!(outgoing.kind, Kind::Notify as i32);
    let Some(Body::Payload(bytes)) = outgoing.body else {
        panic!("missing notification bytes");
    };
    assert_eq!(Item::decode_ipc(bytes.as_slice()).unwrap(), payload);
    assert_eq!(
        peer.post(peer.response(outgoing.id, Kind::Accept, None)),
        204
    );
    block_on(notification).unwrap();
}

#[test]
fn renderer_rpc_decodes_on_worker_and_unknown_error_codes_map_to_transport() {
    let peer = Peer::new(IpcRegistry::new(&SCHEMA), IpcOptions::for_schema(&SCHEMA));
    let item = Item {
        id: u64::MAX,
        bytes: vec![7; 256 * 1024],
    };
    let call = peer
        .session()
        .call::<Label>(item.clone(), CallOptions::default());
    let request = peer.next_frame();
    assert_eq!(request.id, 1);
    assert_eq!(request.method_id, 2001);
    assert_eq!(
        peer.post(peer.response(
            request.id,
            Kind::Result,
            Some(Body::Payload(item.encode_ipc()))
        )),
        204
    );
    assert_eq!(block_on(call).unwrap(), item);
    // Completion IDs and peer invocation IDs are independent.
    assert_eq!(
        peer.post(peer.invocation(1, 1101, Kind::Request, Item::default())),
        204
    );
    let missing = peer.next_frame();
    let Some(Body::Error(error)) = missing.body else {
        panic!("missing error");
    };
    assert_eq!(error.code, "receiver-unavailable");

    let call = peer
        .session()
        .call::<Label>(Item::default(), CallOptions::default());
    let request = peer.next_frame();
    let error = WireError {
        code: "unknown-peer-code".into(),
        message: "bounded".into(),
        help: "check peer".into(),
        application_code: String::new(),
    };
    assert_eq!(
        peer.post(peer.response(request.id, Kind::Error, Some(Body::Error(error)))),
        204
    );
    assert_eq!(block_on(call).unwrap_err().code, IpcErrorCode::Transport);
}

fn eventually(mut condition: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(3);
    while !condition() {
        assert!(Instant::now() < deadline, "condition did not become true");
        std::thread::sleep(Duration::from_millis(1));
    }
}

#[test]
fn post_commit_proof_rejects_old_same_url_hello_without_consuming_new_activation() {
    let owner = IpcWindowOwner::new(
        Arc::new(IpcRegistry::new(&SCHEMA)),
        IpcOptions::for_schema(&SCHEMA),
        IpcHost::Source {
            origin: "webui://app".into(),
        },
    )
    .unwrap();
    let bridge = owner.bridge();
    bridge.navigate(1);
    let old = bridge.begin_document(identity(1), [1; 16]).unwrap();
    assert!(bridge.begin_document(identity(1), [1; 16]).is_err());
    bridge.navigate(2);
    let mut external = identity(2);
    external.origin = "https://untrusted.invalid".into();
    assert_eq!(
        bridge.begin_document(external, [2; 16]).err().unwrap().code,
        IpcErrorCode::PermissionDenied
    );
    let current = bridge.begin_document(identity(2), [2; 16]).unwrap();
    assert!(!owner.window().stats().workers_started);
    let stale = block_on(bridge.admit(Admission {
        hello: hello(),
        proof: old,
    }))
    .err()
    .unwrap();
    assert_eq!(stale.code, IpcErrorCode::PermissionDenied);
    let mut mismatch = hello();
    mismatch.schema_hash = "0".repeat(64);
    assert_eq!(
        block_on(bridge.admit(Admission {
            hello: mismatch,
            proof: current.clone()
        }))
        .err()
        .unwrap()
        .code,
        IpcErrorCode::SchemaMismatch
    );
    let mut forged = current.clone();
    forged.document_nonce[0] ^= 1;
    assert_eq!(
        block_on(bridge.admit(Admission {
            hello: hello(),
            proof: forged
        }))
        .err()
        .unwrap()
        .code,
        IpcErrorCode::PermissionDenied
    );
    let info = block_on(bridge.admit(Admission {
        hello: hello(),
        proof: current.clone(),
    }))
    .unwrap();
    assert_eq!(info.token.len(), 32);
    assert!(info
        .token
        .bytes()
        .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)));
    assert_eq!(
        serde_json::to_value(&info).unwrap()["generation"],
        info.generation.to_string()
    );
    assert_eq!(
        block_on(bridge.admit(Admission {
            hello: hello(),
            proof: current
        }))
        .err()
        .unwrap()
        .code,
        IpcErrorCode::NotReady
    );
    bridge
        .disconnect_authenticated(info.generation, &info.token)
        .unwrap();
    assert_eq!(
        bridge
            .disconnect_authenticated(info.generation, &info.token)
            .unwrap_err()
            .code,
        IpcErrorCode::Closed
    );
    assert!(bridge.begin_document(identity(2), [3; 16]).is_err());
}

#[test]
fn readiness_is_bounded_cancelled_waiters_free_slots_and_close_settles_waiters() {
    let owner = IpcWindowOwner::new(
        Arc::new(IpcRegistry::new(&SCHEMA)),
        IpcOptions::for_schema(&SCHEMA),
        IpcHost::Source {
            origin: "webui://app".into(),
        },
    )
    .unwrap();
    let mut waiters: Vec<_> = (0..16).map(|_| owner.window().ready()).collect();
    assert_eq!(
        block_on(owner.window().ready()).err().unwrap().code,
        IpcErrorCode::Overloaded
    );
    waiters.pop();
    let replacement = owner.window().ready();
    owner.close();
    assert_eq!(
        block_on(replacement).err().unwrap().code,
        IpcErrorCode::Closed
    );
    for waiter in waiters {
        assert_eq!(block_on(waiter).err().unwrap().code, IpcErrorCode::Closed);
    }
    assert!(!owner.window().stats().workers_started);
    assert!(!owner.window().stats().timer_started);
}

#[test]
fn nested_rpc_reenters_single_worker_and_opposite_direction_ids_do_not_collide() {
    let mut registry = IpcRegistry::new(&SCHEMA);
    registry
        .register::<Save, _, _>(|ctx, item| async move {
            if item.id == 1 {
                let reply = ctx
                    .session
                    .call::<Label>(
                        Item {
                            id: 2,
                            bytes: Vec::new(),
                        },
                        CallOptions::default(),
                    )
                    .await?;
                assert_eq!(reply.id, 3);
            }
            Ok(())
        })
        .unwrap();
    let peer = Peer::new(registry, IpcOptions::for_schema(&SCHEMA));
    assert_eq!(
        peer.post(peer.invocation(
            1,
            1101,
            Kind::Request,
            Item {
                id: 1,
                bytes: Vec::new()
            }
        )),
        204
    );
    let nested = peer.next_frame();
    assert_eq!((nested.id, nested.kind), (1, Kind::Request as i32));
    assert_eq!(
        peer.post(peer.invocation(
            2,
            1101,
            Kind::Request,
            Item {
                id: 2,
                bytes: Vec::new()
            }
        )),
        204
    );
    let inner = peer.next_frame();
    assert_eq!((inner.id, inner.kind), (2, Kind::Result as i32));
    assert_eq!(
        peer.post(
            peer.response(
                nested.id,
                Kind::Result,
                Some(Body::Payload(
                    Item {
                        id: 3,
                        bytes: Vec::new()
                    }
                    .encode_ipc()
                ))
            )
        ),
        204
    );
    let outer = peer.next_frame();
    assert_eq!((outer.id, outer.kind), (1, Kind::Result as i32));
}

#[test]
fn cancellation_drop_late_replies_and_navigation_settle_document_scoped_calls() {
    let mut peer = Peer::new(IpcRegistry::new(&SCHEMA), IpcOptions::for_schema(&SCHEMA));
    let old_session = peer.session();
    let call = old_session.call::<Label>(Item::default(), CallOptions::default());
    let request = peer.next_frame();
    call.cancel();
    assert_eq!(block_on(call).unwrap_err().code, IpcErrorCode::Cancelled);
    assert_eq!(peer.next_frame().kind, Kind::Cancel as i32);
    assert_eq!(
        peer.post(peer.response(request.id, Kind::Result, Some(Body::Payload(Vec::new())))),
        204
    );
    assert_eq!(peer.owner.window().stats().stale_replies, 1);
    let dropped = old_session.call::<Label>(Item::default(), CallOptions::default());
    peer.next_frame();
    drop(dropped);
    assert_eq!(peer.next_frame().kind, Kind::Cancel as i32);
    let navigated = old_session.call::<Label>(Item::default(), CallOptions::default());
    peer.next_frame();
    let old_token = peer.info.token.clone();
    let old_generation = peer.info.generation;
    peer.navigate();
    assert_eq!(
        block_on(navigated).unwrap_err().code,
        IpcErrorCode::Navigated
    );
    assert!(old_session.is_closed());
    assert_eq!(
        block_on(old_session.call::<Label>(Item::default(), CallOptions::default()))
            .unwrap_err()
            .code,
        IpcErrorCode::Navigated
    );
    let response = block_on(peer.bridge.submit(OwnedIpcHttpRequest {
        navigation: peer.navigation,
        method: DesktopHttpMethod::Get,
        path: "/_webui/ipc/outbound".into(),
        token: old_token,
        body: Vec::new(),
        input_permit: peer.bridge.reserve_input(0).unwrap(),
    }))
    .unwrap();
    assert_eq!(response.status, 401);
    let mut stale = peer.response(1, Kind::Result, Some(Body::Payload(Vec::new())));
    stale.generation = old_generation;
    assert_eq!(peer.post(stale), 409);
    let closed = peer
        .session()
        .call::<Label>(Item::default(), CallOptions::default());
    peer.bridge.close();
    assert_eq!(block_on(closed).unwrap_err().code, IpcErrorCode::Closed);
    assert!(!peer.bridge.is_enabled());
}

#[test]
fn overload_rejects_immediately_but_completion_slots_survive_full_data_queue() {
    let mut registry = IpcRegistry::new(&SCHEMA);
    registry
        .register::<Save, _, _>(|_, _| async { Ok(()) })
        .unwrap();
    let mut options = IpcOptions::for_schema(&SCHEMA);
    options.limits.max_queued_frames_per_direction = 1;
    let peer = Peer::new(registry, options);
    let call = peer
        .session()
        .call::<Label>(Item::default(), CallOptions::default());
    eventually(|| peer.owner.window().stats().queued_bytes > 0);
    let rejected = peer
        .session()
        .call::<Label>(Item::default(), CallOptions::default());
    assert_eq!(
        block_on(rejected).unwrap_err().code,
        IpcErrorCode::Overloaded
    );
    assert_eq!(
        peer.post(peer.invocation(1, 1101, Kind::Request, Item::default())),
        204
    );
    // A data frame remains queued, but the remote call owns a completion slot.
    let outgoing = peer.next_frame();
    assert_eq!(outgoing.kind, Kind::Request as i32);
    let completed = peer.next_frame();
    assert_eq!(completed.kind, Kind::Result as i32);
    call.cancel();
    assert_eq!(block_on(call).unwrap_err().code, IpcErrorCode::Cancelled);
}

#[test]
fn duplicate_invocations_and_illegal_direction_never_run_handlers_twice() {
    let count = Arc::new(AtomicUsize::new(0));
    let counter = Arc::clone(&count);
    let mut registry = IpcRegistry::new(&SCHEMA);
    registry
        .register::<Save, _, _>(move |_, _| {
            counter.fetch_add(1, Ordering::SeqCst);
            async { Ok(()) }
        })
        .unwrap();
    let peer = Peer::new(registry, IpcOptions::for_schema(&SCHEMA));
    assert_eq!(
        peer.post(peer.invocation(1, 1101, Kind::Request, Item::default())),
        204
    );
    peer.next_frame();
    assert_eq!(
        peer.post(peer.invocation(1, 1101, Kind::Request, Item::default())),
        400
    );
    assert_eq!(
        peer.post(peer.invocation(2, 2001, Kind::Request, Item::default())),
        204
    );
    let Some(Body::Error(error)) = peer.next_frame().body else {
        panic!("missing role error");
    };
    assert_eq!(error.code, "invalid-frame");
    assert_eq!(count.load(Ordering::SeqCst), 1);
}

#[test]
fn input_permits_validate_owner_capacity_and_growth_before_allocation() {
    let peer = Peer::new(IpcRegistry::new(&SCHEMA), IpcOptions::for_schema(&SCHEMA));
    let other = Peer::new(IpcRegistry::new(&SCHEMA), IpcOptions::for_schema(&SCHEMA));
    let mut permit = peer.bridge.reserve_input(10).unwrap();
    permit.try_grow(20).unwrap();
    assert_eq!(peer.owner.window().stats().admitted_input_bytes, 30);
    assert_eq!(
        permit
            .try_grow(peer.info.limits.max_frame_bytes)
            .unwrap_err()
            .code,
        IpcErrorCode::PayloadTooLarge
    );
    let response = block_on(other.bridge.submit(OwnedIpcHttpRequest {
        navigation: other.navigation,
        method: DesktopHttpMethod::Get,
        path: "/_webui/ipc/outbound".into(),
        token: other.info.token.clone(),
        body: Vec::new(),
        input_permit: permit,
    }))
    .unwrap();
    assert_eq!(response.status, 400);
    assert_eq!(peer.owner.window().stats().admitted_input_bytes, 0);
    let body = Vec::with_capacity(128);
    let response = block_on(peer.bridge.submit(OwnedIpcHttpRequest {
        navigation: peer.navigation,
        method: DesktopHttpMethod::Get,
        path: "/_webui/ipc/outbound".into(),
        token: peer.info.token.clone(),
        body,
        input_permit: peer.bridge.reserve_input(1).unwrap(),
    }))
    .unwrap();
    assert_eq!(response.status, 400);
}

#[test]
fn saturated_payload_bytes_do_not_block_small_completion_ingress() {
    let mut options = IpcOptions::for_schema(&SCHEMA);
    options.limits.max_admitted_input_bytes_per_frame = options.limits.max_frame_bytes;
    options.limits.max_retained_bytes_per_frame = 2 * options.limits.max_frame_bytes;
    let peer = Peer::new(IpcRegistry::new(&SCHEMA), options);
    let call = peer
        .session()
        .call::<Label>(Item::default(), CallOptions::default());
    let request = peer.next_frame();
    let retained = peer.owner.window().stats().retained_bytes;
    let fill = peer
        .bridge
        .reserve_input(peer.info.limits.max_retained_bytes_per_frame - retained)
        .unwrap();
    assert_eq!(
        peer.post(peer.response(request.id, Kind::Result, Some(Body::Payload(Vec::new())))),
        204
    );
    block_on(call).unwrap();
    drop(fill);
    eventually(|| peer.owner.window().stats().retained_bytes == 0);
}

#[test]
fn caller_and_receiver_deadlines_include_blocked_worker_queue_time() {
    let (release, wait) = mpsc::channel::<()>();
    let wait = Arc::new(Mutex::new(wait));
    let (started, observed) = mpsc::channel();
    let invoked = Arc::new(AtomicUsize::new(0));
    let calls = Arc::clone(&invoked);
    let mut registry = IpcRegistry::new(&SCHEMA);
    registry
        .register::<Save, _, _>(move |_, item| {
            calls.fetch_add(1, Ordering::SeqCst);
            if item.id == 1 {
                let _ = started.send(());
                let _ = wait.lock().unwrap().recv();
            }
            async { Ok(()) }
        })
        .unwrap();
    let peer = Peer::new(registry, IpcOptions::for_schema(&SCHEMA));
    assert_eq!(
        peer.post(peer.invocation(
            1,
            1101,
            Kind::Request,
            Item {
                id: 1,
                bytes: Vec::new()
            }
        )),
        204
    );
    observed.recv_timeout(Duration::from_secs(2)).unwrap();
    let mut queued = peer.invocation(
        2,
        1101,
        Kind::Request,
        Item {
            id: 2,
            bytes: Vec::new(),
        },
    );
    queued.timeout_ms = 20;
    assert_eq!(peer.post(queued), 204);
    let timed_out = peer.next_frame();
    assert_eq!(timed_out.id, 2);
    let Some(Body::Error(error)) = timed_out.body else {
        panic!("missing receiver deadline");
    };
    assert_eq!(error.code, "deadline-exceeded");
    let outbound = peer.session().call::<Label>(
        Item::default(),
        CallOptions {
            timeout: Duration::from_millis(20),
        },
    );
    assert_eq!(
        block_on(outbound).unwrap_err().code,
        IpcErrorCode::DeadlineExceeded
    );
    // Timeout does not free permits for tasks that have not actually dropped.
    assert!(peer.owner.window().stats().worker_tasks >= 2);
    assert_eq!(invoked.load(Ordering::SeqCst), 1);
    release.send(()).unwrap();
    assert_eq!(peer.next_frame().id, 1);
    eventually(|| peer.owner.window().stats().worker_tasks == 0);
    assert_eq!(invoked.load(Ordering::SeqCst), 1);
    let stats = peer.owner.window().stats();
    std::thread::sleep(Duration::from_millis(30));
    let idle = peer.owner.window().stats();
    assert_eq!(idle.deadline_wakeups, stats.deadline_wakeups);
    assert_eq!(idle.wakeups, stats.wakeups);
}

#[test]
fn blocked_retired_handlers_keep_input_credits_and_drop_never_joins_them() {
    let (release, wait) = mpsc::channel::<()>();
    let wait = Arc::new(Mutex::new(wait));
    let (started, observed) = mpsc::channel();
    let mut registry = IpcRegistry::new(&SCHEMA);
    registry
        .register::<Save, _, _>(move |_, _| {
            let _ = started.send(());
            let _ = wait.lock().unwrap().recv();
            async { Ok(()) }
        })
        .unwrap();
    let mut options = IpcOptions::for_schema(&SCHEMA);
    options.worker_threads = 2;
    options.limits.max_frame_bytes = 400 * 1024;
    options.limits.max_admitted_input_bytes_per_frame = 400 * 1024;
    options.limits.max_retained_bytes_per_frame = 2 * 1024 * 1024;
    let mut peer = Peer::new(registry, options);
    let mut request = peer.invocation(
        1,
        1101,
        Kind::Request,
        Item {
            id: 1,
            bytes: vec![9; 300 * 1024],
        },
    );
    request.timeout_ms = 20;
    assert_eq!(peer.post(request), 204);
    observed.recv_timeout(Duration::from_secs(2)).unwrap();
    let timeout = peer.next_frame();
    assert_eq!(timeout.kind, Kind::Error as i32);
    let before = peer.owner.window().stats();
    assert!(before.admitted_input_bytes >= 300 * 1024);
    assert_eq!(before.worker_tasks, 1);
    peer.navigate();
    let after = peer.owner.window().stats();
    assert_eq!(after.admitted_input_bytes, before.admitted_input_bytes);
    assert!(after.retained_bytes >= 600 * 1024);
    assert_eq!(
        peer.bridge.reserve_input(300 * 1024).err().unwrap().code,
        IpcErrorCode::Overloaded
    );
    let start = Instant::now();
    peer.owner.close();
    assert!(start.elapsed() < Duration::from_secs(1));
    assert_eq!(
        peer.owner.window().stats().admitted_input_bytes,
        before.admitted_input_bytes
    );
    release.send(()).unwrap();
    eventually(|| peer.owner.window().stats().worker_tasks == 0);
    assert_eq!(peer.owner.window().stats().admitted_input_bytes, 0);
}

#[test]
fn task_capacity_rejects_new_work_across_retired_documents() {
    let (release, wait) = mpsc::channel::<()>();
    let wait = Arc::new(Mutex::new(wait));
    let (started, observed) = mpsc::channel();
    let mut registry = IpcRegistry::new(&SCHEMA);
    registry
        .register::<Save, _, _>(move |_, _| {
            let _ = started.send(());
            let _ = wait.lock().unwrap().recv();
            async { Ok(()) }
        })
        .unwrap();
    let mut options = IpcOptions::for_schema(&SCHEMA);
    options.worker_threads = 2;
    options
        .limits
        .max_worker_tasks_per_frame_including_retired_documents = 2;
    let mut peer = Peer::new(registry, options);
    assert_eq!(
        peer.post(peer.invocation(1, 1101, Kind::Request, Item::default())),
        204
    );
    observed.recv_timeout(Duration::from_secs(2)).unwrap();
    peer.navigate();
    assert_eq!(
        peer.post(peer.invocation(1, 1101, Kind::Request, Item::default())),
        204
    );
    observed.recv_timeout(Duration::from_secs(2)).unwrap();
    assert_eq!(peer.owner.window().stats().worker_tasks, 2);
    assert_eq!(
        peer.post(peer.invocation(2, 1101, Kind::Request, Item::default())),
        429
    );
    let start = Instant::now();
    drop(peer.owner);
    assert!(start.elapsed() < Duration::from_secs(1));
    release.send(()).unwrap();
    release.send(()).unwrap();
}

#[test]
fn subscription_receipt_order_disposal_and_capture_release_are_observable() {
    let peer = Peer::new(IpcRegistry::new(&SCHEMA), IpcOptions::for_schema(&SCHEMA));
    let (release, wait) = futures_channel::oneshot::channel::<()>();
    let wait = Arc::new(Mutex::new(Some(wait)));
    let (sent, received) = mpsc::channel();
    let capture = Arc::new(());
    let weak = Arc::downgrade(&capture);
    let mut guard = peer
        .session()
        .subscribe::<Selected, _, _>(move |_, item| {
            let _capture = Arc::clone(&capture);
            let wait = Arc::clone(&wait);
            let sent = sent.clone();
            async move {
                sent.send(item.id).unwrap();
                if item.id == 1 {
                    let wait = wait.lock().unwrap().take().unwrap();
                    let _ = wait.await;
                }
                drop(_capture);
                Ok(())
            }
        })
        .unwrap();
    assert_eq!(
        peer.post(peer.invocation(
            1,
            1102,
            Kind::Notify,
            Item {
                id: 1,
                bytes: Vec::new()
            }
        )),
        204
    );
    assert_eq!(peer.next_frame().kind, Kind::Accept as i32);
    assert_eq!(received.recv_timeout(Duration::from_secs(2)).unwrap(), 1);
    assert_eq!(
        peer.post(peer.invocation(
            2,
            1102,
            Kind::Notify,
            Item {
                id: 2,
                bytes: Vec::new()
            }
        )),
        204
    );
    assert_eq!(peer.next_frame().kind, Kind::Accept as i32);
    assert!(received.try_recv().is_err());
    release.send(()).unwrap();
    assert_eq!(received.recv_timeout(Duration::from_secs(2)).unwrap(), 2);
    guard.close();
    guard.close();
    eventually(|| weak.upgrade().is_none());
    assert_eq!(
        peer.post(peer.invocation(
            3,
            1102,
            Kind::Notify,
            Item {
                id: 3,
                bytes: Vec::new()
            }
        )),
        204
    );
    assert_eq!(peer.next_frame().kind, Kind::Accept as i32);
    assert!(received.try_recv().is_err());
}

#[test]
fn outbound_drain_reserves_one_response_and_transfers_credit_through_native_body_lease() {
    let peer = Peer::new(IpcRegistry::new(&SCHEMA), IpcOptions::for_schema(&SCHEMA));
    let call = peer.session().call::<Label>(
        Item {
            id: 1,
            bytes: vec![3; 32 * 1024],
        },
        CallOptions::default(),
    );
    eventually(|| peer.owner.window().stats().queued_bytes > 0);
    let before = peer.owner.window().stats().retained_bytes;
    let drain = peer.bridge.submit(OwnedIpcHttpRequest {
        navigation: peer.navigation,
        method: DesktopHttpMethod::Get,
        path: "/_webui/ipc/outbound".into(),
        token: peer.info.token.clone(),
        body: Vec::new(),
        input_permit: peer.bridge.reserve_input(0).unwrap(),
    });
    assert_eq!(peer.owner.window().stats().queued_bytes, 0);
    assert_eq!(peer.owner.window().stats().retained_bytes, before);
    assert_eq!(
        peer.submit(DesktopHttpMethod::Get, "/_webui/ipc/outbound", Vec::new())
            .status,
        429
    );
    let response = block_on(drain).unwrap();
    assert_eq!(response.status, 200);
    assert_eq!(peer.owner.window().stats().retained_bytes, before);
    let (bytes, lease) = response.body.into_bytes().unwrap().into_parts();
    assert!(lease.is_some());
    drop(bytes);
    assert_eq!(peer.owner.window().stats().retained_bytes, before);
    drop(lease);
    assert!(peer.owner.window().stats().retained_bytes < before);
    call.cancel();
    assert_eq!(block_on(call).unwrap_err().code, IpcErrorCode::Cancelled);
}

#[test]
fn already_completed_admission_and_binary_response_cannot_cross_navigation() {
    let owner = IpcWindowOwner::new(
        Arc::new(IpcRegistry::new(&SCHEMA)),
        IpcOptions::for_schema(&SCHEMA),
        IpcHost::Source {
            origin: "webui://app".into(),
        },
    )
    .unwrap();
    let bridge = owner.bridge();
    bridge.navigate(1);
    let proof = bridge.begin_document(identity(1), [1; 16]).unwrap();
    let admission = bridge.admit(Admission {
        hello: hello(),
        proof,
    });
    eventually(|| owner.window().current_session().is_ok());
    bridge.navigate(2);
    assert_eq!(
        block_on(admission).err().unwrap().code,
        IpcErrorCode::Navigated
    );

    let peer = Peer::new(IpcRegistry::new(&SCHEMA), IpcOptions::for_schema(&SCHEMA));
    let response = peer.bridge.submit(OwnedIpcHttpRequest {
        navigation: peer.navigation,
        method: DesktopHttpMethod::Get,
        path: "/_webui/ipc/outbound".into(),
        token: peer.info.token.clone(),
        body: Vec::new(),
        input_permit: peer.bridge.reserve_input(0).unwrap(),
    });
    peer.bridge.navigate(peer.navigation + 1);
    assert_eq!(
        block_on(response).err().unwrap().code,
        IpcErrorCode::Navigated
    );
}

#[test]
fn source_permissions_are_explicit_and_packaged_hosts_always_deny_development_methods() {
    for source in [false, true] {
        let count = Arc::new(AtomicUsize::new(0));
        let handler_count = Arc::clone(&count);
        let mut registry = IpcRegistry::new(&SCHEMA);
        registry
            .register::<DebugCall, _, _>(move |_, _| {
                handler_count.fetch_add(1, Ordering::SeqCst);
                async { Ok(()) }
            })
            .unwrap();
        let mut options = IpcOptions::for_schema(&SCHEMA);
        options.development = true;
        let host = if source {
            IpcHost::Source {
                origin: "webui://app".into(),
            }
        } else {
            IpcHost::Packaged {
                origin: "webui://app".into(),
            }
        };
        let peer = Peer::with_host(registry, options, host);
        assert_eq!(
            peer.post(peer.invocation(1, 1103, Kind::Request, Item::default())),
            204
        );
        let response = peer.next_frame();
        assert_eq!(count.load(Ordering::SeqCst), usize::from(source));
        if source {
            assert_eq!(response.kind, Kind::Result as i32);
        } else {
            let Some(Body::Error(error)) = response.body else {
                panic!("missing packaged denial");
            };
            assert_eq!(error.code, "permission-denied");
        }
    }
    let peer = Peer::new(IpcRegistry::new(&SCHEMA), IpcOptions::default());
    assert_eq!(
        block_on(
            peer.session()
                .call::<Label>(Item::default(), CallOptions::default())
        )
        .unwrap_err()
        .code,
        IpcErrorCode::PermissionDenied
    );
}

#[test]
fn closing_subscription_skips_queued_callbacks_and_releases_captures() {
    let peer = Peer::new(IpcRegistry::new(&SCHEMA), IpcOptions::for_schema(&SCHEMA));
    let (sent, received) = mpsc::channel();
    let capture = Arc::new(());
    let weak = Arc::downgrade(&capture);
    let mut guard = peer
        .session()
        .subscribe::<Selected, _, _>(move |_, item| {
            let capture = Arc::clone(&capture);
            let sent = sent.clone();
            async move {
                sent.send(item.id).unwrap();
                if item.id == 1 {
                    std::future::pending::<()>().await;
                }
                drop(capture);
                Ok(())
            }
        })
        .unwrap();
    assert_eq!(
        peer.post(peer.invocation(
            1,
            1102,
            Kind::Notify,
            Item {
                id: 1,
                bytes: Vec::new()
            }
        )),
        204
    );
    assert_eq!(peer.next_frame().kind, Kind::Accept as i32);
    assert_eq!(received.recv_timeout(Duration::from_secs(2)).unwrap(), 1);
    assert_eq!(
        peer.post(peer.invocation(
            2,
            1102,
            Kind::Notify,
            Item {
                id: 2,
                bytes: Vec::new()
            }
        )),
        204
    );
    assert_eq!(peer.next_frame().kind, Kind::Accept as i32);
    guard.close();
    eventually(|| weak.upgrade().is_none());
    assert!(received.try_recv().is_err());
}

#[test]
fn peer_cancel_signals_cooperative_token_without_a_reply() {
    let (sent, received) = mpsc::channel();
    let mut registry = IpcRegistry::new(&SCHEMA);
    registry
        .register::<Save, _, _>(move |ctx, _| {
            sent.send(ctx.cancellation).unwrap();
            async { std::future::pending::<Result<(), IpcError>>().await }
        })
        .unwrap();
    let peer = Peer::new(registry, IpcOptions::for_schema(&SCHEMA));
    assert_eq!(
        peer.post(peer.invocation(1, 1101, Kind::Request, Item::default())),
        204
    );
    let cancellation = received.recv_timeout(Duration::from_secs(2)).unwrap();
    assert!(!cancellation.is_cancelled());
    assert_eq!(peer.post(peer.response(1, Kind::Cancel, None)), 204);
    assert!(cancellation.is_cancelled());
    block_on(cancellation.cancelled());
    eventually(|| peer.owner.window().stats().worker_tasks == 0);
    assert_eq!(
        peer.submit(DesktopHttpMethod::Get, "/_webui/ipc/outbound", Vec::new())
            .status,
        204
    );
}

#[test]
fn handler_errors_are_bounded_and_oversized_outbound_messages_never_enter_queue() {
    let mut registry = IpcRegistry::new(&SCHEMA);
    registry
        .register::<Save, _, _>(|_, _| async {
            Err(IpcError {
                code: IpcErrorCode::Handler,
                message: "é".repeat(5000),
                help: "h".repeat(5000),
                application_code: Some("a".repeat(5000)),
            })
        })
        .unwrap();
    let peer = Peer::new(registry, IpcOptions::for_schema(&SCHEMA));
    assert_eq!(
        peer.post(peer.invocation(1, 1101, Kind::Request, Item::default())),
        204
    );
    let frame = peer.next_frame();
    assert!(frame.encoded_len() <= peer.info.limits.max_frame_bytes);
    let Some(Body::Error(error)) = frame.body else {
        panic!("missing bounded error");
    };
    assert_eq!(error.code, "handler");
    assert!(
        error.code.len() + error.message.len() + error.help.len() + error.application_code.len()
            <= peer.info.limits.max_error_text_bytes_total
    );
    let call = peer.session().call::<Label>(
        Item {
            id: 1,
            bytes: vec![0; peer.info.limits.max_frame_bytes],
        },
        CallOptions::default(),
    );
    assert_eq!(
        block_on(call).unwrap_err().code,
        IpcErrorCode::PayloadTooLarge
    );
    assert_eq!(peer.owner.window().stats().queued_bytes, 0);
}

#[test]
fn control_wakes_coalesce_and_new_empty_to_nonempty_transition_wakes_again() {
    let mut registry = IpcRegistry::new(&SCHEMA);
    registry
        .register::<Save, _, _>(|_, _| async { Ok(()) })
        .unwrap();
    let peer = Peer::new(registry, IpcOptions::for_schema(&SCHEMA));
    for id in 1..=4 {
        assert_eq!(
            peer.post(peer.invocation(id, 1101, Kind::Request, Item::default())),
            204
        );
    }
    eventually(|| peer.owner.window().stats().worker_tasks == 0);
    assert_eq!(peer.owner.window().stats().wakeups, 1);
    assert_eq!(
        peer.bridge.take_control().unwrap(),
        Some(NativeControl::Ready {
            generation: peer.info.generation
        })
    );
    assert_eq!(peer.bridge.take_control().unwrap(), None);
    for _ in 0..4 {
        peer.next_frame();
    }
    assert_eq!(
        peer.submit(DesktopHttpMethod::Get, "/_webui/ipc/outbound", Vec::new())
            .status,
        204
    );
    assert_eq!(
        peer.post(peer.invocation(5, 1101, Kind::Request, Item::default())),
        204
    );
    eventually(|| peer.owner.window().stats().wakeups == 2);
    assert_eq!(peer.next_frame().id, 5);
}

#[test]
fn nested_rpc_at_capacity_fails_instead_of_waiting_for_its_own_permit() {
    let mut registry = IpcRegistry::new(&SCHEMA);
    registry
        .register::<Save, _, _>(|ctx, item| async move {
            ctx.session
                .call::<Label>(item, CallOptions::default())
                .await?;
            Ok(())
        })
        .unwrap();
    let mut options = IpcOptions::for_schema(&SCHEMA);
    options
        .limits
        .max_worker_tasks_per_frame_including_retired_documents = 1;
    let peer = Peer::new(registry, options);
    eventually(|| peer.owner.window().stats().worker_tasks == 0);
    assert_eq!(
        peer.post(peer.invocation(1, 1101, Kind::Request, Item::default())),
        204
    );
    let Some(Body::Error(error)) = peer.next_frame().body else {
        panic!("missing bounded reentrancy failure");
    };
    assert_eq!(error.code, "overloaded");
}

#[test]
fn notification_fanout_capacity_is_reserved_atomically_before_acceptance() {
    let mut options = IpcOptions::for_schema(&SCHEMA);
    options
        .limits
        .max_worker_tasks_per_frame_including_retired_documents = 2;
    let peer = Peer::new(IpcRegistry::new(&SCHEMA), options);
    eventually(|| peer.owner.window().stats().worker_tasks == 0);
    let count = Arc::new(AtomicUsize::new(0));
    let first_count = Arc::clone(&count);
    let second_count = Arc::clone(&count);
    let _first = peer
        .session()
        .subscribe::<Selected, _, _>(move |_, _| {
            first_count.fetch_add(1, Ordering::SeqCst);
            async { Ok(()) }
        })
        .unwrap();
    let second = peer
        .session()
        .subscribe::<Selected, _, _>(move |_, _| {
            second_count.fetch_add(1, Ordering::SeqCst);
            async { Ok(()) }
        })
        .unwrap();
    assert_eq!(
        peer.post(peer.invocation(1, 1102, Kind::Notify, Item::default())),
        429
    );
    assert_eq!(count.load(Ordering::SeqCst), 0);
    assert_eq!(peer.owner.window().stats().queued_bytes, 0);
    drop(second);
    assert_eq!(
        peer.post(peer.invocation(2, 1102, Kind::Notify, Item::default())),
        204
    );
    assert_eq!(peer.next_frame().kind, Kind::Accept as i32);
    eventually(|| count.load(Ordering::SeqCst) == 1);
}

#[test]
fn failing_native_wake_closes_transport_without_retrying_application_work() {
    struct FailedWake;
    impl IpcWake for FailedWake {
        fn wake(&self) -> Result<(), IpcError> {
            Err(IpcError::new(
                IpcErrorCode::Transport,
                "native loop stopped",
                "close the frame",
            ))
        }
    }
    let owner = IpcWindowOwner::new(
        Arc::new(IpcRegistry::new(&SCHEMA)),
        IpcOptions::for_schema(&SCHEMA),
        IpcHost::Source {
            origin: "webui://app".into(),
        },
    )
    .unwrap();
    let bridge = owner.bridge();
    bridge.attach_waker(Arc::new(FailedWake)).unwrap();
    bridge.navigate(1);
    let proof = bridge.begin_document(identity(1), [1; 16]).unwrap();
    block_on(bridge.admit(Admission {
        hello: hello(),
        proof,
    }))
    .unwrap();
    let session = owner.window().current_session().unwrap();
    let error =
        block_on(session.call::<Label>(Item::default(), CallOptions::default())).unwrap_err();
    assert_eq!(error.code, IpcErrorCode::Transport);
    assert!(session.is_closed());
    assert_eq!(owner.window().stats().wakeups, 1);
}

#[test]
fn native_owned_result_body_keeps_its_credit_after_navigation_and_close() {
    let mut registry = IpcRegistry::new(&SCHEMA);
    registry
        .register::<Echo, _, _>(|_, item| async move { Ok(item) })
        .unwrap();
    let mut peer = Peer::new(registry, IpcOptions::for_schema(&SCHEMA));
    let item = Item {
        id: u64::MAX,
        bytes: vec![0x6a; 256 * 1024],
    };
    assert_eq!(
        peer.post(peer.invocation(1, 1104, Kind::Request, item.clone())),
        204
    );
    eventually(|| peer.owner.window().stats().queued_bytes > 0);
    let response = peer.submit(DesktopHttpMethod::Get, "/_webui/ipc/outbound", Vec::new());
    let encoded_size = response.body.as_bytes().unwrap().len();
    let frame = IpcFrame::decode(response.body.as_bytes().unwrap().as_slice()).unwrap();
    assert_eq!(frame.kind, Kind::Result as i32);
    let Some(Body::Payload(payload)) = frame.body else {
        panic!("missing binary reply");
    };
    assert_eq!(Item::decode_ipc(payload.as_slice()).unwrap(), item);
    eventually(|| peer.owner.window().stats().worker_tasks == 0);
    assert_eq!(peer.owner.window().stats().admitted_input_bytes, 0);
    assert_eq!(peer.owner.window().stats().retained_bytes, encoded_size);

    let (bytes, lease) = response.body.into_bytes().unwrap().into_parts();
    assert!(lease.is_some());
    peer.navigate();
    eventually(|| peer.owner.window().stats().worker_tasks == 0);
    assert_eq!(peer.owner.window().stats().retained_bytes, encoded_size);
    peer.owner.close();
    assert_eq!(peer.owner.window().stats().retained_bytes, encoded_size);
    drop(bytes);
    assert_eq!(peer.owner.window().stats().retained_bytes, encoded_size);
    drop(lease);
    assert_eq!(peer.owner.window().stats().retained_bytes, 0);
}

#[test]
fn dropping_retired_drain_does_not_release_the_new_documents_drain_slot() {
    let mut peer = Peer::new(IpcRegistry::new(&SCHEMA), IpcOptions::for_schema(&SCHEMA));
    let call = peer.session().call::<Label>(
        Item {
            id: 1,
            bytes: vec![1; 32 * 1024],
        },
        CallOptions::default(),
    );
    eventually(|| peer.owner.window().stats().queued_bytes > 0);
    let old_drain = peer.bridge.submit(OwnedIpcHttpRequest {
        navigation: peer.navigation,
        method: DesktopHttpMethod::Get,
        path: "/_webui/ipc/outbound".into(),
        token: peer.info.token.clone(),
        body: Vec::new(),
        input_permit: peer.bridge.reserve_input(0).unwrap(),
    });
    peer.navigate();
    assert_eq!(block_on(call).unwrap_err().code, IpcErrorCode::Navigated);
    let new_drain = peer.bridge.submit(OwnedIpcHttpRequest {
        navigation: peer.navigation,
        method: DesktopHttpMethod::Get,
        path: "/_webui/ipc/outbound".into(),
        token: peer.info.token.clone(),
        body: Vec::new(),
        input_permit: peer.bridge.reserve_input(0).unwrap(),
    });
    drop(old_drain);
    assert_eq!(
        peer.submit(DesktopHttpMethod::Get, "/_webui/ipc/outbound", Vec::new())
            .status,
        429
    );
    drop(new_drain);
    assert_eq!(
        peer.submit(DesktopHttpMethod::Get, "/_webui/ipc/outbound", Vec::new())
            .status,
        204
    );
    eventually(|| peer.owner.window().stats().retained_bytes == 0);
}

#[test]
fn disconnect_requires_current_generation_and_secret_from_the_same_window() {
    let mut peer = Peer::new(IpcRegistry::new(&SCHEMA), IpcOptions::for_schema(&SCHEMA));
    let other = Peer::new(IpcRegistry::new(&SCHEMA), IpcOptions::for_schema(&SCHEMA));
    let session = peer.session();
    let call = session.call::<Label>(Item::default(), CallOptions::default());
    peer.next_frame();
    for token in [
        "",
        "0",
        "000000000000000000000000000000000",
        other.info.token.as_str(),
    ] {
        assert_eq!(
            peer.bridge
                .disconnect_authenticated(peer.info.generation, token)
                .unwrap_err()
                .code,
            IpcErrorCode::PermissionDenied
        );
        assert!(!session.is_closed());
    }
    assert_eq!(
        peer.bridge
            .disconnect_authenticated(peer.info.generation + 1, &peer.info.token)
            .unwrap_err()
            .code,
        IpcErrorCode::PermissionDenied
    );
    for index in 0..peer.info.token.len() {
        let mut forged = peer.info.token.clone().into_bytes();
        forged[index] = if forged[index] == b'0' { b'1' } else { b'0' };
        let forged = String::from_utf8(forged).unwrap();
        assert_eq!(
            peer.bridge
                .disconnect_authenticated(peer.info.generation, &forged)
                .unwrap_err()
                .code,
            IpcErrorCode::PermissionDenied
        );
    }
    assert!(!session.is_closed());
    let old_generation = peer.info.generation;
    let old_token = peer.info.token.clone();
    peer.bridge
        .disconnect_authenticated(old_generation, &old_token)
        .unwrap();
    assert_eq!(block_on(call).unwrap_err().code, IpcErrorCode::Closed);
    peer.navigate();
    assert_eq!(
        peer.bridge
            .disconnect_authenticated(old_generation, &old_token)
            .unwrap_err()
            .code,
        IpcErrorCode::PermissionDenied
    );
    assert_eq!(
        peer.bridge
            .disconnect_authenticated(peer.info.generation, &old_token)
            .unwrap_err()
            .code,
        IpcErrorCode::PermissionDenied
    );
    assert!(!peer.session().is_closed());
    assert!(!other.session().is_closed());
}

#[test]
fn every_proof_secret_byte_and_cross_window_challenge_are_checked_without_consumption() {
    let make_owner = || {
        IpcWindowOwner::new(
            Arc::new(IpcRegistry::new(&SCHEMA)),
            IpcOptions::for_schema(&SCHEMA),
            IpcHost::Source {
                origin: "webui://app".into(),
            },
        )
        .unwrap()
    };
    let owner = make_owner();
    let other_owner = make_owner();
    let bridge = owner.bridge();
    let other = other_owner.bridge();
    bridge.navigate(1);
    other.navigate(1);
    let proof = bridge.begin_document(identity(1), [7; 16]).unwrap();
    let other_proof = other.begin_document(identity(1), [7; 16]).unwrap();
    assert_eq!(
        block_on(bridge.admit(Admission {
            hello: hello(),
            proof: other_proof.clone()
        }))
        .err()
        .unwrap()
        .code,
        IpcErrorCode::PermissionDenied
    );
    for index in 0..16 {
        let mut bad_nonce = proof.clone();
        bad_nonce.document_nonce[index] ^= 1;
        assert_eq!(
            block_on(bridge.admit(Admission {
                hello: hello(),
                proof: bad_nonce
            }))
            .err()
            .unwrap()
            .code,
            IpcErrorCode::PermissionDenied
        );
        let mut bad_challenge = proof.clone();
        bad_challenge.challenge[index] ^= 1;
        assert_eq!(
            block_on(bridge.admit(Admission {
                hello: hello(),
                proof: bad_challenge
            }))
            .err()
            .unwrap()
            .code,
            IpcErrorCode::PermissionDenied
        );
    }
    assert!(!owner.window().stats().workers_started);
    block_on(bridge.admit(Admission {
        hello: hello(),
        proof,
    }))
    .unwrap();
    block_on(other.admit(Admission {
        hello: hello(),
        proof: other_proof,
    }))
    .unwrap();
    assert!(!owner.window().current_session().unwrap().is_closed());
    assert!(!other_owner.window().current_session().unwrap().is_closed());
}

#[test]
fn delayed_first_hello_keeps_document_proof_valid_until_retirement() {
    let mut options = IpcOptions::for_schema(&SCHEMA);
    options.limits.handshake_timeout_ms = 50;
    let owner = IpcWindowOwner::new(
        Arc::new(IpcRegistry::new(&SCHEMA)),
        options,
        IpcHost::Source {
            origin: "webui://app".into(),
        },
    )
    .unwrap();
    let bridge = owner.bridge();
    bridge.navigate(1);
    let proof = bridge.begin_document(identity(1), [7; 16]).unwrap();
    std::thread::sleep(Duration::from_millis(100));
    assert!(!owner.window().stats().workers_started);
    let session = block_on(bridge.admit(Admission {
        hello: hello(),
        proof: proof.clone(),
    }))
    .unwrap();
    assert_eq!(session.generation, 1);
    bridge.navigate(2);
    assert!(block_on(bridge.admit(Admission {
        hello: hello(),
        proof,
    }))
    .is_err());
}

#[test]
fn handshake_deadline_includes_time_queued_behind_retired_work() {
    let (release, wait) = mpsc::channel::<()>();
    let wait = Arc::new(Mutex::new(wait));
    let (started, observed) = mpsc::channel();
    let mut registry = IpcRegistry::new(&SCHEMA);
    registry
        .register::<Save, _, _>(move |_, _| {
            started.send(()).unwrap();
            wait.lock().unwrap().recv().unwrap();
            async { Ok(()) }
        })
        .unwrap();
    let mut options = IpcOptions::for_schema(&SCHEMA);
    options.worker_threads = 1;
    options.limits.handshake_timeout_ms = 100;
    let peer = Peer::new(registry, options);
    assert_eq!(
        peer.post(peer.invocation(1, 1101, Kind::Request, Item::default())),
        204
    );
    observed.recv_timeout(Duration::from_secs(2)).unwrap();
    peer.bridge.navigate(2);
    let proof = peer.bridge.begin_document(identity(2), [8; 16]).unwrap();
    let pending = peer.bridge.admit(Admission {
        hello: hello(),
        proof,
    });
    std::thread::sleep(Duration::from_millis(150));
    release.send(()).unwrap();
    assert_eq!(
        block_on(pending).err().unwrap().code,
        IpcErrorCode::DeadlineExceeded
    );
    assert!(peer.owner.window().current_session().is_err());
}

#[test]
fn admitted_session_delivered_after_handshake_deadline_is_revoked() {
    let mut options = IpcOptions::for_schema(&SCHEMA);
    options.limits.handshake_timeout_ms = 100;
    let owner = IpcWindowOwner::new(
        Arc::new(IpcRegistry::new(&SCHEMA)),
        options,
        IpcHost::Source {
            origin: "webui://app".into(),
        },
    )
    .unwrap();
    let bridge = owner.bridge();
    bridge.navigate(1);
    let proof = bridge.begin_document(identity(1), [7; 16]).unwrap();
    let pending = bridge.admit(Admission {
        hello: hello(),
        proof,
    });
    eventually(|| owner.window().current_session().is_ok());
    std::thread::sleep(Duration::from_millis(150));
    assert_eq!(
        block_on(pending).err().unwrap().code,
        IpcErrorCode::DeadlineExceeded
    );
    assert!(owner.window().current_session().is_err());
}

#[test]
fn concurrent_duplicate_hello_admits_exactly_one_session() {
    let mut options = IpcOptions::for_schema(&SCHEMA);
    options.worker_threads = 2;
    let owner = IpcWindowOwner::new(
        Arc::new(IpcRegistry::new(&SCHEMA)),
        options,
        IpcHost::Source {
            origin: "webui://app".into(),
        },
    )
    .unwrap();
    let bridge = owner.bridge();
    bridge.navigate(1);
    let proof = bridge.begin_document(identity(1), [3; 16]).unwrap();
    let ready = owner.window().ready();
    let barrier = Arc::new(std::sync::Barrier::new(3));
    let mut threads = Vec::new();
    for _ in 0..2 {
        let bridge = bridge.clone();
        let proof = proof.clone();
        let barrier = Arc::clone(&barrier);
        threads.push(std::thread::spawn(move || {
            barrier.wait();
            block_on(bridge.admit(Admission {
                hello: hello(),
                proof,
            }))
        }));
    }
    barrier.wait();
    let mut successes = 0;
    let mut failures = 0;
    for thread in threads {
        match thread.join().unwrap() {
            Ok(info) => {
                successes += 1;
                assert_eq!(info.generation, 1);
            }
            Err(error) => {
                failures += 1;
                assert_eq!(error.code, IpcErrorCode::NotReady);
            }
        }
    }
    assert_eq!((successes, failures), (1, 1));
    assert_eq!(block_on(ready).unwrap().generation(), 1);
    assert_eq!(owner.window().current_session().unwrap().generation(), 1);
}

#[test]
fn failed_hello_delivery_revokes_only_its_specific_authenticated_session() {
    let owner = IpcWindowOwner::new(
        Arc::new(IpcRegistry::new(&SCHEMA)),
        IpcOptions::for_schema(&SCHEMA),
        IpcHost::Source {
            origin: "webui://app".into(),
        },
    )
    .unwrap();
    let bridge = owner.bridge();
    bridge.navigate(1);
    let proof = bridge.begin_document(identity(1), [1; 16]).unwrap();
    let failed = bridge.admit(Admission {
        hello: hello(),
        proof,
    });
    eventually(|| owner.window().current_session().is_ok());
    drop(failed);
    eventually(|| owner.window().current_session().is_err());
    assert!(bridge.begin_document(identity(1), [2; 16]).is_err());

    bridge.navigate(2);
    let proof = bridge.begin_document(identity(2), [2; 16]).unwrap();
    let old_delivery = bridge.admit(Admission {
        hello: hello(),
        proof,
    });
    eventually(|| owner.window().current_session().is_ok());
    bridge.navigate(3);
    let proof = bridge.begin_document(identity(3), [3; 16]).unwrap();
    let current = block_on(bridge.admit(Admission {
        hello: hello(),
        proof,
    }))
    .unwrap();
    drop(old_delivery);
    assert_eq!(
        owner.window().current_session().unwrap().generation(),
        current.generation
    );
    assert!(!owner.window().current_session().unwrap().is_closed());
}

#[test]
fn concurrent_old_disconnect_and_navigation_cannot_retire_replacement_session() {
    for _ in 0..8 {
        let mut peer = Peer::new(IpcRegistry::new(&SCHEMA), IpcOptions::for_schema(&SCHEMA));
        let bridge = peer.bridge.clone();
        let generation = peer.info.generation;
        let token = peer.info.token.clone();
        let barrier = Arc::new(std::sync::Barrier::new(2));
        let other = Arc::clone(&barrier);
        let disconnect = std::thread::spawn(move || {
            other.wait();
            bridge.disconnect_authenticated(generation, &token)
        });
        barrier.wait();
        peer.navigate();
        if let Err(error) = disconnect.join().unwrap() {
            assert!(matches!(
                error.code,
                IpcErrorCode::Closed | IpcErrorCode::PermissionDenied
            ));
        }
        assert_eq!(peer.session().generation(), peer.info.generation);
        assert!(!peer.session().is_closed());
    }
}

#[test]
fn authenticated_disconnect_revokes_an_already_queued_binary_response() {
    let peer = Peer::new(IpcRegistry::new(&SCHEMA), IpcOptions::for_schema(&SCHEMA));
    let call = peer.session().call::<Label>(
        Item {
            id: 7,
            bytes: vec![4; 32 * 1024],
        },
        CallOptions::default(),
    );
    eventually(|| peer.owner.window().stats().queued_bytes > 0);
    let response = peer.bridge.submit(OwnedIpcHttpRequest {
        navigation: peer.navigation,
        method: DesktopHttpMethod::Get,
        path: "/_webui/ipc/outbound".into(),
        token: peer.info.token.clone(),
        body: Vec::new(),
        input_permit: peer.bridge.reserve_input(0).unwrap(),
    });
    peer.bridge
        .disconnect_authenticated(peer.info.generation, &peer.info.token)
        .unwrap();
    assert_eq!(block_on(response).err().unwrap().code, IpcErrorCode::Closed);
    assert_eq!(block_on(call).unwrap_err().code, IpcErrorCode::Closed);
    eventually(|| peer.owner.window().stats().retained_bytes == 0);
}

#[test]
fn sequencing_cancelled_middle_call_cannot_overtake_blocked_validation() {
    let (started, observed) = mpsc::channel();
    let (release, wait) = mpsc::channel();
    *OUTBOUND_VALIDATION_GATE.lock().unwrap() = Some(ValidationGate {
        started,
        release: wait,
    });
    let mut options = IpcOptions::for_schema(&SCHEMA);
    options.worker_threads = 2;
    let peer = Peer::new(IpcRegistry::new(&SCHEMA), options);
    let session = peer.session();
    let first = session.call::<Label>(
        Item {
            id: 9999,
            bytes: Vec::new(),
        },
        CallOptions::default(),
    );
    observed.recv_timeout(Duration::from_secs(2)).unwrap();
    let middle = session.call::<Label>(
        Item {
            id: 2,
            bytes: Vec::new(),
        },
        CallOptions::default(),
    );
    let last = session.call::<Label>(
        Item {
            id: 3,
            bytes: Vec::new(),
        },
        CallOptions::default(),
    );
    middle.cancel();
    assert_eq!(block_on(middle).unwrap_err().code, IpcErrorCode::Cancelled);
    // A premature native wake proves a later request reached the outbound queue
    // while request 1's validator is still blocked.
    let _ = peer.wakes.recv_timeout(Duration::from_millis(100));
    let retained_tasks = peer.owner.window().stats().worker_tasks;
    release.send(()).unwrap();
    let a = peer.next_frame();
    let b = peer.next_frame();
    assert_eq!((a.id, b.id), (1, 3));
    assert_eq!(
        retained_tasks, 3,
        "the cancelled sequencing node lost its permit"
    );
    assert_eq!(
        (a.kind, b.kind),
        (Kind::Request as i32, Kind::Request as i32)
    );
    first.cancel();
    last.cancel();
    assert_eq!(block_on(first).unwrap_err().code, IpcErrorCode::Cancelled);
    assert_eq!(block_on(last).unwrap_err().code, IpcErrorCode::Cancelled);
    eventually(|| peer.owner.window().stats().worker_tasks == 0);
}

#[test]
fn sequencing_invalid_middle_notification_cannot_overlap_running_callback() {
    let peer = Peer::new(IpcRegistry::new(&SCHEMA), IpcOptions::for_schema(&SCHEMA));
    let (release, wait) = futures_channel::oneshot::channel::<()>();
    let wait = Arc::new(Mutex::new(Some(wait)));
    let (sent, received) = mpsc::channel();
    let finished = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let callback_finished = Arc::clone(&finished);
    let _subscription = peer
        .session()
        .subscribe::<Selected, _, _>(move |_, item| {
            let wait = Arc::clone(&wait);
            let sent = sent.clone();
            let finished = Arc::clone(&callback_finished);
            async move {
                sent.send((item.id, finished.load(Ordering::SeqCst)))
                    .unwrap();
                if item.id == 1 {
                    let wait = wait.lock().unwrap().take().unwrap();
                    let _ = wait.await;
                    finished.store(true, Ordering::SeqCst);
                }
                Ok(())
            }
        })
        .unwrap();
    assert_eq!(
        peer.post(peer.invocation(
            1,
            1102,
            Kind::Notify,
            Item {
                id: 1,
                bytes: Vec::new()
            }
        )),
        204
    );
    assert_eq!(peer.next_frame().kind, Kind::Accept as i32);
    assert_eq!(
        received.recv_timeout(Duration::from_secs(2)).unwrap(),
        (1, false)
    );

    let mut invalid = peer.invocation(2, 1102, Kind::Notify, Item::default());
    invalid.body = Some(Body::Payload(vec![0]));
    assert_eq!(peer.post(invalid), 204);
    let rejected = peer.next_frame();
    assert_eq!((rejected.id, rejected.kind), (2, Kind::Error as i32));
    assert_eq!(
        peer.post(peer.invocation(
            3,
            1102,
            Kind::Notify,
            Item {
                id: 3,
                bytes: Vec::new()
            }
        )),
        204
    );
    assert_eq!(peer.next_frame().kind, Kind::Accept as i32);
    let early = received.recv_timeout(Duration::from_millis(100)).ok();
    let retained_tasks = peer.owner.window().stats().worker_tasks;
    release.send(()).unwrap();
    let third = early.unwrap_or_else(|| received.recv_timeout(Duration::from_secs(2)).unwrap());
    assert_eq!(
        third,
        (3, true),
        "callback 3 started before callback 1 finished"
    );
    assert_eq!(
        retained_tasks, 6,
        "skipped fanout nodes lost their bounded permits"
    );
    assert!(finished.load(Ordering::SeqCst));
    eventually(|| peer.owner.window().stats().worker_tasks == 0);
}

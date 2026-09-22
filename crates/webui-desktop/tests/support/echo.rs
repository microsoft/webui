// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

#![allow(clippy::disallowed_methods)]

use std::sync::{mpsc, Arc};
use std::time::{Duration, Instant};

use futures_executor::block_on;
use prost::Message;
use webui_desktop::{
    ipc::{
        validate_message,
        wire::{ipc_frame::Body, IpcFrame, Kind},
        Admission, CommittedMainDocument, Endpoint, FieldKind, FieldShape, Hello, Host, IpcBridge,
        IpcError, IpcErrorCode, IpcLimits, IpcOptions, IpcRegistry, IpcSchema, IpcWake,
        MessageShape, MethodDescriptor, MethodKind, NativeControl, OwnedIpcHttpRequest, Rpc,
        SessionInfo,
    },
    DesktopFrame, DesktopHttpMethod,
};

#[derive(Clone, PartialEq, Message)]
struct EchoPayload {
    #[prost(bytes = "vec", tag = "1")]
    bytes: Vec<u8>,
}

struct Echo;

impl Rpc for Echo {
    type Request = EchoPayload;
    type Response = EchoPayload;
    type Receiver = Host;
    const ID: u32 = 1101;
}

fn validate(bytes: &[u8], limits: &IpcLimits) -> Result<(), IpcError> {
    static SHAPES: &[MessageShape] = &[MessageShape {
        fields: &[FieldShape {
            number: 1,
            kind: FieldKind::Bytes,
            repeated: false,
            oneof: None,
            map_key: false,
        }],
    }];
    validate_message(bytes, 0, SHAPES, limits)
}

static SCHEMA: IpcSchema = IpcSchema {
    name: "test.frame.echo",
    major: 1,
    hash: "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
    methods: &[MethodDescriptor {
        id: Echo::ID,
        name: "Host.Echo",
        receiver: Endpoint::Host,
        kind: MethodKind::Rpc,
        development_only: false,
        validate_request: validate,
        validate_response: Some(validate),
    }],
};

pub fn registry() -> IpcRegistry {
    let mut registry = IpcRegistry::new(&SCHEMA);
    registry
        .register::<Echo, _, _>(|_, payload| async move { Ok(payload) })
        .unwrap();
    registry
}

pub fn options() -> IpcOptions {
    IpcOptions::for_schema(&SCHEMA)
}

struct Wake(mpsc::Sender<()>);

impl IpcWake for Wake {
    fn wake(&self) -> Result<(), IpcError> {
        self.0.send(()).map_err(|_| {
            IpcError::new(
                IpcErrorCode::Transport,
                "test receiver is closed",
                "retain the test peer until its response is delivered",
            )
        })
    }
}

fn submit(
    bridge: &IpcBridge,
    info: &SessionInfo,
    method: DesktopHttpMethod,
    path: &str,
    body: Vec<u8>,
) -> webui_desktop::DesktopProtocolResponse {
    let input_permit = bridge.reserve_input(body.capacity()).unwrap();
    block_on(bridge.submit(OwnedIpcHttpRequest {
        navigation: 1,
        method,
        path: path.to_string(),
        token: info.token.clone(),
        body,
        input_permit,
    }))
    .unwrap()
}

pub fn assert_echo(frame: &DesktopFrame, payload: &[u8]) {
    let bridge = frame.ipc_bridge();
    let (sender, wakes) = mpsc::channel();
    bridge.attach_waker(Arc::new(Wake(sender))).unwrap();
    bridge.navigate(1);
    let origin = if cfg!(target_os = "windows") {
        "https://app.webui.localhost"
    } else {
        "webui://app"
    };
    let proof = bridge
        .begin_document(
            CommittedMainDocument {
                navigation: 1,
                origin: origin.to_string(),
            },
            [1; 16],
        )
        .unwrap();
    let info = block_on(bridge.admit(Admission {
        hello: Hello {
            wire_version: 2,
            contract_name: SCHEMA.name.to_string(),
            contract_major: SCHEMA.major,
            schema_hash: SCHEMA.hash.to_string(),
        },
        proof,
    }))
    .unwrap();
    let request = IpcFrame {
        version: 2,
        generation: info.generation,
        id: 7,
        kind: Kind::Request as i32,
        method_id: Echo::ID,
        timeout_ms: 3_000,
        body: Some(Body::Payload(
            EchoPayload {
                bytes: payload.to_vec(),
            }
            .encode_to_vec(),
        )),
    };
    assert_eq!(
        submit(
            &bridge,
            &info,
            DesktopHttpMethod::Post,
            "/_webui/ipc",
            request.encode_to_vec(),
        )
        .status,
        204
    );
    let deadline = Instant::now() + Duration::from_secs(3);
    loop {
        let response = submit(
            &bridge,
            &info,
            DesktopHttpMethod::Get,
            "/_webui/ipc/outbound",
            Vec::new(),
        );
        if response.status == 200 {
            let reply = IpcFrame::decode(response.body.as_bytes().unwrap().as_slice()).unwrap();
            assert_eq!(reply.id, 7);
            assert_eq!(reply.kind, Kind::Result as i32);
            let Some(Body::Payload(bytes)) = reply.body else {
                panic!("expected typed echo response");
            };
            assert_eq!(
                EchoPayload::decode(bytes.as_slice()).unwrap().bytes,
                payload
            );
            break;
        }
        assert_eq!(response.status, 204);
        let mut ready = false;
        while let Some(control) = bridge.take_control().unwrap() {
            ready |= matches!(
                control,
                NativeControl::Ready { generation } if generation == info.generation
            );
        }
        if ready {
            assert!(Instant::now() < deadline);
            continue;
        }
        wakes
            .recv_timeout(deadline.saturating_duration_since(Instant::now()))
            .unwrap();
    }
}

// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

// Include this module from the portable SDK's integration test after its
// generated-validation API is available. It uses the real SDK, not a mock.
#![allow(dead_code)]

mod generated {
    include!("../../../webui-desktop/tests/fixtures/typed-ipc/rust/ipc.rs");
}

use generated::messages::example::desktop::{Item, Label};
use webui_desktop::ipc::{Event, IpcFuture, IpcRegistry, NotificationContext, RequestContext, Rpc};

struct Handler;
impl generated::HostHandler for Handler {
    fn save(&self, _: RequestContext, _: Item) -> IpcFuture<()> {
        Box::pin(async { Ok(()) })
    }

    fn selected(&self, _: NotificationContext, _: Item) -> IpcFuture<()> {
        Box::pin(async { Ok(()) })
    }
}

#[test]
fn generated_host_trait_registers_and_empty_is_unit() {
    let mut registry = IpcRegistry::new(&generated::SCHEMA);
    assert!(generated::register_host(&mut registry, std::sync::Arc::new(Handler)).is_ok());
    assert!(generated::register_host(&mut registry, std::sync::Arc::new(Handler)).is_err());
    fn empty_response<M: Rpc<Response = ()>>() {}
    empty_response::<generated::host::Save>();
    fn renderer_response<M: Rpc<Response = Label>>() {}
    renderer_response::<generated::renderer::LabelFor>();
    fn notification<M: Event<Payload = Item>>() {}
    notification::<generated::host::Selected>();
}

#[test]
fn mixed_service_registration_preflights_before_mutating() {
    let mut registry = IpcRegistry::new(&generated::SCHEMA);
    assert!(registry
        .register_notification::<generated::host::Selected, _, _>(|_, _| async { Ok(()) })
        .is_ok());
    assert!(generated::register_host(&mut registry, std::sync::Arc::new(Handler)).is_err());
    assert!(registry
        .register::<generated::host::Save, _, _>(|_, _| async { Ok(()) })
        .is_ok());
    let mut disabled = IpcRegistry::default();
    assert!(generated::register_host(&mut disabled, std::sync::Arc::new(Handler)).is_err());
}

#[test]
fn generated_wire_guard_accepts_all_native_map_keys() {
    let limits = webui_desktop::ipc::IpcLimits::default();
    for bytes in [
        include_bytes!("../../../webui-desktop/tests/fixtures/typed-ipc/golden.bin").as_slice(),
        include_bytes!("../../../webui-desktop/tests/fixtures/typed-ipc/golden-ts.bin").as_slice(),
    ] {
        let save = generated::SCHEMA
            .methods
            .iter()
            .find(|method| method.id == 1101);
        assert!(save.is_some_and(|method| (method.validate_request)(bytes, &limits).is_ok()));
    }
}

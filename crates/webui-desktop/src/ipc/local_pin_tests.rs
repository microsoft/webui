// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

use super::*;
use std::net::TcpListener;

#[test]
fn terminal_retirement_unpins_socket_while_blocked_work_keeps_core_alive() {
    let listener = crate::bind_owned_local_server("127.0.0.1:0".parse().unwrap()).unwrap();
    let address = listener.local_addr().unwrap();
    let origin = crate::LoopbackOrigin::from_socket_addr(address).unwrap();
    let (_host_owner, lifetime) = crate::HostLifetime::new();
    let owned =
        crate::local_server::OwnedLocalServerIpc::from_listener(&listener, &origin, &lifetime)
            .unwrap();
    let owner = IpcWindowOwner::new(
        Arc::new(crate::ipc_test_support::registry()),
        crate::ipc_test_support::options(),
        IpcHost::LocalOwned(Arc::new(owned)),
    )
    .unwrap();
    // A worker may hold this strong Core reference until application code
    // returns, even after the native frame and its owner are gone.
    let blocked_work = Arc::clone(&owner.core);
    drop(listener);
    assert!(TcpListener::bind(address).is_err());
    owner.close();
    drop(owner);
    assert!(lock(&blocked_work.state).closed);
    assert!(TcpListener::bind(address).is_ok());
    drop(blocked_work);
}

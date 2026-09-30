// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

//! Retained socket identity for opt-in local application IPC.
//! The native window and HTTP server retain separate handles to the same
//! listener; an attached daemon's port or loss notification is not proof.

use std::net::TcpListener;
use std::sync::Mutex;

use super::{
    invalid_local_server, listener::ensure_exclusive_listener, HostLifetime, LoopbackOrigin,
};
use crate::{DesktopError, Result};

/// Unforgeable socket provenance retained by the IPC owner until shutdown.
/// Only `from_listener` can construct this; attached-daemon port/loss signals
/// cannot. The duplicated handle keeps the actual bound listener from rebind.
pub struct OwnedLocalServerIpc {
    pub(crate) origin: LoopbackOrigin,
    pub(crate) lifetime: HostLifetime,
    listener: Mutex<Option<TcpListener>>,
}

impl std::fmt::Debug for OwnedLocalServerIpc {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OwnedLocalServerIpc")
            .finish_non_exhaustive()
    }
}

impl OwnedLocalServerIpc {
    pub(crate) fn from_listener(
        listener: &TcpListener,
        origin: &LoopbackOrigin,
        lifetime: &HostLifetime,
    ) -> Result<Self> {
        lifetime.require_active()?;
        let address = listener.local_addr().map_err(|source| DesktopError::Io {
            context: "verifying the owned local-server IPC listener".into(),
            source,
        })?;
        if LoopbackOrigin::from_socket_addr(address)? != *origin {
            return Err(invalid_local_server(
                "IPC listener must be bound to the exact configured loopback origin",
            ));
        }
        ensure_exclusive_listener(listener)?;
        let duplicate = listener.try_clone().map_err(|source| DesktopError::Io {
            context: "retaining the owned local-server IPC listener".into(),
            source,
        })?;
        lifetime.require_active()?;
        Ok(Self {
            origin: origin.clone(),
            lifetime: lifetime.clone(),
            listener: Mutex::new(Some(duplicate)),
        })
    }

    /// Called only after the core's terminal retirement has revoked document
    /// authority under its state lock. Blocked handlers may keep Core alive;
    /// they must not pin the address past native window shutdown.
    pub(crate) fn release_listener_pin(&self) {
        self.listener
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .take();
    }
}

#[cfg(test)]
#[allow(clippy::disallowed_methods)]
mod tests {
    use super::*;
    use crate::bind_owned_local_server;
    #[cfg(target_os = "linux")]
    use std::net::SocketAddr;
    use std::sync::Arc;

    #[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
    #[test]
    fn only_the_exact_retained_owned_listener_can_enable_ipc() {
        let listener = bind_owned_local_server("127.0.0.1:0".parse().unwrap()).unwrap();
        let origin = LoopbackOrigin::from_socket_addr(listener.local_addr().unwrap()).unwrap();
        let other = TcpListener::bind("127.0.0.1:0").unwrap();
        let (owner, lifetime) = HostLifetime::new();
        let builder = || {
            crate::DesktopApp::from_local_server(super::super::LocalServerOptions::new(
                origin.clone(),
                lifetime.clone(),
            ))
        };
        assert!(builder()
            .application_ipc(
                &other,
                crate::ipc_test_support::registry(),
                crate::ipc_test_support::options(),
            )
            .is_err());
        let frame = builder()
            .application_ipc(
                &listener,
                crate::ipc_test_support::registry(),
                crate::ipc_test_support::options(),
            )
            .unwrap()
            .build()
            .unwrap();
        assert!(frame.ipc().is_some());
        assert!(frame.ipc_bridge().is_some_and(|bridge| bridge.is_enabled()));
        drop(listener);
        assert!(
            TcpListener::bind(origin.as_str().trim_start_matches("http://")).is_err(),
            "the frame must retain the actual bound socket against rebind"
        );
        owner.revoke().unwrap();
        assert!(!frame.ipc_bridge().unwrap().is_enabled() || !frame.lifetime().is_active());
        drop(frame);
    }

    #[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
    #[test]
    fn terminal_retirement_releases_pin_even_while_host_arc_survives() {
        let listener = bind_owned_local_server("127.0.0.1:0".parse().unwrap()).unwrap();
        let address = listener.local_addr().unwrap();
        let origin = LoopbackOrigin::from_socket_addr(address).unwrap();
        let (owner, lifetime) = HostLifetime::new();
        let pin =
            Arc::new(OwnedLocalServerIpc::from_listener(&listener, &origin, &lifetime).unwrap());
        let ipc = crate::ipc::IpcWindowOwner::new(
            Arc::new(crate::ipc_test_support::registry()),
            crate::ipc_test_support::options(),
            crate::ipc::IpcHost::LocalOwned(Arc::clone(&pin)),
        )
        .unwrap();
        drop(listener);
        assert!(TcpListener::bind(address).is_err());
        owner.revoke().unwrap();
        assert!(
            TcpListener::bind(address).is_err(),
            "revoke cannot unpin before native retirement"
        );
        ipc.close();
        let replacement = TcpListener::bind(address).unwrap();
        assert!(!ipc.bridge().is_enabled());
        assert!(
            Arc::strong_count(&pin) > 1,
            "core still owns an Arc after retirement"
        );
        drop(replacement);
        drop(ipc);
        assert!(Arc::strong_count(&pin) == 1);
    }

    #[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
    #[test]
    fn failed_local_ipc_build_releases_its_duplicate_listener() {
        let listener = bind_owned_local_server("127.0.0.1:0".parse().unwrap()).unwrap();
        let address = listener.local_addr().unwrap();
        let origin = LoopbackOrigin::from_socket_addr(address).unwrap();
        let (_owner, lifetime) = HostLifetime::new();
        let mut invalid = crate::ipc_test_support::options();
        invalid.worker_threads = 0;
        let built = crate::DesktopApp::from_local_server(super::super::LocalServerOptions::new(
            origin, lifetime,
        ))
        .application_ipc(&listener, crate::ipc_test_support::registry(), invalid)
        .unwrap()
        .build();
        assert!(built.is_err());
        drop(listener);
        assert!(TcpListener::bind(address).is_ok());
    }

    #[cfg(target_os = "linux")]
    #[test]
    #[allow(unsafe_code)]
    fn linux_owned_listener_rejects_reuse_and_competing_binders() {
        use socket2::{Domain, Protocol, SockAddr, Socket, Type};
        use std::os::fd::AsRawFd;
        let address = "127.0.0.1:0".parse().unwrap();
        let listener = bind_owned_local_server(address).unwrap();
        let origin = LoopbackOrigin::from_socket_addr(listener.local_addr().unwrap()).unwrap();
        let (owner, lifetime) = HostLifetime::new();
        let pin = OwnedLocalServerIpc::from_listener(&listener, &origin, &lifetime).unwrap();
        let attacker = Socket::new(Domain::IPV4, Type::STREAM, Some(Protocol::TCP)).unwrap();
        attacker.set_reuse_address(true).unwrap();
        let reuse_port: libc::c_int = 1;
        let size = std::mem::size_of_val(&reuse_port).try_into().unwrap();
        // SAFETY: The attacker socket is live; initialized option bytes stay
        // readable for the duration of this synchronous Linux syscall.
        assert_eq!(
            unsafe {
                libc::setsockopt(
                    attacker.as_raw_fd(),
                    libc::SOL_SOCKET,
                    libc::SO_REUSEPORT,
                    std::ptr::from_ref(&reuse_port).cast(),
                    size,
                )
            },
            0
        );
        assert!(attacker
            .bind(&SockAddr::from(listener.local_addr().unwrap()))
            .is_err());
        drop(listener);
        assert!(TcpListener::bind(origin.as_str().trim_start_matches("http://")).is_err());
        owner.revoke().unwrap();
        drop(pin);
        assert!(TcpListener::bind(origin.as_str().trim_start_matches("http://")).is_ok());
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn linux_reusable_listener_is_not_an_owned_ipc_capability() {
        use socket2::{Domain, Protocol, SockAddr, Socket, Type};
        let socket = Socket::new(Domain::IPV4, Type::STREAM, Some(Protocol::TCP)).unwrap();
        socket.set_reuse_address(true).unwrap();
        socket
            .bind(&SockAddr::from(
                "127.0.0.1:0".parse::<SocketAddr>().unwrap(),
            ))
            .unwrap();
        socket.listen(8).unwrap();
        let listener: TcpListener = socket.into();
        let origin = LoopbackOrigin::from_socket_addr(listener.local_addr().unwrap()).unwrap();
        let (_owner, lifetime) = HostLifetime::new();
        assert!(OwnedLocalServerIpc::from_listener(&listener, &origin, &lifetime).is_err());
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn linux_owned_ipc_rejects_a_listener_from_a_different_port() {
        let listener = bind_owned_local_server("127.0.0.1:0".parse().unwrap()).unwrap();
        let other = bind_owned_local_server("127.0.0.1:0".parse().unwrap()).unwrap();
        let origin = LoopbackOrigin::from_socket_addr(listener.local_addr().unwrap()).unwrap();
        let (_owner, lifetime) = HostLifetime::new();
        let frame = crate::DesktopApp::from_local_server(super::super::LocalServerOptions::new(
            origin, lifetime,
        ))
        .application_ipc(
            &other,
            crate::ipc_test_support::registry(),
            crate::ipc_test_support::options(),
        );
        assert!(frame.is_err());
    }
}

// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

//! Socket identity and exclusive binding for opt-in local application IPC.
//! The native window and HTTP server retain separate handles to the same
//! listener; an attached daemon's port or loss notification is not proof.

use std::net::{SocketAddr, TcpListener};
use std::sync::Mutex;

use super::{invalid_local_server, HostLifetime, LoopbackOrigin};
use crate::{DesktopError, Result};

/// Bind an exclusively owned loopback listener for local application IPC.
///
/// On Windows this sets `SO_EXCLUSIVEADDRUSE` **before** bind, unlike an
/// ordinary `TcpListener::bind`. On Linux it disables address reuse before
/// binding and verifies both reuse options on the live socket. On macOS it
/// uses a normal non-reusable listener. Never pass a daemon's port or
/// ownership-loss notification in place of the returned socket; retain it
/// until the native window closes.
///
/// # Errors
///
/// Rejects non-loopback addresses, unsupported targets, socket option and
/// binding failures.
#[allow(unsafe_code)]
pub fn bind_owned_local_server(address: SocketAddr) -> Result<TcpListener> {
    if !address.ip().is_loopback() {
        return Err(invalid_local_server(
            "owned IPC listener must bind a loopback IP",
        ));
    }
    #[cfg(target_os = "windows")]
    {
        use socket2::{Domain, Protocol, SockAddr, Socket, Type};
        use std::os::windows::io::AsRawSocket;
        use windows::Win32::Networking::WinSock::{
            setsockopt, SOCKET, SOL_SOCKET, SO_EXCLUSIVEADDRUSE,
        };
        let socket = Socket::new(
            Domain::for_address(address),
            Type::STREAM,
            Some(Protocol::TCP),
        )
        .map_err(|source| DesktopError::Io {
            context: "creating exclusive local IPC listener".into(),
            source,
        })?;
        let raw = usize::try_from(socket.as_raw_socket())
            .map_err(|_| invalid_local_server("listener socket handle is out of range"))?;
        let exclusive = 1_i32.to_ne_bytes();
        // SAFETY: The new, unbound SOCKET is owned by socket2. The initialized
        // integer bytes remain valid throughout this synchronous WinSock call.
        if unsafe {
            setsockopt(
                SOCKET(raw),
                SOL_SOCKET,
                SO_EXCLUSIVEADDRUSE,
                Some(&exclusive),
            )
        } != 0
        {
            return Err(DesktopError::Io {
                context: "setting SO_EXCLUSIVEADDRUSE before local IPC bind".into(),
                source: std::io::Error::last_os_error(),
            });
        }
        socket
            .bind(&SockAddr::from(address))
            .map_err(|source| DesktopError::Io {
                context: "binding exclusive local IPC listener".into(),
                source,
            })?;
        socket.listen(128).map_err(|source| DesktopError::Io {
            context: "listening on exclusive local IPC socket".into(),
            source,
        })?;
        let listener: TcpListener = socket.into();
        OwnedLocalServerIpc::ensure_exclusive_listener(&listener)?;
        Ok(listener)
    }
    #[cfg(target_os = "macos")]
    {
        let listener = TcpListener::bind(address).map_err(|source| DesktopError::Io {
            context: "binding owned local IPC listener".into(),
            source,
        })?;
        OwnedLocalServerIpc::ensure_exclusive_listener(&listener)?;
        Ok(listener)
    }
    #[cfg(target_os = "linux")]
    {
        use socket2::{Domain, Protocol, SockAddr, Socket, Type};
        let socket = Socket::new(
            Domain::for_address(address),
            Type::STREAM,
            Some(Protocol::TCP),
        )
        .map_err(|source| DesktopError::Io {
            context: "creating exclusive Linux local IPC listener".into(),
            source,
        })?;
        socket
            .set_reuse_address(false)
            .map_err(|source| DesktopError::Io {
                context: "disabling local IPC address reuse before bind".into(),
                source,
            })?;
        if address.is_ipv6() {
            socket
                .set_only_v6(true)
                .map_err(|source| DesktopError::Io {
                    context: "restricting the Linux local IPC listener to IPv6".into(),
                    source,
                })?;
        }
        socket
            .bind(&SockAddr::from(address))
            .map_err(|source| DesktopError::Io {
                context: "binding exclusive Linux local IPC listener".into(),
                source,
            })?;
        socket.listen(128).map_err(|source| DesktopError::Io {
            context: "listening on exclusive Linux local IPC socket".into(),
            source,
        })?;
        let listener: TcpListener = socket.into();
        OwnedLocalServerIpc::ensure_exclusive_listener(&listener)?;
        Ok(listener)
    }
    #[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
    {
        Err(invalid_local_server(
            "local application IPC is unavailable on this platform",
        ))
    }
}

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
        Self::ensure_exclusive_listener(listener)?;
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

    #[cfg(target_os = "macos")]
    #[allow(unsafe_code)]
    fn ensure_exclusive_listener(listener: &TcpListener) -> Result<()> {
        use std::os::fd::AsRawFd;
        let mut reuse_port: libc::c_int = 0;
        let mut len = std::mem::size_of_val(&reuse_port)
            .try_into()
            .map_err(|_| invalid_local_server("invalid socket option size"))?;
        // SAFETY: The listener is live; the initialized integer and length are
        // writable for the duration of getsockopt.
        let code = unsafe {
            libc::getsockopt(
                listener.as_raw_fd(),
                libc::SOL_SOCKET,
                libc::SO_REUSEPORT,
                std::ptr::from_mut(&mut reuse_port).cast(),
                &mut len,
            )
        };
        if code != 0 {
            return Err(DesktopError::Io {
                context: "checking local-server listener exclusivity".into(),
                source: std::io::Error::last_os_error(),
            });
        }
        if reuse_port != 0 || len as usize != std::mem::size_of_val(&reuse_port) {
            return Err(invalid_local_server(
                "IPC requires an exclusive owned listener without SO_REUSEPORT",
            ));
        }
        Ok(())
    }

    #[cfg(target_os = "windows")]
    #[allow(unsafe_code)]
    fn ensure_exclusive_listener(listener: &TcpListener) -> Result<()> {
        use std::os::windows::io::AsRawSocket;
        use windows::Win32::Networking::WinSock::{
            getsockopt, SOCKET, SOL_SOCKET, SO_EXCLUSIVEADDRUSE, SO_REUSEADDR,
        };
        let socket = usize::try_from(listener.as_raw_socket())
            .map_err(|_| invalid_local_server("listener socket handle is out of range"))?;
        let option = |name| -> Result<i32> {
            let mut value: i32 = 0;
            let expected_len = i32::try_from(std::mem::size_of_val(&value))
                .map_err(|_| invalid_local_server("invalid socket option size"))?;
            let mut len = expected_len;
            // SAFETY: The live SOCKET was supplied by the host. The mutable value
            // buffer and length are valid throughout the synchronous WinSock call.
            let code = unsafe {
                getsockopt(
                    SOCKET(socket),
                    SOL_SOCKET,
                    name,
                    windows::core::PSTR(std::ptr::from_mut(&mut value).cast()),
                    &mut len,
                )
            };
            if code != 0 {
                return Err(DesktopError::Io {
                    context: "checking local-server listener exclusivity".into(),
                    source: std::io::Error::last_os_error(),
                });
            }
            if len != expected_len {
                return Err(invalid_local_server(
                    "invalid local-server socket option length",
                ));
            }
            Ok(value)
        };
        if option(SO_EXCLUSIVEADDRUSE)? != 1 || option(SO_REUSEADDR)? != 0 {
            return Err(invalid_local_server(
                "IPC requires a listener bound with SO_EXCLUSIVEADDRUSE and without SO_REUSEADDR",
            ));
        }
        Ok(())
    }

    #[cfg(target_os = "linux")]
    #[allow(unsafe_code)]
    fn ensure_exclusive_listener(listener: &TcpListener) -> Result<()> {
        use std::os::fd::AsRawFd;
        let option = |name| -> Result<libc::c_int> {
            let mut value: libc::c_int = 0;
            let mut len = std::mem::size_of_val(&value)
                .try_into()
                .map_err(|_| invalid_local_server("invalid Linux socket option size"))?;
            // SAFETY: The bound listener owns this live fd; the initialized
            // integer and length remain writable throughout getsockopt.
            let status = unsafe {
                libc::getsockopt(
                    listener.as_raw_fd(),
                    libc::SOL_SOCKET,
                    name,
                    std::ptr::from_mut(&mut value).cast(),
                    &mut len,
                )
            };
            if status != 0 {
                return Err(DesktopError::Io {
                    context: "checking Linux local IPC listener exclusivity".into(),
                    source: std::io::Error::last_os_error(),
                });
            }
            if usize::try_from(len).ok() != Some(std::mem::size_of_val(&value)) {
                return Err(invalid_local_server(
                    "invalid Linux socket option result length",
                ));
            }
            Ok(value)
        };
        if option(libc::SO_REUSEPORT)? != 0 || option(libc::SO_REUSEADDR)? != 0 {
            return Err(invalid_local_server(
                "IPC requires an exclusive Linux listener without SO_REUSEPORT or SO_REUSEADDR",
            ));
        }
        Ok(())
    }

    #[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
    fn ensure_exclusive_listener(_listener: &TcpListener) -> Result<()> {
        Err(invalid_local_server(
            "local-server IPC requires exclusive macOS or Windows listener proof",
        ))
    }
}

#[cfg(test)]
#[allow(clippy::disallowed_methods)]
mod tests {
    use super::*;
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

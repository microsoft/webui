// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

//! Exclusive loopback binding shared by plain HTTP hosts and opt-in IPC.

use std::net::{SocketAddr, TcpListener};

use super::invalid_local_server;
use crate::{DesktopError, Result};

/// Bind an exclusively owned loopback listener for a local-server window.
///
/// Available with `local-server` alone; binding does not enable application IPC.
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
        ensure_exclusive_listener(&listener)?;
        Ok(listener)
    }
    #[cfg(target_os = "macos")]
    {
        let listener = TcpListener::bind(address).map_err(|source| DesktopError::Io {
            context: "binding owned local IPC listener".into(),
            source,
        })?;
        ensure_exclusive_listener(&listener)?;
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
        ensure_exclusive_listener(&listener)?;
        Ok(listener)
    }
    #[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
    {
        Err(invalid_local_server(
            "local application IPC is unavailable on this platform",
        ))
    }
}

#[cfg(target_os = "macos")]
#[allow(unsafe_code)]
pub(super) fn ensure_exclusive_listener(listener: &TcpListener) -> Result<()> {
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
pub(super) fn ensure_exclusive_listener(listener: &TcpListener) -> Result<()> {
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
pub(super) fn ensure_exclusive_listener(listener: &TcpListener) -> Result<()> {
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

#[cfg(all(
    feature = "application-ipc",
    not(any(target_os = "macos", target_os = "windows", target_os = "linux"))
))]
pub(super) fn ensure_exclusive_listener(_listener: &TcpListener) -> Result<()> {
    Err(invalid_local_server(
        "local-server IPC requires exclusive macOS or Windows listener proof",
    ))
}

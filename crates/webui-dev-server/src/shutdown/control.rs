// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

use super::{error::protocol_error, Error, CHILD_ENV, CHILD_VERSION};
use actix_web::dev::Server;
use std::io::{self, Read};
use std::thread;
use tokio::sync::watch;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum State {
    Running,
    Stop,
    Disconnected,
    Invalid,
    ReadError(io::ErrorKind),
}

/// Child-side HTTP stop bridge. This does not own the watcher or rebuild worker.
pub struct Control(pub(super) watch::Receiver<State>);

impl Control {
    /// Serve with Actix signals disabled until HTTP exits or the parent stops it.
    ///
    /// Stops active HTTP connections with `handle.stop(false)` and awaits the
    /// server. The caller must then drop its watcher and join its rebuild worker,
    /// **even on error**, before returning [`super::JOINED_EXIT_CODE`] on success.
    pub async fn serve(mut self, server: Server) -> Result<(), Error> {
        let handle = server.handle();
        tokio::pin!(server);
        tokio::select! {
            biased;
            reason = self.stop_reason() => {
                // Server itself must keep polling to process the stop command;
                // awaiting the handle alone can deadlock its acknowledgement.
                let (_, result) = futures_util::future::join(handle.stop(false), &mut server).await;
                result.map_err(Error::Http)?;
                control_result(reason)
            }
            result = &mut server => result.map_err(Error::Http),
        }
    }

    async fn stop_reason(&mut self) -> State {
        loop {
            let state = *self.0.borrow_and_update();
            if state != State::Running {
                return state;
            }
            if self.0.changed().await.is_err() {
                return State::Disconnected;
            }
        }
    }
}

pub(super) fn child_gate() -> Result<Option<Control>, Error> {
    let Some(version) = std::env::var_os(CHILD_ENV) else {
        return Ok(None);
    };
    // prepare's entry-point contract forbids application threads at this point.
    // Clear even an invalid marker so no later build command can inherit it.
    std::env::remove_var(CHILD_ENV);
    if version != CHILD_VERSION {
        return Err(Error::Startup(protocol_error(
            "unsupported child startup protocol",
        )));
    }
    let mut start = [0_u8; 1];
    io::stdin().read_exact(&mut start).map_err(Error::Startup)?;
    if start != *b"G" {
        return Err(Error::Startup(protocol_error(
            "missing child startup permission",
        )));
    }
    let (sender, receiver) = watch::channel(State::Running);
    thread::Builder::new()
        .name("webui-shutdown-control".to_owned())
        .spawn(move || {
            let state = read_stop(&mut io::stdin());
            // Receiver disposal means HTTP serving has already returned. This
            // dedicated child's reader never owns or detaches any rebuild work.
            let _ = sender.send(state);
        })
        .map_err(Error::Control)?;
    Ok(Some(Control(receiver)))
}

fn read_stop(reader: &mut impl Read) -> State {
    let mut command = [0_u8; 1];
    match reader.read_exact(&mut command) {
        Ok(()) if command == *b"S" => State::Stop,
        Ok(()) => State::Invalid,
        Err(error) if error.kind() == io::ErrorKind::UnexpectedEof => State::Disconnected,
        Err(error) => State::ReadError(error.kind()),
    }
}

#[cold]
#[inline(never)]
fn control_result(state: State) -> Result<(), Error> {
    match state {
        State::Stop => Ok(()),
        State::Disconnected => Err(Error::Control(io::Error::new(
            io::ErrorKind::UnexpectedEof,
            "supervisor closed the control pipe",
        ))),
        State::ReadError(kind) => Err(Error::Control(io::Error::new(
            kind,
            "could not read the supervisor control pipe",
        ))),
        State::Running | State::Invalid => Err(Error::Control(protocol_error(
            "invalid supervisor stop command",
        ))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn control_protocol_distinguishes_stop_disconnect_and_invalid_data() {
        assert_eq!(read_stop(&mut &b"S"[..]), State::Stop);
        assert_eq!(read_stop(&mut &b""[..]), State::Disconnected);
        assert_eq!(read_stop(&mut &b"X"[..]), State::Invalid);
        assert!(control_result(State::Stop).is_ok());
        for state in [
            State::Disconnected,
            State::Invalid,
            State::ReadError(io::ErrorKind::Other),
        ] {
            assert!(matches!(control_result(state), Err(Error::Control(_))));
        }
    }
}

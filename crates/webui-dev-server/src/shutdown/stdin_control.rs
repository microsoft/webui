// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

use super::notification::{Notification, Requests};
use super::task::Task;
use super::{worker, Error};
use std::io::{self, Read};

/// One process-lifetime reader, never joined while it could be waiting for the
/// external controller. Completing a worker must not require controller EOF.
pub(super) struct StdinControl {
    reader: Option<Task<io::Result<()>>>,
    failure: Option<Error>,
}

impl StdinControl {
    pub(super) fn start(enabled: bool, notification: Notification) -> Result<Self, Error> {
        let reader = if enabled {
            Some(
                Task::spawn("webui-shutdown-stdin", notification, read_request)
                    .map_err(Error::Control)?,
            )
        } else {
            None
        };
        Ok(Self {
            reader,
            failure: None,
        })
    }

    pub(super) fn poll(&mut self, requests: &Requests) {
        let Some(reader) = self.reader.as_mut() else {
            return;
        };
        let result = match reader.take_result() {
            Ok(Some(result)) => result,
            Ok(None) => return,
            Err(error) => Err(error),
        };
        // Even malformed input requests cooperative cleanup first. The error
        // remains owned here until the worker and its scope have been joined.
        self.failure = result.err().map(|error| reader.failure(error));
        requests.request();
    }

    pub(super) fn resolve(self, result: Result<i32, Error>) -> Result<i32, Error> {
        match (self.failure, result) {
            (None, result) => result,
            (Some(error), Ok(_)) => Err(error),
            (Some(error), Err(cleanup)) => Err(worker::combined(error, cleanup)),
        }
    }
}

fn read_request() -> io::Result<()> {
    let mut byte = [0_u8; 1];
    match io::stdin().read_exact(&mut byte) {
        Ok(()) if byte == *b"S" => Ok(()),
        Ok(()) => Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!(
                "invalid supervisor stdin control byte 0x{:02x}; send ASCII S or close stdin to stop",
                byte[0]
            ),
        )),
        Err(error) if error.kind() == io::ErrorKind::UnexpectedEof => Ok(()),
        Err(error) => Err(error),
    }
}

// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

use std::io::{self, BufRead, BufReader, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use anyhow::{anyhow, Context, Result};

pub struct Backend {
    pub port: u16,
    requests: mpsc::Receiver<String>,
    stopping: Arc<AtomicBool>,
    worker: Option<JoinHandle<Result<()>>>,
}

impl Backend {
    pub fn start(responses: Vec<(u16, &'static str)>) -> Result<Self> {
        let listener = TcpListener::bind(("127.0.0.1", 0))?;
        let port = listener.local_addr()?.port();
        listener.set_nonblocking(true)?;
        let stopping = Arc::new(AtomicBool::new(false));
        let cancel = Arc::clone(&stopping);
        let (sender, requests) = mpsc::channel();
        let worker = thread::spawn(move || {
            for (status, body) in responses {
                let Some(stream) = accept(&listener, &cancel)? else {
                    return Ok(());
                };
                stream.set_read_timeout(Some(Duration::from_secs(5)))?;
                stream.set_write_timeout(Some(Duration::from_secs(5)))?;
                let mut reader = BufReader::new(stream);
                let mut line = String::new();
                reader.read_line(&mut line)?;
                sender.send(line.trim().to_owned())?;
                loop {
                    line.clear();
                    reader.read_line(&mut line)?;
                    anyhow::ensure!(!line.is_empty(), "backend request truncated");
                    if line == "\r\n" {
                        break;
                    }
                }
                write!(
                    reader.get_mut(),
                    "HTTP/1.1 {status} Response\r\nContent-Type: application/json\r\n\
                     Content-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                )?;
                reader.get_mut().flush()?;
            }
            Ok(())
        });
        Ok(Self {
            port,
            requests,
            stopping,
            worker: Some(worker),
        })
    }

    pub fn expect_request(&mut self, target: &str) -> Result<()> {
        let received = self.requests.recv_timeout(Duration::from_secs(5));
        if matches!(received, Err(mpsc::RecvTimeoutError::Disconnected)) {
            self.finish()
                .with_context(|| format!("backend worker failed before request {target}"))?;
        }
        let line = received
            .with_context(|| format!("backend did not receive state acquisition for {target}"))?;
        assert_eq!(line, format!("GET {target} HTTP/1.1"));
        Ok(())
    }

    pub fn finish(&mut self) -> Result<()> {
        self.stopping.store(true, Ordering::Relaxed);
        if let Some(worker) = self.worker.take() {
            worker
                .join()
                .map_err(|_| anyhow!("native backend worker panicked"))??;
        }
        Ok(())
    }
}

impl Drop for Backend {
    fn drop(&mut self) {
        let _ = self.finish();
    }
}

fn accept(listener: &TcpListener, stopping: &AtomicBool) -> Result<Option<TcpStream>> {
    loop {
        if stopping.load(Ordering::Relaxed) {
            return Ok(None);
        }
        match listener.accept() {
            Ok((stream, _)) => {
                // Winsock inherits the listener's nonblocking mode on accepted sockets.
                stream.set_nonblocking(false)?;
                return Ok(Some(stream));
            }
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                thread::sleep(Duration::from_millis(10));
            }
            Err(error) => return Err(error.into()),
        }
    }
}

#[test]
fn backend_accept_waits_for_request_bytes_on_windows_too() -> Result<()> {
    let listener = TcpListener::bind(("127.0.0.1", 0))?;
    listener.set_nonblocking(true)?;
    let mut client = TcpStream::connect(listener.local_addr()?)?;
    let stream =
        accept(&listener, &AtomicBool::new(false))?.context("accepted backend connection")?;
    stream.set_read_timeout(Some(Duration::from_secs(5)))?;
    let (sender, result) = mpsc::channel();
    let worker = thread::spawn(move || {
        let mut request = String::new();
        let read = BufReader::new(stream).read_line(&mut request);
        let _ = sender.send(read.map(|_| request));
    });
    let early = result.recv_timeout(Duration::from_millis(100));
    client.write_all(b"GET /delayed HTTP/1.1\r\n")?;
    worker
        .join()
        .map_err(|_| anyhow!("backend request reader panicked"))?;
    assert!(
        matches!(early, Err(mpsc::RecvTimeoutError::Timeout)),
        "accepted socket must wait for bytes, not fail with WouldBlock: {early:?}"
    );
    assert_eq!(
        result.recv_timeout(Duration::from_secs(5))??,
        "GET /delayed HTTP/1.1\r\n"
    );
    Ok(())
}

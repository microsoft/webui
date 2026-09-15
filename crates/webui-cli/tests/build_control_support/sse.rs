// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

use std::io::{self, BufRead, BufReader, Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::time::Duration;

use anyhow::{bail, Context, Result};

pub struct Events {
    reader: BufReader<TcpStream>,
    pending: String,
    pub headers: String,
}

impl Events {
    pub fn connect(address: SocketAddr) -> Result<Self> {
        let mut stream = TcpStream::connect_timeout(&address, Duration::from_secs(5))?;
        stream.set_read_timeout(Some(Duration::from_secs(5)))?;
        stream.set_write_timeout(Some(Duration::from_secs(5)))?;
        write!(
            stream,
            "GET /__webui/livereload HTTP/1.1\r\nHost: {address}\r\nAccept: text/event-stream\r\n\r\n"
        )?;
        stream.flush()?;
        let mut events = Self {
            reader: BufReader::new(stream),
            pending: String::new(),
            headers: String::new(),
        };
        let mut line = String::new();
        events.reader.read_line(&mut line)?;
        assert!(line.starts_with("HTTP/1.1 200"), "{line}");
        loop {
            line.clear();
            events.reader.read_line(&mut line)?;
            if line == "\r\n" {
                break;
            }
            anyhow::ensure!(!line.is_empty(), "SSE headers truncated");
            events.headers.push_str(&line.to_ascii_lowercase());
        }
        assert!(events.headers.contains("text/event-stream"));
        assert_eq!(events.frame()?, ": connected");
        Ok(events)
    }

    fn frame(&mut self) -> Result<String> {
        loop {
            if let Some(end) = self.pending.find("\n\n") {
                let rest = self.pending.split_off(end + 2);
                let frame = std::mem::replace(&mut self.pending, rest);
                return Ok(frame[..end].to_owned());
            }
            let mut line = String::new();
            self.reader.read_line(&mut line)?;
            let size = usize::from_str_radix(line.trim(), 16).context("SSE HTTP chunk size")?;
            anyhow::ensure!(size > 0, "SSE stream unexpectedly closed");
            let mut chunk = vec![0; size];
            self.reader.read_exact(&mut chunk)?;
            self.pending.push_str(std::str::from_utf8(&chunk)?);
            let mut ending = [0; 2];
            self.reader.read_exact(&mut ending)?;
            assert_eq!(&ending, b"\r\n");
        }
    }

    pub fn expect(&mut self, name: &str) -> Result<()> {
        let frame = self.frame()?;
        assert!(
            frame.starts_with(&format!("event: {name}\n")),
            "expected {name}: {frame}"
        );
        Ok(())
    }

    pub fn quiet(&mut self) -> Result<()> {
        assert!(
            self.pending.is_empty(),
            "unexpected SSE data: {}",
            self.pending
        );
        self.reader
            .get_ref()
            .set_read_timeout(Some(Duration::from_millis(200)))?;
        let result = match self.reader.fill_buf() {
            Err(error)
                if matches!(
                    error.kind(),
                    io::ErrorKind::TimedOut | io::ErrorKind::WouldBlock
                ) =>
            {
                Ok(())
            }
            other => bail!("unexpected SSE activity: {other:?}"),
        };
        self.reader
            .get_ref()
            .set_read_timeout(Some(Duration::from_secs(5)))?;
        result
    }
}

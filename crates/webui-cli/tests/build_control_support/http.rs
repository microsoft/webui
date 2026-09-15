// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

use std::collections::HashMap;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::thread;
use std::time::{Duration, Instant};

use anyhow::{Context, Result};

pub struct Response {
    pub status: u16,
    pub headers: HashMap<String, String>,
    pub body: String,
}

impl Response {
    pub fn header(&self, name: &str) -> &str {
        self.headers.get(name).map_or("", String::as_str)
    }
}

pub fn request(address: SocketAddr, path: &str, accept: &str) -> Result<Response> {
    let mut stream = TcpStream::connect_timeout(&address, Duration::from_secs(5))?;
    stream.set_read_timeout(Some(Duration::from_secs(5)))?;
    stream.set_write_timeout(Some(Duration::from_secs(5)))?;
    write!(
        stream,
        "GET {path} HTTP/1.1\r\nHost: {address}\r\nAccept: {accept}\r\nConnection: close\r\n\r\n"
    )?;
    stream.flush()?;
    let mut reader = BufReader::new(stream);
    let mut line = String::new();
    reader.read_line(&mut line)?;
    let status = line
        .split_whitespace()
        .nth(1)
        .context("HTTP status line")?
        .parse()?;
    let mut headers = HashMap::new();
    loop {
        line.clear();
        reader.read_line(&mut line)?;
        if line == "\r\n" {
            break;
        }
        let (name, value) = line.split_once(':').context("HTTP response header")?;
        headers.insert(name.to_ascii_lowercase(), value.trim().to_owned());
    }
    let mut body = Vec::new();
    if headers
        .get("transfer-encoding")
        .is_some_and(|value| value.eq_ignore_ascii_case("chunked"))
    {
        loop {
            line.clear();
            reader.read_line(&mut line)?;
            let size = usize::from_str_radix(
                line.trim().split(';').next().context("HTTP chunk size")?,
                16,
            )?;
            if size == 0 {
                break;
            }
            let offset = body.len();
            body.resize(offset + size, 0);
            reader.read_exact(&mut body[offset..])?;
            let mut ending = [0; 2];
            reader.read_exact(&mut ending)?;
            assert_eq!(&ending, b"\r\n");
        }
    } else {
        reader.read_to_end(&mut body)?;
    }
    Ok(Response {
        status,
        headers,
        body: String::from_utf8(body)?,
    })
}

pub fn assert_gate(address: SocketAddr, status: u16) -> Result<()> {
    for (path, accept) in [
        ("/", "text/html"),
        ("/", "application/json"),
        ("/client.js", "*/*"),
        ("/test-card.css", "*/*"),
        ("/_webui/templates?t=test-card", "application/json"),
    ] {
        let response = request(address, path, accept)?;
        assert_eq!(response.status, status, "{path}: {}", response.body);
        assert!(
            response.header("cache-control").contains("no-store"),
            "{path}"
        );
    }
    Ok(())
}

pub fn wait_status(address: SocketAddr, expected: u16) -> Result<()> {
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        let response = request(address, "/", "text/html")?;
        if response.status == expected {
            return Ok(());
        }
        anyhow::ensure!(
            Instant::now() < deadline,
            "waiting for HTTP {expected}, got {}: {}",
            response.status,
            response.body
        );
        thread::sleep(Duration::from_millis(20));
    }
}

pub fn document_nonce(response: &Response) -> Result<&str> {
    let policy = response.header("content-security-policy");
    let tail = policy.split_once("'nonce-").context("CSP nonce source")?.1;
    let nonce = tail.split_once('\'').context("CSP nonce terminator")?.0;
    assert!(nonce.len() >= 32, "nonce must carry at least 128 bits");
    assert!(!policy.contains("{nonce}"));
    let expected = format!("nonce=\"{nonce}\"");
    let mut scripts = 0;
    for script in response.body.split("<script").skip(1) {
        let tag = script.split_once('>').context("script tag closing")?.0;
        if !tag.contains("src=") {
            assert!(tag.contains(&expected), "SDK nonce mismatch: <script{tag}>");
            scripts += 1;
        }
    }
    assert!(scripts > 0, "SDK scripts were not emitted");
    assert_eq!(response.body.matches("new EventSource(").count(), 1);
    Ok(nonce)
}

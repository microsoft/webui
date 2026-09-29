// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

use std::fs;
use std::io::{Read, Write};
use std::net::{Ipv4Addr, TcpListener, TcpStream};
use std::process::{Child, Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

struct RunningServer(Child);

impl Drop for RunningServer {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn request(
    port: u16,
    host: Option<&str>,
    path: &str,
    extra_headers: &str,
) -> std::io::Result<String> {
    let mut stream = TcpStream::connect((Ipv4Addr::LOCALHOST, port))?;
    stream.set_read_timeout(Some(Duration::from_secs(5)))?;
    write!(stream, "GET {path} HTTP/1.1\r\n")?;
    if let Some(host) = host {
        write!(stream, "Host: {host}\r\n")?;
    }
    write!(stream, "{extra_headers}Connection: close\r\n\r\n")?;
    let mut response = String::new();
    stream.read_to_string(&mut response)?;
    Ok(response)
}

#[test]
fn serve_rejects_foreign_hosts_before_routes() -> Result<(), Box<dyn std::error::Error>> {
    let temp = tempfile::tempdir()?;
    let app = temp.path().join("app");
    let assets = temp.path().join("assets");
    fs::create_dir_all(&app)?;
    fs::create_dir_all(&assets)?;
    fs::write(
        app.join("index.html"),
        "<html><body>host check</body></html>",
    )?;
    fs::write(app.join("state.json"), "{}")?;
    fs::write(assets.join("site.css"), "body { color: black; }")?;

    let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0))?;
    let port = listener.local_addr()?.port();
    drop(listener);

    let log = fs::File::create(temp.path().join("server.log"))?;
    let child = Command::new(env!("CARGO_BIN_EXE_webui"))
        .arg("serve")
        .arg(&app)
        .arg("--state")
        .arg(app.join("state.json"))
        .arg("--servedir")
        .arg(&assets)
        .arg("--port")
        .arg(port.to_string())
        .arg("--allowed-host")
        .arg("preview.example:443")
        .stdout(Stdio::from(log.try_clone()?))
        .stderr(Stdio::from(log))
        .spawn()?;
    let mut server = RunningServer(child);

    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        if TcpStream::connect((Ipv4Addr::LOCALHOST, port)).is_ok() {
            break;
        }
        if server.0.try_wait()?.is_some() || Instant::now() >= deadline {
            let output = fs::read_to_string(temp.path().join("server.log"))?;
            panic!("dev server did not start: {output}");
        }
        thread::sleep(Duration::from_millis(25));
    }

    for host in [
        format!("127.0.0.1:{port}"),
        format!("localhost:{port}"),
        format!("play.xbox.localhost:{port}"),
        "preview.example:443".to_string(),
    ] {
        let response = request(port, Some(&host), "/", "")?;
        assert!(response.starts_with("HTTP/1.1 200"), "{host}: {response}");
    }

    for (host, path, headers) in [
        (format!("attacker.example:{port}"), "/", ""),
        (format!("attacker.example:{port}"), "/index.html", ""),
        (format!("attacker.example:{port}"), "/site.css", ""),
        (
            format!("attacker.example:{port}"),
            "/",
            "Accept: application/json\r\n",
        ),
        (format!("attacker.example:{port}"), "/api/action", ""),
        (format!("localhost.evil.example:{port}"), "/", ""),
        ("preview.example:3000".to_string(), "/", ""),
        (format!("localhost:{}", port.saturating_sub(1)), "/", ""),
    ] {
        let response = request(port, Some(&host), path, headers)?;
        assert!(
            response.starts_with("HTTP/1.1 400"),
            "{host} {path}: {response}"
        );
        assert!(!response.contains("host check"));
    }
    let response = request(port, None, "/", "")?;
    assert!(response.starts_with("HTTP/1.1 400"), "{response}");
    Ok(())
}

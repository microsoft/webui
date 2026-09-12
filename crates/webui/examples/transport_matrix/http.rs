// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

use std::sync::{mpsc as control, Arc};
use std::thread;
use std::time::Instant;

use actix_web::{dev::ServerHandle, web, App, HttpResponse, HttpServer};
use bytes::Bytes;
use futures_util::StreamExt;
use tokio::sync::mpsc;

use super::consumer::service_time;
use super::workload::{Reference, Workload};
use super::{Config, Pool, Result};

struct ServerState {
    config: Config,
    workload: Workload,
    pool: Pool,
}

async fn response(state: web::Data<ServerState>) -> HttpResponse {
    let (tx, rx) = mpsc::channel::<Bytes>(super::QUEUE_SLOTS);
    actix_web::rt::task::spawn_blocking(move || {
        let mut writer = state.config.writer(tx, &state.pool);
        if let Err(error) = state.workload.write(&mut writer) {
            eprintln!("HTTP benchmark producer failed: {error}");
        }
    });
    let stream = tokio_stream::wrappers::ReceiverStream::new(rx).map(Ok::<Bytes, actix_web::Error>);
    HttpResponse::Ok()
        .content_type("text/html; charset=utf-8")
        .streaming(stream)
}

struct Server {
    url: String,
    handle: ServerHandle,
    thread: thread::JoinHandle<std::io::Result<()>>,
}

fn start_server(config: &Config, workload: Workload) -> Result<Server> {
    let listener = std::net::TcpListener::bind("127.0.0.1:0")?;
    let port = listener.local_addr()?.port();
    let (handle_tx, handle_rx) = control::sync_channel(1);
    let state = web::Data::new(ServerState {
        config: config.clone(),
        workload,
        pool: config.pool(),
    });
    let thread = thread::spawn(move || {
        actix_web::rt::System::new().block_on(async move {
            let server = HttpServer::new(move || {
                App::new()
                    .app_data(state.clone())
                    .route("/", web::get().to(response))
            })
            .disable_signals()
            .workers(1)
            .tcp_nodelay(true)
            .on_connect(|connection, _| assert_nodelay(connection))
            .listen(listener)?
            .run();
            if handle_tx.send(server.handle()).is_err() {
                server.handle().stop(false).await;
                return Ok(());
            }
            server.await
        })
    });
    Ok(Server {
        url: format!("http://127.0.0.1:{port}/"),
        handle: handle_rx.recv()?,
        thread,
    })
}

#[derive(Default, serde::Serialize)]
struct Request {
    header_ns: u128,
    first_body_ns: u128,
    drained_ns: u128,
    bytes: usize,
}

async fn request(
    client: &awc::Client,
    url: &str,
    reference: &Reference,
    config: &Config,
    timed: bool,
) -> Result<Request> {
    let start = timed.then(Instant::now);
    let mut response = client.get(url).send().await.map_err(|e| e.to_string())?;
    if response.status() != actix_web::http::StatusCode::OK {
        return Err(format!("unexpected HTTP status {}", response.status()).into());
    }
    let mut result = Request::default();
    if let Some(start) = start {
        result.header_ns = start.elapsed().as_nanos();
    }
    let mut pace_start = None;
    while let Some(chunk) = response.next().await {
        let chunk = chunk.map_err(|e| e.to_string())?;
        if chunk.is_empty() {
            continue;
        }
        if result.bytes == 0 {
            if let Some(start) = start {
                result.first_body_ns = start.elapsed().as_nanos();
            }
            if config.bytes_per_second != 0 {
                pace_start = Some(Instant::now());
            }
        }
        let end = result.bytes + chunk.len();
        if end > reference.bytes.len() {
            return Err("HTTP response has unexpected trailing bytes".into());
        }
        if !timed && chunk.as_ref() != &reference.bytes[result.bytes..end] {
            return Err("HTTP output differs from reference".into());
        }
        result.bytes = end;
        if let Some(start) = pace_start {
            let deadline = start + service_time(end, config.bytes_per_second);
            tokio::time::sleep_until(deadline.into()).await;
        }
    }
    if result.bytes != reference.bytes.len() {
        return Err("HTTP body ended before expected output".into());
    }
    if let Some(start) = start {
        result.drained_ns = start.elapsed().as_nanos();
    }
    Ok(result)
}

#[derive(serde::Serialize)]
struct Sample<'a> {
    config: &'a Config,
    tcp_nodelay: bool,
    sample: usize,
    output_sha256: &'a str,
    user_cpu_ns: u128,
    system_cpu_ns: u128,
    requests: Vec<Request>,
}

async fn requests(
    server: &Server,
    reference: &Reference,
    config: &Config,
    timed: bool,
) -> Result<()> {
    let client = awc::Client::default();
    for _ in 0..super::WARMUPS {
        request(&client, &server.url, reference, config, false).await?;
    }
    if !timed {
        println!(
            "HTTP smoke passed: {} bytes sha256={}",
            reference.bytes.len(),
            reference.sha256()
        );
        return Ok(());
    }
    let sha256 = reference.sha256();
    for sample in 0..config.samples {
        let mut requests = Vec::with_capacity(config.iterations);
        let before = crate::ProcessUsage::now();
        for _ in 0..config.iterations {
            requests.push(request(&client, &server.url, reference, config, true).await?);
        }
        let after = crate::ProcessUsage::now();
        let row = Sample {
            config,
            tcp_nodelay: true,
            sample,
            output_sha256: &sha256,
            user_cpu_ns: (after.user_cpu - before.user_cpu).as_nanos(),
            system_cpu_ns: (after.sys_cpu - before.sys_cpu).as_nanos(),
            requests,
        };
        println!("{}", serde_json::to_string(&row)?);
    }
    Ok(())
}

pub(super) fn run(config: Config, workload: Workload, timed: bool) -> Result<()> {
    let reference = Arc::new(workload.reference()?);
    let server = start_server(&config, workload)?;
    let result = actix_web::rt::System::new().block_on(async {
        let result = requests(&server, &reference, &config, timed).await;
        server.handle.stop(true).await;
        result
    });
    server.thread.join().map_err(|_| "HTTP server panicked")??;
    result
}

fn assert_nodelay(connection: &dyn std::any::Any) {
    let Some(socket) = connection.downcast_ref::<actix_web::rt::net::TcpStream>() else {
        panic!("HTTP benchmark expected a plaintext TCP socket");
    };
    assert!(
        matches!(socket.nodelay(), Ok(true)),
        "HTTP benchmark requires TCP_NODELAY"
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn connection_check_rejects_nagle_and_accepts_nodelay() -> Result<()> {
        actix_web::rt::System::new().block_on(async {
            let listener = std::net::TcpListener::bind("127.0.0.1:0")?;
            let stream = std::net::TcpStream::connect(listener.local_addr()?)?;
            stream.set_nonblocking(true)?;
            let stream = actix_web::rt::net::TcpStream::from_std(stream)?;
            stream.set_nodelay(false)?;
            assert!(std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                assert_nodelay(&stream);
            }))
            .is_err());
            stream.set_nodelay(true)?;
            assert_nodelay(&stream);
            Ok(())
        })
    }
}

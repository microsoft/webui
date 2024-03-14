use btjs_parser::parser::{handle_btr, ServerHandler};
use btjs_parser::protocol::{load_protocol_from_file, BuildTimeRenderingProtocol};
use tokio::fs::read;

use std::collections::HashMap;
use std::convert::Infallible;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};

use http_body_util::Full;
use hyper::body::{Bytes, Incoming};
use hyper::header::CONTENT_TYPE;
use hyper::server::conn::http1;
use hyper::service::service_fn;
use hyper::{Method, Request, Response, StatusCode};
use hyper_util::rt::TokioIo;
use mime_guess::from_path;
use serde_json::Value;
use tokio::net::TcpListener;

struct ResponseServerHandler {
    response: Vec<u8>,
}

// TODO: Change to channel implementation
impl ServerHandler for ResponseServerHandler {
    fn write(&mut self, value: &str) {
        self.response.extend_from_slice(value.as_bytes());
    }

    fn end(&mut self) {}
}

struct BTRServer {
    addr: SocketAddr,
    handlers: Arc<Mutex<HashMap<String, (BuildTimeRenderingProtocol, Value)>>>,
    app_path: String,
}

impl BTRServer {
    fn new(addr: SocketAddr, app_path: String) -> Self {
        BTRServer {
            addr,
            handlers: Arc::new(Mutex::new(HashMap::new())),
            app_path,
        }
    }

    fn add_handler(&mut self, method: Method, path: &str, protocol: &str, state: Value) {
        let key = format!("{}:{}", method, path);
        let protocol = load_protocol_from_file(protocol);
        if let Ok(protocol) = protocol {
            self.handlers.lock().unwrap().insert(key, (protocol, state));
        } else {
            println!("Error loading protocol: {:?}", protocol.err());
        }
    }

    async fn handle_request(
        handlers: Arc<Mutex<HashMap<String, (BuildTimeRenderingProtocol, Value)>>>,
        req: Request<Incoming>,
        app_path: String,
    ) -> Result<Response<Full<Bytes>>, Infallible> {
        let start = std::time::Instant::now();
        let key = format!("{}:{}", req.method(), req.uri().path());

        let handler = {
            let handlers = handlers.lock().unwrap();
            handlers.get(&key).cloned()
        };

        if let Some((protocol, state)) = handler {
            let mut server_handler = ResponseServerHandler {
                response: Vec::new(),
            };
            handle_btr(protocol.clone(), state.clone(), &mut server_handler);
            let duration = start.elapsed();
            println!(
                "{}:{}: {}ms",
                req.method(),
                req.uri().path(),
                duration.as_secs_f64() * 1000.0
            );

            Ok(Response::new(Full::new(Bytes::from(
                server_handler.response,
            ))))
        } else {
            let path = format!("{}/{}", app_path, req.uri().path().trim_start_matches('/'));
            let path_clone = path.clone();
            match read(path).await {
                Ok(data) => {
                    let mime_type: String =
                        from_path(&path_clone).first_or_octet_stream().to_string();
                    Ok(Response::builder()
                        .header(CONTENT_TYPE, mime_type)
                        .body(Full::new(Bytes::from(data)))
                        .unwrap())
                }
                Err(_) => Ok(Response::builder()
                    .status(StatusCode::NOT_FOUND)
                    .body(Full::new(Bytes::from("File not found")))
                    .unwrap()),
            }
        }
    }

    async fn start(&self) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let listener = TcpListener::bind(&self.addr).await?;
        let handlers = Arc::clone(&self.handlers);
        let app_path = self.app_path.clone();

        loop {
            let (stream, _) = listener.accept().await?;
            let io = TokioIo::new(stream);
            let http = http1::Builder::new();
            let handlers_clone = Arc::clone(&handlers);
            let app_path_clone = app_path.clone();
            let serve_conn = http.serve_connection(
                io,
                service_fn(move |req| {
                    Self::handle_request(Arc::clone(&handlers_clone), req, app_path_clone.clone())
                }),
            );

            let handle = tokio::spawn(async {
                if let Err(err) = serve_conn.await {
                    eprintln!("server connection error: {}", err);
                }
            });

            let _ = handle.await;
        }
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 2 {
        eprintln!("Please provide the app_path as a command line argument");
        std::process::exit(1);
    }
    let app_path = &args[1];

    let state = Value::Null;
    let mut server = BTRServer::new(([127, 0, 0, 1], 3000).into(), app_path.to_string());
    server.add_handler(
        Method::GET,
        "/",
        &format!("{}\\index.streams.json", app_path),
        state,
    );
    server.start().await?;
    Ok(())
}

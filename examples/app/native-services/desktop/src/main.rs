// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use anyhow::{Context, Result};
use futures_executor::block_on;
use webui_desktop::{
    CaptureOptions, CapturedContent, ConfirmDialog, DesktopApp, DesktopEvent, DialogOutcome,
    DirectoryPickerOptions, DirectorySelection, ErrorDialog, EventResponse, HostLifetime,
    LocalServerOptions, LoopbackOrigin, NativeServices, WindowOptions,
};
use webui_handler::plugin::webui::WebUIHydrationPlugin;
use webui_handler::{Protocol, RenderOptions, ResponseWriter, WebUIHandler};

const MAX_REQUEST_BYTES: usize = 8 * 1024;
const CAPABILITY_BYTES: usize = 32;

struct DemoServer {
    page: Vec<u8>,
    script: Vec<u8>,
    styles: Vec<u8>,
    capability: String,
    services: NativeServices,
    capture: Mutex<Option<CapturedContent>>,
}

fn main() -> Result<()> {
    let script = built_asset("index.js")?;
    let styles = built_asset("native-services-app.css")?;
    let page = render_page()?;
    let listener = TcpListener::bind("127.0.0.1:0").context("failed to bind demo server")?;
    listener
        .set_nonblocking(true)
        .context("failed to configure demo server")?;
    let address = listener
        .local_addr()
        .context("failed to read demo address")?;
    let origin = LoopbackOrigin::from_socket_addr(address)?;
    println!("Native services demo: {}/", origin.as_str());
    let capability = native_capability()?;
    let initial_path = format!("/?native-capability={capability}");
    let (owner, lifetime) = HostLifetime::new();
    let frame = DesktopApp::from_local_server(
        LocalServerOptions::new(origin, lifetime).initial_path(&initial_path)?,
    )
    .window(WindowOptions {
        title: "WebUI native services".to_string(),
        width: 1080,
        height: 820,
        ..WindowOptions::default()
    })
    .build()?;
    let services = frame.native_services()?;
    let stop = Arc::new(AtomicBool::new(false));
    let stopped = Arc::clone(&stop);
    frame.on_event(move |event| {
        if matches!(
            event,
            DesktopEvent::WindowClosed { .. } | DesktopEvent::Exiting
        ) {
            stopped.store(true, Ordering::Release);
        }
        EventResponse::Continue
    })?;
    let server_stop = Arc::clone(&stop);
    let demo = DemoServer {
        page,
        script,
        styles,
        capability,
        services,
        capture: Mutex::new(None),
    };
    let server = thread::Builder::new()
        .name("native-services-demo-http".to_string())
        .spawn(move || serve(listener, &demo, &server_stop))
        .context("failed to start demo server")?;
    let result = webui_desktop::run_local_server_frame(frame);
    owner.revoke().ok();
    stop.store(true, Ordering::Release);
    server
        .join()
        .map_err(|_| anyhow::anyhow!("demo server thread panicked"))??;
    result?;
    Ok(())
}

fn native_capability() -> Result<String> {
    let mut bytes = [0_u8; CAPABILITY_BYTES];
    getrandom::fill(&mut bytes).context("failed to generate native demo capability")?;
    let mut capability = String::with_capacity(CAPABILITY_BYTES * 2);
    for byte in bytes {
        use std::fmt::Write;
        write!(&mut capability, "{byte:02x}").context("failed to encode native demo capability")?;
    }
    Ok(capability)
}

fn built_asset(name: &str) -> Result<Vec<u8>> {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../dist")
        .join(name);
    std::fs::read(&path).with_context(|| {
        format!(
            "missing {}; run `pnpm build` in examples/app/native-services first",
            path.display()
        )
    })
}

fn render_page() -> Result<Vec<u8>> {
    let protocol = Protocol::from_protobuf(&built_asset("protocol.bin")?)
        .context("failed to decode native services protocol")?;
    let mut state: serde_json::Value = serde_json::from_str(include_str!("../../data/state.json"))
        .context("failed to parse native services state")?;
    let token_file = webui_tokens::parse_token_content(
        include_str!("../../../../../packages/webui-examples-theme/tokens.json"),
        Path::new("packages/webui-examples-theme/tokens.json"),
    )
    .context("failed to parse example theme")?;
    let tokens = webui_tokens::resolve_tokens(protocol.tokens(), &token_file)
        .context("failed to resolve example theme")?;
    webui_tokens::inject_into_state(&mut state, &tokens);
    let handler = WebUIHandler::with_plugin(|| Box::new(WebUIHydrationPlugin::new()));
    let mut writer = ByteWriter::default();
    handler
        .render(
            &protocol,
            &state,
            &RenderOptions::new("index.html", "/"),
            &mut writer,
        )
        .context("failed to render native services page")?;
    Ok(writer.bytes)
}

fn serve(listener: TcpListener, demo: &DemoServer, stop: &AtomicBool) -> Result<()> {
    while !stop.load(Ordering::Acquire) {
        match listener.accept() {
            Ok((mut stream, _)) => {
                if let Err(error) = demo.handle_request(&mut stream) {
                    eprintln!("native services request failed: {error:#}");
                    write_json(
                        &mut stream,
                        500,
                        "Action failed",
                        "The trusted host could not complete the request.",
                    )?;
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                thread::sleep(Duration::from_millis(8));
            }
            Err(error) => return Err(error).context("demo server accept failed"),
        }
    }
    Ok(())
}

impl DemoServer {
    fn handle_request(&self, stream: &mut TcpStream) -> Result<()> {
        stream
            .set_nonblocking(false)
            .context("failed to configure request stream")?;
        stream
            .set_read_timeout(Some(Duration::from_secs(2)))
            .context("failed to set request timeout")?;
        let mut request = [0_u8; MAX_REQUEST_BYTES];
        let request_head = read_request_head(stream, &mut request)?;
        let request_line = request_head
            .lines()
            .next()
            .context("request line was missing")?;
        let mut request_parts = request_line.split_whitespace();
        let method = request_parts.next().context("request method was missing")?;
        let target = request_parts.next().context("request target was missing")?;
        let version = request_parts.next().context("HTTP version was missing")?;
        if request_parts.next().is_some() || version != "HTTP/1.1" {
            anyhow::bail!("request line was invalid");
        }
        if method == "POST" && !authorized(request_head, &self.capability) {
            return write_json(
                stream,
                403,
                "Request denied",
                "Native demo actions require this window's private capability.",
            );
        }
        let path = target.split_once('?').map_or(target, |(path, _)| path);

        match (method, path) {
            ("GET", "/") => write_response(stream, 200, "text/html; charset=utf-8", &self.page),
            ("GET", "/index.js") => {
                write_response(stream, 200, "text/javascript; charset=utf-8", &self.script)
            }
            ("GET", "/native-services-app.css") => {
                write_response(stream, 200, "text/css; charset=utf-8", &self.styles)
            }
            ("POST", "/api/picker") => pick_directory(stream, &self.services),
            ("POST", "/api/dialog/error") => show_error(stream, &self.services),
            ("POST", "/api/dialog/confirm") => confirm(stream, &self.services),
            ("POST", "/api/capture") => capture_view(stream, &self.services, &self.capture),
            ("POST", "/api/clipboard") => copy_capture(stream, &self.services, &self.capture),
            _ => write_json(
                stream,
                404,
                "Not found",
                "The requested demo route does not exist.",
            ),
        }
    }
}

fn read_request_head<'a>(reader: &mut impl Read, buffer: &'a mut [u8]) -> Result<&'a str> {
    let mut size = 0;
    loop {
        if size == buffer.len() {
            anyhow::bail!("request headers exceed {MAX_REQUEST_BYTES} bytes");
        }
        let read = reader
            .read(&mut buffer[size..])
            .context("failed to read request")?;
        if read == 0 {
            anyhow::bail!("request ended before its headers were complete");
        }
        size += read;
        if let Some(end) = buffer[..size]
            .windows(4)
            .position(|bytes| bytes == b"\r\n\r\n")
        {
            return std::str::from_utf8(&buffer[..end + 4])
                .context("request headers were not UTF-8");
        }
    }
}

fn authorized(request_head: &str, capability: &str) -> bool {
    request_head
        .lines()
        .skip(1)
        .filter_map(|header| header.split_once(':'))
        .any(|(name, value)| {
            name.eq_ignore_ascii_case("X-WebUI-Native-Capability") && value.trim() == capability
        })
}

fn pick_directory(stream: &mut TcpStream, services: &NativeServices) -> Result<()> {
    let options = DirectoryPickerOptions::new().title("Choose a demo directory")?;
    match block_on(services.pick_directory(options)?)? {
        DirectorySelection::Selected(_path) => write_json(
            stream,
            200,
            "Directory selected",
            "The trusted host received one absolute directory selection.",
        ),
        DirectorySelection::Cancelled => write_json(
            stream,
            200,
            "Picker cancelled",
            "No directory was selected.",
        ),
    }
}

fn show_error(stream: &mut TcpStream, services: &NativeServices) -> Result<()> {
    let request = services.show_error(ErrorDialog::new(
        "Demo error",
        "This is bounded, host-authored copy. No raw application error crosses the boundary.",
        "Acknowledge",
    )?)?;
    let outcome = block_on(request)?;
    write_json(
        stream,
        200,
        "Error acknowledged",
        &format!("Native dialog completed with {outcome:?}."),
    )
}

fn confirm(stream: &mut TcpStream, services: &NativeServices) -> Result<()> {
    let request = services.confirm(ConfirmDialog::new(
        "Continue the demo?",
        "The cancel action is the safe default.",
        "Continue",
        "Cancel",
    )?)?;
    match block_on(request)? {
        DialogOutcome::Confirmed => write_json(
            stream,
            200,
            "Confirmed",
            "The native confirmation was accepted.",
        ),
        DialogOutcome::Cancelled => write_json(
            stream,
            200,
            "Cancelled",
            "The native confirmation was dismissed safely.",
        ),
        DialogOutcome::Acknowledged => write_json(
            stream,
            500,
            "Unexpected result",
            "A confirmation returned an acknowledgement outcome.",
        ),
    }
}

fn capture_view(
    stream: &mut TcpStream,
    services: &NativeServices,
    retained: &Mutex<Option<CapturedContent>>,
) -> Result<()> {
    retained
        .lock()
        .map_err(|_| anyhow::anyhow!("capture state lock is poisoned"))?
        .take();
    let content =
        block_on(services.capture_web_content(CaptureOptions::new().max_dimensions(1200, 1000)?)?)?;
    let mut png = Vec::with_capacity(content.png_bytes);
    let mut offset = 0;
    loop {
        let chunk = services.read_captured_content(&content, offset)?;
        png.extend_from_slice(&chunk.bytes);
        offset = chunk.next_offset;
        if chunk.eof {
            break;
        }
    }
    let mut slot = retained
        .lock()
        .map_err(|_| anyhow::anyhow!("capture state lock is poisoned"))?;
    *slot = Some(content);
    write_response(stream, 200, "image/png", &png)
}

fn copy_capture(
    stream: &mut TcpStream,
    services: &NativeServices,
    retained: &Mutex<Option<CapturedContent>>,
) -> Result<()> {
    let slot = retained
        .lock()
        .map_err(|_| anyhow::anyhow!("capture state lock is poisoned"))?;
    let content = slot
        .as_ref()
        .context("capture the visible webview before copying it")?;
    block_on(services.write_capture_to_clipboard(content)?)?;
    write_json(
        stream,
        200,
        "Copied to clipboard",
        "The native PNG write was acknowledged and read back byte for byte.",
    )
}

fn write_json(stream: &mut TcpStream, status: u16, heading: &str, detail: &str) -> Result<()> {
    let mut body = Vec::with_capacity(heading.len() + detail.len() + 25);
    body.extend_from_slice(b"{\"status\":");
    serde_json::to_writer(&mut body, heading)?;
    body.extend_from_slice(b",\"detail\":");
    serde_json::to_writer(&mut body, detail)?;
    body.push(b'}');
    write_response(stream, status, "application/json", &body)
}

fn write_response(
    stream: &mut TcpStream,
    status: u16,
    content_type: &str,
    body: &[u8],
) -> Result<()> {
    let reason = match status {
        200 => "OK",
        403 => "Forbidden",
        404 => "Not Found",
        _ => "Internal Server Error",
    };
    write!(
        stream,
        "HTTP/1.1 {status} {reason}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nCache-Control: no-store\r\nConnection: close\r\n\r\n",
        body.len()
    )?;
    stream.write_all(body)?;
    Ok(())
}

#[derive(Default)]
struct ByteWriter {
    bytes: Vec<u8>,
}

impl ResponseWriter for ByteWriter {
    fn write(&mut self, content: &str) -> webui_handler::Result<()> {
        self.bytes.extend_from_slice(content.as_bytes());
        Ok(())
    }

    fn end(&mut self) -> webui_handler::Result<()> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::io::Read;

    use super::{authorized, native_capability, read_request_head};

    const CAPABILITY: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

    struct ChunkReader<'a> {
        bytes: &'a [u8],
        offset: usize,
    }

    impl Read for ChunkReader<'_> {
        fn read(&mut self, output: &mut [u8]) -> std::io::Result<usize> {
            if self.offset == self.bytes.len() || output.is_empty() {
                return Ok(0);
            }
            output[0] = self.bytes[self.offset];
            self.offset += 1;
            Ok(1)
        }
    }

    #[test]
    fn request_head_is_read_across_partial_reads() {
        let bytes = b"POST /api/capture HTTP/1.1\r\nX-WebUI-Native-Capability: 0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef\r\n\r\nbody";
        let mut reader = ChunkReader { bytes, offset: 0 };
        let mut buffer = [0_u8; 256];
        let head = read_request_head(&mut reader, &mut buffer).unwrap_or_default();

        assert!(authorized(head, CAPABILITY));
        assert!(!head.contains("body"));
    }

    #[test]
    fn native_actions_require_header_not_body_text() {
        let bytes =
            b"POST /api/capture HTTP/1.1\r\nContent-Type: text/plain\r\n\r\nX-WebUI-Native-Capability: 0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
        let mut reader = ChunkReader { bytes, offset: 0 };
        let mut buffer = [0_u8; 256];
        let head = read_request_head(&mut reader, &mut buffer).unwrap_or_default();

        assert!(!authorized(head, CAPABILITY));
        assert!(!authorized(
            "POST /api/capture HTTP/1.1\r\n\r\n",
            CAPABILITY
        ));
        assert!(!authorized(
            "POST /api/capture HTTP/1.1\r\nX-WebUI-Native-Capability: wrong\r\n\r\n",
            CAPABILITY
        ));
    }

    #[test]
    fn native_capability_is_random_hex_with_full_entropy_width() {
        let first = native_capability().unwrap();
        let second = native_capability().unwrap();
        assert_eq!(first.len(), 64);
        assert!(first.bytes().all(|byte| byte.is_ascii_hexdigit()));
        assert_ne!(first, second);
    }
}

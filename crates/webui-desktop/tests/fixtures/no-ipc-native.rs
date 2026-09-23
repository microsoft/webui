// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

//! Actual-native no-IPC smoke test (requires a usable desktop session).
//! Run with `cargo run -p microsoft-webui-desktop --example no-ipc-native
//! --no-default-features --features native,source -- source` (or `bundle`).
//! The `frameless-source` and `frameless-bundle` modes also verify close vetoes.
//! Application IPC must not be enabled by workspace feature unification.

use std::sync::{
    atomic::{AtomicBool, AtomicUsize, Ordering},
    Arc, OnceLock,
};
use webui_desktop::{
    build_desktop_bundle, BuildOptions, DesktopApp, DesktopBundleOptions, DesktopEvent,
    DesktopProtocolResponse, DesktopShellConfig, DesktopSourceConfig, EventResponse, TitlebarStyle,
    WindowHandle, WindowOptions,
};

const SCRIPT: &str = r#"
(async () => {
  const assert = (ok, message) => { if (!ok) throw new Error(message); };
  assert(document.querySelector('h1').textContent === 'No application IPC', 'SSR');
  assert(!('__webuiDesktopIpcV2' in globalThis), 'IPC bootstrap installed');
  assert(!('__webuiDesktopIpcReceiveV2' in globalThis), 'IPC receiver installed');
  assert(!window.webkit?.messageHandlers?.webuiDesktopIpc, 'IPC native handler installed');
  assert(typeof window.webuiHostPostMessage === 'function' || !!window.chrome?.webview,
         'window control bridge missing');
  for (const path of ['/_webui/ipc', '/_webui/ipc/outbound',
                      '/_webui/ipc/runtime.js', '/_webui/ipc/bootstrap.js']) {
    assert((await fetch(path)).status === 404, 'reserved IPC route: ' + path);
  }
  const response = await fetch('/api/echo', {method: 'POST', body: 'ordinary API'});
  assert(await response.text() === 'ordinary API', 'custom protocol API');
  const worker = await fetch('/api/worker');
  assert(worker.ok && (await worker.text()).startsWith('webui-app-'), 'API ran on native UI thread');
  await fetch('/result', {method: 'POST', body: 'pass'});
})().catch(error => fetch('/result', {method: 'POST', body: String(error)}));
"#;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    if cfg!(feature = "application-ipc") {
        return Err("run this fixture without application-ipc".into());
    }
    let mode = std::env::args()
        .nth(1)
        .ok_or("expected source, bundle, frameless-source, or frameless-bundle")?;
    let frameless = mode.starts_with("frameless-");
    let input_mode = mode.strip_prefix("frameless-").unwrap_or(&mode);
    let root = tempfile::tempdir()?;
    let app = root.path().join("app");
    std::fs::create_dir(&app)?;
    std::fs::write(
        app.join("index.html"),
        "<!doctype html><html><head></head><body><h1>No application IPC</h1>\
         <script src=\"/check.js\"></script></body></html>",
    )?;
    let options = BuildOptions {
        app_dir: app,
        ..BuildOptions::default()
    };
    let builder = match input_mode {
        "source" => DesktopApp::from_source(DesktopSourceConfig::new(options)),
        "bundle" => {
            let bundle = root.path().join("bundle");
            let manifest = build_desktop_bundle(DesktopBundleOptions {
                build_options: options,
                out_dir: bundle.clone(),
                state_file: None,
                asset_root: None,
                token_css: None,
                app_id: "webui.test.no-ipc".into(),
                app_name: "No IPC fixture".into(),
                version: "0.0.0".into(),
                publisher: "Microsoft".into(),
                window: WindowOptions::default(),
                icon_file: None,
                shell: DesktopShellConfig::default(),
                package_targets: vec![],
            })?;
            if bundle.join("assets/_webui/ipc").exists()
                || manifest
                    .integrity
                    .assets
                    .iter()
                    .any(|asset| asset.path.contains("_webui/ipc"))
            {
                return Err("bundle contains IPC assets".into());
            }
            DesktopApp::from_bundle(bundle)?
        }
        _ => return Err("expected source, bundle, frameless-source, or frameless-bundle".into()),
    };
    let builder = if frameless {
        builder.window(WindowOptions {
            titlebar: TitlebarStyle::None,
            ..WindowOptions::default()
        })
    } else {
        builder
    };
    let window = Arc::new(OnceLock::<WindowHandle>::new());
    let passed = Arc::new(AtomicBool::new(false));
    let closed = Arc::new(AtomicBool::new(false));
    let result_window = Arc::clone(&window);
    let result_passed = Arc::clone(&passed);
    let ui_thread = std::thread::current().id();
    let frame = builder
        .api_route("/check.js", |_| {
            Ok(DesktopProtocolResponse::new(
                200,
                "text/javascript",
                SCRIPT.as_bytes().to_vec(),
            ))
        })?
        .api_route("/api/echo", |context| {
            Ok(DesktopProtocolResponse::new(
                200,
                "text/plain",
                context.body.to_vec(),
            ))
        })?
        .api_route("/api/worker", move |_| {
            let current = std::thread::current();
            Ok(DesktopProtocolResponse::text(
                if current.id() == ui_thread { 500 } else { 200 },
                current.name().unwrap_or("unnamed"),
            ))
        })?
        .api_route("/result", move |context| {
            let success = context.body == b"pass";
            result_passed.store(success, Ordering::SeqCst);
            eprintln!(
                "NO_IPC_NATIVE_RESULT {}",
                String::from_utf8_lossy(context.body)
            );
            if let Some(window) = result_window.get() {
                window
                    .request_close()
                    .map_err(|source| webui_desktop::DesktopError::Backend {
                        source: Box::new(source),
                    })?;
            }
            Ok(DesktopProtocolResponse::text(200, "recorded"))
        })?
        .build()?;
    window
        .set(frame.window_handle().clone())
        .map_err(|_| "duplicate window")?;
    let closed_event = Arc::clone(&closed);
    let passed_event = Arc::clone(&passed);
    let close_requests = AtomicUsize::new(0);
    let close_handle = frame.window_handle().clone();
    let closed_mode = mode;
    frame.on_event(move |event| {
        if frameless && matches!(event, DesktopEvent::WindowCloseRequested { .. }) {
            if close_requests.fetch_add(1, Ordering::SeqCst) == 0 {
                if let Err(error) = close_handle.request_close() {
                    eprintln!("NO_IPC_NATIVE_FAILURE could not retry vetoed close: {error}");
                    std::process::exit(1);
                }
                return EventResponse::PreventDefault;
            }
        }
        if matches!(event, DesktopEvent::WindowClosed { .. }) {
            if closed_event.swap(true, Ordering::SeqCst) {
                eprintln!("NO_IPC_NATIVE_FAILURE duplicate native close");
                std::process::exit(1);
            }
            // AppKit's termination can exit the process before run_frame returns.
            // Report only after the real close event, and fail closed on errors.
            if !passed_event.load(Ordering::SeqCst) {
                eprintln!("NO_IPC_NATIVE_FAILURE no successful renderer result");
                std::process::exit(1);
            }
            if frameless && close_requests.load(Ordering::SeqCst) != 2 {
                eprintln!("NO_IPC_NATIVE_FAILURE frameless close did not honor its first veto");
                std::process::exit(1);
            }
            println!("NO_IPC_NATIVE_PASS mode={closed_mode}");
        }
        EventResponse::Continue
    })?;
    webui_desktop::run_frame(frame)?;
    if !passed.load(Ordering::SeqCst) || !closed.load(Ordering::SeqCst) {
        return Err("no-IPC assertions or native close lifecycle did not complete".into());
    }
    Ok(())
}

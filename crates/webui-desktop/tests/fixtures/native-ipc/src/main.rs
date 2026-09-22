// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

mod generated {
    #![allow(dead_code)]
    include!("../generated/rust/ipc.rs");
}
mod host;
mod lifecycle;
mod platform;

use std::path::PathBuf;
use std::sync::{Arc, OnceLock};
#[cfg(feature = "source")]
use webui_desktop::{
    build_desktop_bundle, BuildOptions, DesktopBundleOptions, DesktopShellConfig,
    DesktopSourceConfig, WindowOptions,
};
use webui_desktop::{ipc::IpcOptions, DesktopApp, DesktopEvent, EventResponse, IpcRegistry};

#[cfg(feature = "source")]
fn build_options(root: PathBuf) -> BuildOptions {
    BuildOptions {
        app_dir: root,
        ..BuildOptions::default()
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().collect();
    let mode = args.get(1).ok_or("expected build, source, or packaged")?;
    if mode == "metadata" {
        platform::print_metadata();
        return Ok(());
    }
    let root = PathBuf::from(args.get(2).ok_or("expected app directory")?);
    #[cfg(feature = "source")]
    if mode == "package" {
        return platform::package(
            &root,
            PathBuf::from(args.get(3).ok_or("expected package output")?),
            PathBuf::from(args.get(4).ok_or("expected runtime-only runner")?),
        );
    }
    #[cfg(feature = "source")]
    if mode == "build" {
        build_desktop_bundle(DesktopBundleOptions {
            build_options: build_options(root.clone()),
            out_dir: PathBuf::from(args.get(3).ok_or("expected bundle output")?),
            state_file: None,
            asset_root: Some(root.join("assets")),
            token_css: None,
            app_id: "webui.test.native-ipc".into(),
            app_name: "Native IPC fixture".into(),
            version: "0.0.0".into(),
            publisher: "Microsoft".into(),
            window: WindowOptions::default(),
            icon_file: None,
            shell: DesktopShellConfig::default(),
            package_targets: vec![],
        })?;
        return Ok(());
    }
    let window = Arc::new(OnceLock::new());
    let host = Arc::new(host::Host::new(Arc::clone(&window)));
    let mut registry = IpcRegistry::new(&generated::SCHEMA);
    generated::register_host(&mut registry, host.clone())?;
    let builder = match mode.as_str() {
        #[cfg(feature = "source")]
        "source" => {
            let mut config = DesktopSourceConfig::new(build_options(root.clone()));
            config.asset_root = Some(root.join("assets"));
            DesktopApp::from_source(config)
        }
        "packaged" => {
            let resources = webui_desktop::find_packaged_resources_dir()
                .ok_or("SDK packaged resources missing")?;
            if resources.canonicalize()? != root.canonicalize()? {
                return Err("SDK packaged resource discovery differs from package metadata".into());
            }
            DesktopApp::from_bundle(resources)?
        }
        _ => return Err("unknown mode".into()),
    };
    let diagnostic_window = Arc::clone(&window);
    let disconnect_host = Arc::clone(&host);
    let frame = builder
        // This module exists only in the runtime, never in packaged assets.
        // Its browser and worker loads prove that WebView2's native resource
        // handler owns source and packaged requests.
        .api_route("/fixture-runtime.js", |_| {
            Ok(webui_desktop::DesktopProtocolResponse::new(
                200,
                "text/javascript",
                b"export const servedByRuntime = true;".to_vec(),
            ))
        })?
        .api_route("/fixture-resource-echo", |context| {
            Ok(webui_desktop::DesktopProtocolResponse::new(
                if *context.method == webui_desktop::DesktopHttpMethod::Post {
                    200
                } else {
                    405
                },
                "application/octet-stream",
                context.body.to_vec(),
            ))
        })?
        .api_route("/fixture-disconnect-observation", move |_| {
            let observed = disconnect_host.disconnect_observed();
            Ok(webui_desktop::DesktopProtocolResponse::new(
                if observed { 200 } else { 409 },
                "text/plain",
                if observed {
                    b"closed".to_vec()
                } else {
                    b"native session is not closed".to_vec()
                },
            ))
        })?
        .api_route("/fixture-diagnostic", move |context| {
            eprintln!(
                "NATIVE_IPC_DIAGNOSTIC {}",
                String::from_utf8_lossy(context.body)
            );
            if context.body.starts_with(b"failure") {
                if let Some(window) = diagnostic_window.get() {
                    let _ = window.request_close();
                }
            }
            Ok(webui_desktop::DesktopProtocolResponse::new(
                200,
                "text/plain",
                vec![],
            ))
        })?
        .ipc_registry(registry)
        .ipc_options(IpcOptions::for_schema(&generated::SCHEMA))
        .build()?;
    window
        .set(frame.window_handle().clone())
        .map_err(|_| "duplicate window")?;
    let mode = mode.clone();
    frame.on_event(move |event| {
        eprintln!("NATIVE_IPC_LIFECYCLE {event:?}");
        if matches!(event, DesktopEvent::WindowClosed { .. }) {
            host.report_closed(&mode);
        }
        EventResponse::Continue
    })?;
    eprintln!("NATIVE_IPC_START mode={}", args[1]);
    webui_desktop::run_frame(frame)?;
    Ok(())
}

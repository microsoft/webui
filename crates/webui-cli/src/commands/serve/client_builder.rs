// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

mod coordinator;
mod inputs;
mod paths;
mod worker;

use std::io::{IsTerminal, Read};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use actix_web::{web, App, HttpServer};
use anyhow::{Context, Result};
use webui_dev_server::LiveReload;

use super::{
    configure_routes, map_bind_error, RenderConfig, ResponsePolicy, ServeArgs, ServePaths,
    ServerContext, SharedState, HMR_ENDPOINT,
};
use crate::utils::output;
use coordinator::Coordinator;
pub(super) use paths::input_path;
use worker::ClientBuilder;

pub(super) fn serve(args: &ServeArgs, policy: Arc<ResponsePolicy>) -> Result<()> {
    let paths = ServePaths::from_args(args)?;
    let module = paths::module_path(args, &paths)?;
    let entry = paths::client_entry(args, &paths)?;
    let output_dir = paths
        .serve_dir
        .as_ref()
        .context("Client builds require --servedir")?;
    let config = RenderConfig {
        app_args: args.app_args.clone(),
        app_dir: paths.app_dir.clone(),
        state_file: paths.state_file.clone(),
        token_file: None,
        component_asset_roots: args.emit_component_assets.clone(),
        metafile: paths.metafile.clone(),
        base_path: args.base_path.clone(),
    };
    let watching = args.watch && !super::watch_disabled_by_env();
    let livereload = watching.then(|| LiveReload::new(HMR_ENDPOINT));
    let state = Arc::new(Mutex::new(SharedState {
        rendered_html: String::new(),
        css_files: Default::default(),
        component_assets: Default::default(),
        protocol: None,
        state_data: None,
        token_css: None,
        rebuild_error: None,
        rebuild_pending: true,
        entry: args.app_args.entry.clone(),
    }));
    let context = web::Data::new(ServerContext {
        state: Arc::clone(&state),
        livereload: livereload.clone(),
        assets_dir: paths.serve_dir.clone(),
        api_port: args.api_port,
        plugin: args.app_args.plugin,
        base_path: args.base_path.clone(),
        response_policy: policy,
        api_state_errors: args.api_state_errors.unwrap_or_default(),
        chunk_pool: Arc::new(webui::streaming::ChunkPool::new(
            256,
            webui::streaming::StreamingWriter::CHUNK_TARGET + 1024,
        )),
    });
    let listener = std::net::TcpListener::bind(("127.0.0.1", args.port))
        .map_err(|error| map_bind_error(args.port, "127.0.0.1", error))?;
    let url = format!("http://{}/", listener.local_addr()?);
    output::header("WebUI Dev Server");
    output::field("Local:", &url);
    output::field(
        "Client",
        &module
            .as_deref()
            .or(entry.as_deref())
            .context("Select --client-builder or --client-entry")?
            .display(),
    );
    output::field("Output", &output_dir.display());
    output::field("Watch", &watching);
    let timeout = Duration::from_millis(args.client_build_timeout_ms);
    std::time::Instant::now()
        .checked_add(timeout)
        .context("--client-build-timeout-ms exceeds the platform's timer limit")?;
    actix_web::rt::System::new().block_on(async {
        let server = HttpServer::new(move || {
            let mut app = App::new()
                .wrap(context.response_policy.default_headers())
                .app_data(context.clone())
                .configure(|cfg| configure_routes(cfg, context.api_port.is_some(), watching));
            if let Some(livereload) = &context.livereload {
                app = app.app_data(web::Data::new(livereload.clone()));
            }
            app
        })
        .workers(1)
        .disable_signals()
        .listen(listener)?
        .run();
        let handle = server.handle();
        let mut server_task = actix_web::rt::spawn(server);
        let shutdown = shutdown_signal();
        tokio::pin!(shutdown);
        let spawned = if let Some(module) = &module {
            ClientBuilder::spawn(module, &paths.app_dir, output_dir, timeout)
        } else {
            ClientBuilder::spawn_builtin(
                &paths.app_dir,
                output_dir,
                entry.as_deref().context("Missing client entry")?,
                timeout,
            )
        };
        let mut builder = match spawned {
            Ok(builder) => builder,
            Err(error) => {
                handle.stop(false).await;
                return Err(error);
            }
        };
        let result = async {
            let initialized = tokio::select! {
                result = builder.initialize() => result.map(|()| true),
                result = &mut shutdown => result.map(|()| false),
                result = &mut server_task => {
                    Err(anyhow::anyhow!("Development listener stopped unexpectedly: {result:?}"))
                }
            }?;
            if !initialized {
                return Ok(());
            }
            let watch_config =
                paths::watch_config(args, &paths, module.as_deref(), builder.watch_paths())?;
            let mut coordinator =
                Coordinator::new(state, config, args.theme.clone(), livereload, output_dir)?;
            let watcher = watching
                .then(|| coordinator.watch(watch_config, module))
                .transpose()?;
            let result = tokio::select! {
                result = coordinator.run(&mut builder, watching, timeout) => result,
                result = &mut shutdown => result,
                result = &mut server_task => {
                    Err(anyhow::anyhow!("Development listener stopped unexpectedly: {result:?}"))
                }
            };
            coordinator.stop()?;
            drop(watcher);
            result
        }
        .await;
        handle.stop(false).await;
        let cleanup = builder.close().await;
        if let Err(error) = &cleanup {
            output::error(error);
        }
        result.and(cleanup)
    })
}

async fn shutdown_signal() -> Result<()> {
    let (sender, receiver) = tokio::sync::oneshot::channel();
    if std::io::stdin().is_terminal() {
        tokio::signal::ctrl_c()
            .await
            .context("Cannot receive shutdown signal")?;
        return Ok(());
    }
    std::thread::Builder::new()
        .name("webui-host-lifetime".into())
        .spawn(move || {
            let mut input = std::io::stdin().lock();
            let mut bytes = [0_u8; 256];
            loop {
                match input.read(&mut bytes) {
                    Ok(0) => {
                        let _ = sender.send(Ok(()));
                        break;
                    }
                    Ok(_) => {}
                    Err(error) if error.kind() == std::io::ErrorKind::Interrupted => {}
                    Err(error) => {
                        let _ = sender.send(Err(error));
                        break;
                    }
                }
            }
        })
        .context("Cannot monitor host lifetime")?;
    tokio::select! {
        result = receiver => result.context("Host lifetime monitor stopped")?.context("Cannot read host input"),
        result = tokio::signal::ctrl_c() => result.context("Cannot receive shutdown signal"),
    }
}

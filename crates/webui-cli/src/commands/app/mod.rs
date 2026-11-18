// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

mod config;
mod frontend;
mod host;
mod process;

use std::fs::File;
use std::num::NonZeroU64;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use clap::{Args, Subcommand};
use tokio::sync::Notify;
use webui_dev_server::shutdown::{self, Mode};
use webui_dev_server::{spawn_watcher, WatchConfig};

use super::init::Server;
use super::native::control::Stop;
use super::native::{self, NativeError};
use crate::utils::output;
use config::Project;

#[derive(Args)]
pub(crate) struct AppArgs {
    /// Application directory containing webui.toml
    #[arg(long, default_value = ".", global = true)]
    project: PathBuf,
    /// Accept one S byte or stdin EOF as a graceful supervisor stop request
    #[arg(long, global = true)]
    control_stdin: bool,
    #[command(subcommand)]
    command: AppCommand,
}

#[derive(Subcommand)]
enum AppCommand {
    /// Check TypeScript with the native compiler and check the selected host
    Check,
    /// Build matching browser assets, protocol and native host
    Build {
        /// Publish only frontend artifacts for a direct Cargo/Go build
        #[arg(long)]
        frontend_only: bool,
    },
    /// Watch source events, rebuild and restart the selected host
    Dev,
    /// Run the last complete build without restoring or compiling
    Start,
}

pub(crate) fn starter_configuration(server: Server) -> Result<String> {
    Ok(toml::to_string(&config::starter(server))?)
}

pub(crate) fn execute(args: &AppArgs) -> Result<i32> {
    let result = (|| match shutdown::prepare_with_stdin(NonZeroU64::new(13), args.control_stdin)? {
        Mode::Supervisor(code) => Ok(code),
        Mode::Child(control) => {
            let runtime = tokio::runtime::Builder::new_multi_thread()
                .worker_threads(2)
                .max_blocking_threads(16)
                .enable_all()
                .build()?;
            let result = runtime.block_on(async {
                let mut stop = Stop::new(control)?;
                run(args, &mut stop).await
            });
            match result {
                Ok(0) => Ok(shutdown::JOINED_EXIT_CODE),
                Err(error) if cancelled(&error) => Ok(shutdown::JOINED_EXIT_CODE),
                other => other,
            }
        }
        Mode::Direct => Err(NativeError::Cleanup(
            "native application supervisor was not established".into(),
        )
        .into()),
    })();
    result.inspect_err(output::error)
}

fn cancelled(error: &anyhow::Error) -> bool {
    error
        .downcast_ref::<NativeError>()
        .is_some_and(|error| matches!(error, NativeError::Cancelled))
}

fn writer(project: &Project) -> Result<File> {
    native::ensure_directory(&project.root, Path::new(".webui"))?;
    let path = project.root.join(".webui/app.lock");
    let lock = native::open_lock(&path)?;
    lock.try_lock().map_err(|error| NativeError::Config(format!(
        "another app build, dev or start owns this application, or locking is unavailable: {error}")))?;
    Ok(lock)
}

async fn run(args: &AppArgs, stop: &mut Stop) -> Result<i32> {
    let project = Project::load(&args.project)?;
    match &args.command {
        AppCommand::Check => {
            let _writer = if matches!(
                project.config.server,
                Server::C | Server::Cpp | Server::Dotnet
            ) {
                Some(writer(&project)?)
            } else {
                None
            };
            if project.config.browser.is_some() {
                frontend::prepare_resources(&project)?;
                let checker = super::toolchain::resolve().map_err(NativeError::from)?;
                let mut command = Command::new(checker.compiler);
                command.current_dir(&project.root).args([
                    "--project",
                    "tsconfig.json",
                    "--noEmit",
                    "--incremental",
                    "false",
                    "--pretty",
                    "false",
                ]);
                process::run(command, stop).await?;
            }
            host::check(&project, stop).await?;
            output::success("Native application checks passed");
            Ok(0)
        }
        AppCommand::Build { frontend_only } => {
            let _writer = writer(&project)?;
            build(&project, false, *frontend_only, stop).await?;
            Ok(0)
        }
        AppCommand::Start => {
            let _writer = writer(&project)?;
            let mut server = host::start(&project)?;
            tokio::select! {
                reason = stop.requested() => {
                    server.stop().await?;
                    reason?;
                    Ok(0)
                }
                status = server.child.wait() => {
                    let status = status?;
                    status.code().ok_or_else(|| NativeError::Host(format!("native host terminated with {status}")).into())
                }
            }
        }
        AppCommand::Dev => {
            let _writer = writer(&project)?;
            development(args, project, stop).await
        }
    }
}

async fn build(
    project: &Project,
    development: bool,
    frontend_only: bool,
    stop: &mut Stop,
) -> Result<()> {
    frontend::build_frontend(project, development, stop).await?;
    let executable = if frontend_only {
        None
    } else {
        Some(host::build(project, stop).await?)
    };
    frontend::publish(project)?;
    if let Some(executable) = executable {
        host::save(project, &executable)?;
    }
    output::success("Native application build complete");
    Ok(())
}

async fn development(args: &AppArgs, mut project: Project, stop: &mut Stop) -> Result<i32> {
    let changed = Arc::new(Notify::new());
    let version = Arc::new(AtomicU64::new(0));
    let failed = Arc::new(AtomicBool::new(false));
    let callback_changed = Arc::clone(&changed);
    let callback_version = Arc::clone(&version);
    let retry = Arc::clone(&failed);
    let ignore = [
        ".git",
        "node_modules",
        "dist",
        ".webui",
        "target",
        "bin",
        "obj",
        "build",
    ]
    .iter()
    .map(|name| project.root.join(name))
    .collect();
    let _watcher = spawn_watcher(
        WatchConfig {
            paths: vec![project.root.clone()],
            explicit_files: Vec::new(),
            ignore,
            debounce: Duration::from_millis(50),
            retry_unchanged_when: Some(Arc::new(move || retry.load(Ordering::Acquire))),
        },
        move |_| {
            callback_version.fetch_add(1, Ordering::AcqRel);
            callback_changed.notify_one();
        },
    )?;
    let mut server = None;
    loop {
        if let Some(previous) = server.take() {
            process::Server::stop(previous).await?;
        }
        let current = version.load(Ordering::Acquire);
        let built = async {
            project = Project::load(&args.project)?;
            build(&project, true, false, stop).await
        }
        .await;
        match built {
            Ok(()) => {
                failed.store(false, Ordering::Release);
                if version.load(Ordering::Acquire) != current {
                    continue;
                }
                server = Some(host::start(&project)?);
            }
            Err(error) => {
                if cancelled(&error)
                    || error
                        .downcast_ref::<NativeError>()
                        .is_some_and(|error| matches!(error, NativeError::Cleanup(_)))
                {
                    return Err(error);
                }
                output::error(&error);
                failed.store(true, Ordering::Release);
            }
        }
        if version.load(Ordering::Acquire) != current {
            continue;
        }
        if let Some(active) = &mut server {
            tokio::select! {
                reason = stop.requested() => {
                    if let Some(active) = server.take() { active.stop().await?; }
                    reason?;
                    return Ok(0);
                }
                _ = changed_since(&changed, &version, current) => {}
                status = active.child.wait() => {
                    return Err(NativeError::Host(format!("development host exited unexpectedly: {}", status?)).into());
                }
            }
        } else {
            tokio::select! {
                reason = stop.requested() => { reason?; return Ok(0); }
                _ = changed_since(&changed, &version, current) => {}
            }
        }
    }
}

async fn changed_since(changed: &Notify, version: &AtomicU64, current: u64) {
    loop {
        let notified = changed.notified();
        if version.load(Ordering::Acquire) != current {
            return;
        }
        notified.await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures_util::FutureExt;

    #[tokio::test]
    async fn stale_build_notifications_do_not_restart_a_current_generation() {
        let changed = Notify::new();
        let version = AtomicU64::new(1);
        changed.notify_one();
        assert!(changed_since(&changed, &version, 1)
            .now_or_never()
            .is_none());
        version.store(2, Ordering::Release);
        assert!(changed_since(&changed, &version, 1)
            .now_or_never()
            .is_some());
    }
}

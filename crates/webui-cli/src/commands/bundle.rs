// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

use std::fs::File;
use std::num::NonZeroU64;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use clap::Args;
use microsoft_webui_client_build::{build, ClientBuild, ClientBuildOptions};
use sha2::{Digest, Sha256};
use tokio::sync::Notify;
use webui_dev_server::shutdown::{self, Mode};
use webui_dev_server::{spawn_watcher, WatchConfig};

use super::native::control::Stop;
use super::native::{self, NativeError};
use crate::utils::output;

#[derive(Args)]
pub(crate) struct BundleArgs {
    /// JSON configuration for the native browser compiler
    #[arg(long)]
    config: PathBuf,
    /// Projection manifest destination inside the configured project
    #[arg(long)]
    manifest: Option<PathBuf>,
    /// Rebuild on native filesystem events
    #[arg(long)]
    watch: bool,
    /// Explicit project subtrees excluded from watch invalidation
    #[arg(long)]
    watch_ignore: Vec<PathBuf>,
    /// Accept one S byte or stdin EOF as a graceful supervisor stop request
    #[arg(long)]
    control_stdin: bool,
}

pub(crate) fn execute(args: &BundleArgs) -> Result<i32> {
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
                Ok(()) => Ok(shutdown::JOINED_EXIT_CODE),
                Err(error) if cancelled(&error) => Ok(shutdown::JOINED_EXIT_CODE),
                Err(error) => Err(error),
            }
        }
        Mode::Direct => Err(NativeError::Cleanup(
            "native browser build supervisor was not established".into(),
        )
        .into()),
    })();
    result.inspect_err(output::error)
}

fn load(path: &Path) -> Result<ClientBuildOptions> {
    let bytes = native::read_metadata(path)?;
    serde_json::from_slice(&bytes).map_err(|error| {
        NativeError::Config(format!(
            "invalid native browser configuration {}: {error}",
            path.display()
        ))
        .into()
    })
}

fn cancelled(error: &anyhow::Error) -> bool {
    error
        .downcast_ref::<NativeError>()
        .is_some_and(|error| matches!(error, NativeError::Cancelled))
}

fn writer(project: &Path, out_dir: &Path) -> Result<File> {
    native::ensure_directory(project, Path::new(".webui/bundle-locks"))?;
    let output = project.join(out_dir);
    let portable = output.to_string_lossy().replace('\\', "/");
    let identity = if cfg!(any(windows, target_os = "macos")) {
        portable.to_lowercase()
    } else {
        portable
    };
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut name = String::with_capacity(69);
    for byte in Sha256::digest(identity.as_bytes()) {
        name.push(char::from(HEX[usize::from(byte >> 4)]));
        name.push(char::from(HEX[usize::from(byte & 15)]));
    }
    name.push_str(".lock");
    let lock = native::open_lock(&project.join(".webui/bundle-locks").join(name))?;
    lock.try_lock().map_err(|error| {
        NativeError::Config(format!(
        "another browser watcher owns this output directory, or locking is unavailable: {error}"
    ))
    })?;
    Ok(lock)
}

fn paths(options: &ClientBuildOptions, config: &Path) -> Vec<PathBuf> {
    let mut paths = Vec::with_capacity(options.entries.len() + 1);
    paths.push(config.to_path_buf());
    paths.extend(
        options
            .entries
            .values()
            .filter(|path| !path.to_string_lossy().starts_with('\0'))
            .map(|path| options.project.join(path)),
    );
    paths
}

fn protect_config(client: &ClientBuild, manifest: &Path, config: &Path) -> Result<()> {
    let config_metadata = std::fs::metadata(config)?;
    for path in client.files.keys().map(PathBuf::as_path).chain([manifest]) {
        let destination = client.project().join(path);
        let canonical = std::fs::canonicalize(&destination).ok();
        let mut same = destination == config || canonical.as_deref() == Some(config);
        #[cfg(unix)]
        if let Ok(metadata) = std::fs::metadata(&destination) {
            use std::os::unix::fs::MetadataExt;
            same |=
                metadata.dev() == config_metadata.dev() && metadata.ino() == config_metadata.ino();
        }
        #[cfg(windows)]
        {
            let _ = &config_metadata;
            same |= destination.to_string_lossy().to_lowercase()
                == config.to_string_lossy().to_lowercase();
        }
        if same {
            return Err(NativeError::Config(
                "browser outputs cannot replace their build configuration; choose another output or manifest path".into()
            ).into());
        }
    }
    Ok(())
}

async fn run(args: &BundleArgs, stop: &mut Stop) -> Result<()> {
    let config = args.config.canonicalize()?;
    let initial = load(&config)?;
    let project = initial.project.canonicalize()?;
    let output_scope = project.join(&initial.out_dir);
    let _writer = writer(&project, &initial.out_dir)?;
    let changed = Arc::new(Notify::new());
    let epoch = Arc::new(AtomicU64::new(0));
    let failed = Arc::new(AtomicBool::new(false));
    let callback_changed = Arc::clone(&changed);
    let callback_epoch = Arc::clone(&epoch);
    let retry = Arc::clone(&failed);
    let manifest_scope = project.join(
        args.manifest
            .clone()
            .unwrap_or_else(|| initial.out_dir.join("webui-projection.json")),
    );
    let mut ignored = vec![manifest_scope, project.join(".webui/bundle-locks")];
    ignored.extend(args.watch_ignore.iter().map(|path| project.join(path)));
    let mut previous_outputs = Vec::new();
    let mut watcher = if args.watch {
        Some(spawn_watcher(
            WatchConfig {
                paths: vec![project.clone()],
                explicit_files: paths(&initial, &config),
                ignore: ignored.clone(),
                debounce: Duration::from_millis(50),
                retry_unchanged_when: Some(Arc::new(move || retry.load(Ordering::Acquire))),
            },
            move |_| {
                callback_epoch.fetch_add(1, Ordering::AcqRel);
                callback_changed.notify_one();
            },
        )?)
    } else {
        None
    };
    loop {
        let current = epoch.load(Ordering::Acquire);
        let result = async {
            let mut options = load(&config)?;
            if options.project.canonicalize()? != project {
                return Err(NativeError::Config(
                    "the native browser project changed; restart the watcher".into(),
                )
                .into());
            }
            options.project = project.clone();
            if options.out_dir.is_absolute() {
                options.out_dir = options
                    .out_dir
                    .strip_prefix(&project)
                    .map_err(|error| {
                        NativeError::Config(format!(
                            "browser output is outside the project: {error}"
                        ))
                    })?
                    .to_path_buf();
            }
            let out_dir = project.join(&options.out_dir);
            if out_dir != output_scope {
                return Err(NativeError::Config(
                    "the native browser output directory changed; restart the watcher".into(),
                )
                .into());
            }
            let manifest = args
                .manifest
                .clone()
                .unwrap_or_else(|| options.out_dir.join("webui-projection.json"));
            let client = tokio::select! {
                reason = stop.requested() => {
                    reason?;
                    return Err(NativeError::Cancelled.into());
                }
                result = build(options) => result.map_err(NativeError::Build)?,
            };
            protect_config(&client, &manifest, &config)?;
            let outputs: Vec<_> = client.files.keys().map(|path| project.join(path)).collect();
            if let Some(watcher) = &mut watcher {
                let mut exclusions = ignored.clone();
                exclusions.extend(
                    previous_outputs
                        .iter()
                        .chain(&outputs)
                        .filter(|path| !client.dependencies.contains(*path))
                        .cloned(),
                );
                exclusions.extend(
                    client
                        .publication_temporary_roots(&manifest)
                        .map_err(NativeError::Build)?,
                );
                watcher.set_ignored_paths(exclusions);
            }
            if publish_current(&client, &manifest, &epoch, current)? {
                previous_outputs = outputs;
            }
            Ok::<(), anyhow::Error>(())
        }
        .await;
        match result {
            Ok(()) => failed.store(false, Ordering::Release),
            Err(error) if !args.watch || cancelled(&error) => return Err(error),
            Err(error) => {
                output::error(&error);
                failed.store(true, Ordering::Release);
            }
        }
        if !args.watch {
            return Ok(());
        }
        if epoch.load(Ordering::Acquire) != current {
            continue;
        }
        tokio::select! {
            reason = stop.requested() => { reason?; return Ok(()); }
            _ = changed_since(&changed, &epoch, current) => {}
        }
    }
}

fn publish_current(
    client: &ClientBuild,
    manifest: &Path,
    epoch: &AtomicU64,
    current: u64,
) -> Result<bool> {
    if epoch.load(Ordering::Acquire) != current {
        return Ok(false);
    }
    client.write_outputs(manifest).map_err(NativeError::Build)?;
    for warning in &client.warnings {
        output::native_build_warning(&warning.code, &warning.message);
    }
    output::success("Native browser build complete");
    Ok(true)
}

async fn changed_since(changed: &Notify, epoch: &AtomicU64, current: u64) {
    loop {
        let notified = changed.notified();
        if epoch.load(Ordering::Acquire) != current {
            return;
        }
        notified.await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    #[tokio::test]
    async fn observed_edit_before_publication_discards_stale_native_build() -> Result<()> {
        let temp = tempfile::tempdir()?;
        let project = temp.path().canonicalize()?;
        let source = project.join("index.ts");
        let manifest = Path::new("dist/projection.json");
        let epoch = Arc::new(AtomicU64::new(0));
        let options = ClientBuildOptions {
            project: project.clone(),
            entries: BTreeMap::from([("index".into(), "index.ts".into())]),
            out_dir: "dist".into(),
            ..Default::default()
        };
        std::fs::write(&source, "export const message = 'initial';")?;
        let initial = build(options.clone()).await?;
        assert!(publish_current(&initial, manifest, &epoch, 0)?);
        let previous = std::fs::read(project.join("dist/index.js"))?;

        std::fs::write(&source, "export const message = 'superseded';")?;
        let (ready, built) = tokio::sync::oneshot::channel();
        let (release, proceed) = tokio::sync::oneshot::channel();
        let pending_options = options.clone();
        let pending_epoch = Arc::clone(&epoch);
        let pending = tokio::spawn(async move {
            let client = build(pending_options).await?;
            ready
                .send(())
                .map_err(|()| anyhow::anyhow!("build observer closed"))?;
            proceed.await?;
            publish_current(&client, manifest, &pending_epoch, 0)
        });
        built.await?;
        std::fs::write(&source, "export const message = 'newest';")?;
        epoch.fetch_add(1, Ordering::AcqRel);
        release
            .send(())
            .map_err(|()| anyhow::anyhow!("build gate closed"))?;
        assert!(!pending.await??);
        assert_eq!(std::fs::read(project.join("dist/index.js"))?, previous);

        let newest = build(options).await?;
        assert!(publish_current(&newest, manifest, &epoch, 1)?);
        let bytes = std::fs::read_to_string(project.join("dist/index.js"))?;
        assert!(bytes.contains("newest"));
        assert!(!bytes.contains("superseded"));
        Ok(())
    }
}

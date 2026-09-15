// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use tokio::sync::Notify;
use webui_dev_server::{LiveReload, WatchConfig, WatcherHandle};

use super::super::{prepare_build, publish_metafile, BuildRenderResult, RenderConfig, SharedState};
use super::inputs::{Inputs, Snapshot};
use super::worker::{BuildError, ClientBuilder};
use crate::utils::output;

#[derive(Default)]
struct Pending {
    revision: u64,
    dirty: bool,
    stopped: bool,
    failure: Option<String>,
}

struct Queue {
    pending: Mutex<Pending>,
    shared: Arc<Mutex<SharedState>>,
    changed: Notify,
}

impl Queue {
    fn invalidate(&self, paths: &[PathBuf], module: Option<&Path>) -> Result<()> {
        let mut pending = self
            .pending
            .lock()
            .map_err(|_| anyhow::anyhow!("Build queue poisoned"))?;
        if pending.stopped {
            return Ok(());
        }
        pending.revision = pending
            .revision
            .checked_add(1)
            .context("Build revision exhausted; restart the server")?;
        pending.dirty = true;
        if module.is_some_and(|module| paths.iter().any(|path| path == module)) {
            pending.failure = Some(
                "Client builder configuration changed; restart the server to load the new module"
                    .into(),
            );
        }
        let mut shared = self
            .shared
            .lock()
            .map_err(|_| anyhow::anyhow!("Render state poisoned"))?;
        shared.rebuild_pending = true;
        shared.rebuild_error = None;
        Ok(())
    }

    fn take(&self) -> Result<Option<u64>> {
        let mut pending = self
            .pending
            .lock()
            .map_err(|_| anyhow::anyhow!("Build queue poisoned"))?;
        if let Some(error) = &pending.failure {
            anyhow::bail!("{error}");
        }
        if !pending.dirty {
            return Ok(None);
        }
        pending.dirty = false;
        Ok(Some(pending.revision))
    }
}

enum Prepared {
    Reused,
    Fresh(Box<BuildRenderResult>, Option<Snapshot>),
    Changed,
}

enum AttemptError {
    Client(BuildError),
    Ssr(anyhow::Error),
}

fn input_change_detected(before: &Option<Snapshot>, after: &Option<Snapshot>) -> bool {
    before != after
}

pub(super) struct Coordinator {
    queue: Arc<Queue>,
    config: RenderConfig,
    theme: Option<String>,
    livereload: Option<LiveReload>,
    inputs: Arc<Inputs>,
    cached: Option<Snapshot>,
}

impl Coordinator {
    pub(super) fn new(
        shared: Arc<Mutex<SharedState>>,
        config: RenderConfig,
        theme: Option<String>,
        livereload: Option<LiveReload>,
        output: &Path,
    ) -> Result<Self> {
        let inputs = Arc::new(Inputs::new(&config, theme.as_deref(), output)?);
        Ok(Self {
            queue: Arc::new(Queue {
                pending: Mutex::new(Pending {
                    dirty: true,
                    ..Pending::default()
                }),
                shared,
                changed: Notify::new(),
            }),
            config,
            theme,
            livereload,
            inputs,
            cached: None,
        })
    }

    pub(super) fn watch(
        &self,
        config: WatchConfig,
        module: Option<PathBuf>,
    ) -> Result<WatcherHandle> {
        let queue = Arc::clone(&self.queue);
        webui_dev_server::spawn_watcher(config, move |paths| {
            if let Err(error) = queue.invalidate(&paths, module.as_deref()) {
                output::error(&error);
            }
            queue.changed.notify_one();
        })
    }

    pub(super) async fn run(
        &mut self,
        builder: &mut ClientBuilder,
        watching: bool,
        timeout: Duration,
    ) -> Result<()> {
        loop {
            if let Some(revision) = self.queue.take()? {
                let started = Instant::now();
                let result = tokio::time::timeout(timeout, self.prepare(builder)).await
                    .context("Client/SSR rebuild timed out; fix the builder or adjust --client-build-timeout-ms")?;
                self.finish(revision, result, started.elapsed(), watching)?;
            } else {
                let changed = async {
                    if watching {
                        self.queue.changed.notified().await;
                    } else {
                        std::future::pending::<()>().await;
                    }
                };
                tokio::select! {
                    () = changed => {}
                    status = builder.exited() => {
                        anyhow::bail!("Client builder exited unexpectedly ({}); restart the development server",
                            status.context("Cannot monitor client builder process")?);
                    }
                }
            }
        }
    }

    async fn prepare(&self, builder: &mut ClientBuilder) -> Result<Prepared, AttemptError> {
        builder.rebuild().await.map_err(AttemptError::Client)?;
        let inputs = Arc::clone(&self.inputs);
        let before = tokio::task::spawn_blocking(move || inputs.capture())
            .await
            .context("SSR input fingerprint task failed")
            .map_err(AttemptError::Ssr)?
            .map_err(AttemptError::Ssr)?;
        if before.is_some() && before == self.cached {
            return Ok(Prepared::Reused);
        }
        let mut config = self.config.clone();
        let theme = self.theme.clone();
        let livereload = self.livereload.clone();
        let inputs = Arc::clone(&self.inputs);
        tokio::task::spawn_blocking(move || {
            config.token_file = theme
                .as_deref()
                .map(|theme| crate::commands::common::load_theme(theme, &config.app_dir))
                .transpose()?;
            let result = prepare_build(&config, livereload.as_ref())?;
            let after = inputs.capture()?;
            if input_change_detected(&before, &after) {
                return Ok(Prepared::Changed);
            }
            Ok(Prepared::Fresh(Box::new(result), after))
        })
        .await
        .context("SSR compiler task failed")
        .map_err(AttemptError::Ssr)?
        .map_err(AttemptError::Ssr)
    }

    fn finish(
        &mut self,
        revision: u64,
        result: Result<Prepared, AttemptError>,
        elapsed: Duration,
        watching: bool,
    ) -> Result<()> {
        let result = match result {
            Err(AttemptError::Client(BuildError::Runtime(error))) => return Err(error),
            result => result,
        };
        let queue = Arc::clone(&self.queue);
        let mut pending = queue
            .pending
            .lock()
            .map_err(|_| anyhow::anyhow!("Build queue poisoned"))?;
        if pending.stopped || pending.revision != revision {
            return Ok(());
        }
        let result = match result {
            Ok(Prepared::Changed) => {
                pending.dirty = true;
                return Ok(());
            }
            Ok(value) => value,
            Err(AttemptError::Client(BuildError::Runtime(error))) => return Err(error),
            Err(AttemptError::Client(error @ BuildError::Build(_))) => {
                return self.failed(error.into(), watching, pending);
            }
            Err(AttemptError::Ssr(error)) => return self.failed(error, watching, pending),
        };
        let reused = matches!(result, Prepared::Reused);
        let warnings = if let Prepared::Fresh(result, stamp) = result {
            if let Err(error) = publish_metafile(&self.config, &result) {
                return self.failed(error, watching, pending);
            }
            let mut shared = queue
                .shared
                .lock()
                .map_err(|_| anyhow::anyhow!("Render state poisoned"))?;
            shared.rendered_html = result.html;
            shared.css_files = result.css_files;
            shared.component_assets = result.component_assets;
            shared.protocol = Some(result.protocol);
            shared.state_data = Some(Arc::new(result.state_data));
            shared.token_css = result.token_css.map(Arc::new);
            self.cached = stamp;
            result.warnings
        } else {
            Vec::new()
        };
        {
            let mut shared = queue
                .shared
                .lock()
                .map_err(|_| anyhow::anyhow!("Render state poisoned"))?;
            shared.rebuild_pending = false;
            shared.rebuild_error = None;
        }
        if let Some(livereload) = &self.livereload {
            livereload.broadcast_reload();
        }
        drop(pending);
        output::success(&format!(
            "Rebuilt in {:.1}ms (SSR {})",
            elapsed.as_secs_f64() * 1000.0,
            if reused { "reused" } else { "compiled" }
        ));
        for warning in warnings {
            output::warning_diagnostic(&warning);
        }
        Ok(())
    }

    fn failed(
        &self,
        error: anyhow::Error,
        watching: bool,
        pending: MutexGuard<'_, Pending>,
    ) -> Result<()> {
        let message = format!("{error:#}");
        {
            let mut shared = self
                .queue
                .shared
                .lock()
                .map_err(|_| anyhow::anyhow!("Render state poisoned"))?;
            shared.rebuild_pending = false;
            shared.rebuild_error = Some(message.clone());
        }
        if let Some(livereload) = &self.livereload {
            livereload.broadcast_error(&message);
        }
        drop(pending);
        if !watching {
            return Err(error);
        }
        output::error(&error);
        Ok(())
    }

    pub(super) fn stop(&self) -> Result<()> {
        let mut pending = self
            .queue
            .pending
            .lock()
            .map_err(|_| anyhow::anyhow!("Build queue poisoned"))?;
        pending.stopped = true;
        Ok(())
    }
}

#[cfg(test)]
mod tests;

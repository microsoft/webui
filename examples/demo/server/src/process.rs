// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

use crate::registry::{AppEntry, AppRunConfig};
use std::process::{Child, Command, Stdio};

/// A handle to all spawned child processes.
pub(crate) struct ProcessManager {
    children: Vec<(String, Child)>,
}

impl ProcessManager {
    /// Spawn all app servers based on the discovered registry.
    pub fn spawn_all(apps: &[AppEntry]) -> anyhow::Result<Self> {
        let mut children = Vec::with_capacity(apps.len() * 2);

        for app in apps {
            // Spawn the API server first (if any), so it's ready when the
            // main server starts.
            if let Some(api_port) = app.api_port {
                if let Some(child) = spawn_api_server(app, api_port)? {
                    children.push((format!("{}-api", app.slug), child));
                }
            }

            let child = spawn_app_server(app)?;
            children.push((app.slug.clone(), child));
        }

        Ok(Self { children })
    }

    /// Kill all child processes.
    pub fn kill_all(&mut self) {
        for (name, child) in &mut self.children {
            log::info!("Stopping {name} (pid {})", child.id());
            let _ = child.kill();
            let _ = child.wait();
        }
        self.children.clear();
    }
}

impl Drop for ProcessManager {
    fn drop(&mut self) {
        self.kill_all();
    }
}

fn spawn_app_server(app: &AppEntry) -> anyhow::Result<Child> {
    match &app.config {
        AppRunConfig::WebuiCli {
            plugin,
            source,
            servedir,
            state,
            theme,
        } => {
            let mut cmd = Command::new("webui");
            cmd.arg("serve")
                .arg(source)
                .arg("--servedir")
                .arg(servedir)
                .arg("--port")
                .arg(app.port.to_string())
                .arg("--plugin")
                .arg(plugin);

            if let Some(state) = state {
                cmd.arg("--state").arg(state);
            }
            if let Some(theme) = theme {
                cmd.arg("--theme").arg(theme);
            }
            if let Some(api_port) = app.api_port {
                cmd.arg("--api-port").arg(api_port.to_string());
            }

            // Pass base path for sub-path deployment
            let base = format!("/{}/", app.slug);
            cmd.arg("--base-path").arg(&base);

            cmd.current_dir(&app.dir)
                .stdout(Stdio::inherit())
                .stderr(Stdio::inherit());

            log::info!(
                "Starting {} (webui serve, port {}, dir {:?})",
                app.slug,
                app.port,
                app.dir
            );

            cmd.spawn()
                .map_err(|e| anyhow::anyhow!("Failed to start webui for {}: {e}", app.slug))
        }
        AppRunConfig::CustomBinary { binary, args } => {
            let mut cmd = Command::new(binary);
            cmd.arg("--port").arg(app.port.to_string());
            cmd.arg("--base-path").arg(format!("/{}/", app.slug));
            for arg in args {
                cmd.arg(arg);
            }

            cmd.current_dir(&app.dir)
                .stdout(Stdio::inherit())
                .stderr(Stdio::inherit());

            log::info!(
                "Starting {} ({binary}, port {}, dir {:?})",
                app.slug,
                app.port,
                app.dir
            );

            cmd.spawn()
                .map_err(|e| anyhow::anyhow!("Failed to start {binary} for {}: {e}", app.slug))
        }
    }
}

fn spawn_api_server(app: &AppEntry, api_port: u16) -> anyhow::Result<Option<Child>> {
    // Re-read the demo.toml to get the API config (we don't store it in AppEntry
    // to keep AppEntry Clone-friendly).
    let toml_path = app.dir.join("demo.toml");
    let toml_str = std::fs::read_to_string(&toml_path)?;
    let config: crate::registry::AppConfig = toml::from_str(&toml_str)?;

    let api = match config.api {
        Some(api) => api,
        None => return Ok(None),
    };

    match api.api_type.as_str() {
        "node" => {
            let mut cmd = Command::new("node");
            cmd.arg(&api.entry);
            cmd.env("PORT", api_port.to_string());
            cmd.current_dir(&app.dir)
                .stdout(Stdio::inherit())
                .stderr(Stdio::inherit());

            log::info!(
                "Starting {}-api (node {}, port {api_port})",
                app.slug,
                api.entry
            );

            let child = cmd
                .spawn()
                .map_err(|e| anyhow::anyhow!("Failed to start Node API for {}: {e}", app.slug))?;
            Ok(Some(child))
        }
        other => {
            log::warn!("Unknown API type '{other}' for {}, skipping", app.slug);
            Ok(None)
        }
    }
}

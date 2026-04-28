// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// Metadata loaded from an app's `demo.toml` file.
#[derive(Debug, Deserialize)]
pub(crate) struct AppConfig {
    pub name: String,
    pub description: String,
    pub backend: String,
    pub server: ServerConfig,
    pub api: Option<ApiConfig>,
}

/// Server configuration for the app.
#[derive(Debug, Deserialize)]
pub(crate) struct ServerConfig {
    #[serde(rename = "type")]
    pub server_type: String,
    pub plugin: Option<String>,
    pub source: Option<String>,
    pub servedir: Option<String>,
    pub binary: Option<String>,
    pub args: Option<Vec<String>>,
    pub state: Option<String>,
    pub theme: Option<String>,
}

/// Optional API server configuration.
#[derive(Debug, Deserialize)]
pub(crate) struct ApiConfig {
    #[serde(rename = "type")]
    pub api_type: String,
    pub entry: String,
    #[serde(rename = "port-offset", default = "default_port_offset")]
    pub port_offset: u16,
}

fn default_port_offset() -> u16 {
    10
}

/// Runtime representation of a discovered app with assigned ports.
#[derive(Debug, Clone, Serialize)]
pub(crate) struct AppEntry {
    pub name: String,
    pub slug: String,
    pub description: String,
    pub backend: String,
    pub port: u16,
    pub api_port: Option<u16>,
    #[serde(skip)]
    pub dir: PathBuf,
    #[serde(skip)]
    pub config: AppRunConfig,
}

/// Pre-computed runtime configuration derived from the `demo.toml`.
#[derive(Debug, Clone)]
pub(crate) enum AppRunConfig {
    WebuiCli {
        plugin: String,
        source: String,
        servedir: String,
        state: Option<String>,
        theme: Option<String>,
    },
    CustomBinary {
        binary: String,
        args: Vec<String>,
    },
}

impl AppEntry {
    /// GitHub source URL for this app.
    pub fn source_url(&self) -> String {
        format!(
            "https://github.com/microsoft/webui/tree/main/examples/app/{}",
            self.slug
        )
    }
}

/// Scan `apps_dir` for subdirectories containing `demo.toml` and build
/// the runtime registry with dynamically assigned ports.
pub(crate) fn discover(apps_dir: &Path, base_port: u16) -> anyhow::Result<Vec<AppEntry>> {
    let mut entries = Vec::new();

    let mut dirs: Vec<_> = std::fs::read_dir(apps_dir)?
        .filter_map(|e| e.ok())
        .filter(|e| e.path().is_dir())
        .collect();
    dirs.sort_by_key(|d| d.file_name());

    for (idx, dir_entry) in dirs.iter().enumerate() {
        let dir = dir_entry.path();
        let toml_path = dir.join("demo.toml");

        if !toml_path.exists() {
            log::debug!(
                "Skipping {:?}: no demo.toml",
                dir.file_name().unwrap_or_default()
            );
            continue;
        }

        let toml_str = std::fs::read_to_string(&toml_path)?;
        let config: AppConfig = toml::from_str(&toml_str)
            .map_err(|e| anyhow::anyhow!("Failed to parse {}: {e}", toml_path.display()))?;

        let slug = dir
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .to_string();

        let idx_offset = u16::try_from(idx * 2)
            .map_err(|_| anyhow::anyhow!("Too many apps: port space exhausted"))?;
        let port = base_port + idx_offset;
        let api_port = config.api.as_ref().map(|a| port + a.port_offset);

        let run_config = match config.server.server_type.as_str() {
            "webui-cli" => AppRunConfig::WebuiCli {
                plugin: config.server.plugin.unwrap_or_else(|| "webui".to_string()),
                source: config.server.source.unwrap_or_else(|| "src".to_string()),
                servedir: config.server.servedir.unwrap_or_else(|| "dist".to_string()),
                state: config.server.state,
                theme: config.server.theme,
            },
            "custom-binary" => AppRunConfig::CustomBinary {
                binary: config.server.binary.ok_or_else(|| {
                    anyhow::anyhow!("{slug}: custom-binary requires 'binary' field")
                })?,
                args: config.server.args.unwrap_or_default(),
            },
            other => anyhow::bail!("{slug}: unknown server type '{other}'"),
        };

        log::info!(
            "Discovered app: {} ({slug}) → port {port}{}",
            config.name,
            api_port.map_or(String::new(), |p| format!(", API port {p}"))
        );

        entries.push(AppEntry {
            name: config.name,
            slug,
            description: config.description,
            backend: config.backend,
            port,
            api_port,
            dir,
            config: run_config,
        });
    }

    if entries.is_empty() {
        anyhow::bail!("No apps found in {}", apps_dir.display());
    }

    Ok(entries)
}

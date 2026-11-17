// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

use std::path::{Component, Path, PathBuf};

use anyhow::Result;
use serde::{Deserialize, Serialize};

use crate::commands::init::Server;
use crate::commands::native::{self, NativeError};

#[derive(Deserialize, Serialize)]
#[serde(rename_all = "kebab-case", deny_unknown_fields)]
pub(super) struct Config {
    pub schema: u32,
    pub version: String,
    pub server: Server,
    pub app: PathBuf,
    pub entry: String,
    pub browser: Option<PathBuf>,
    pub manifest: PathBuf,
    pub target: String,
    pub host: PathBuf,
}

pub(super) fn starter(server: Server) -> Config {
    let (manifest, host, target) = match server {
        Server::Rust => ("Cargo.toml", "src/main.rs", "webui-app"),
        Server::Go => ("go.mod", "cmd/server", "webui-app"),
        Server::Dotnet => ("App.csproj", "Program.cs", "App"),
        Server::C => ("CMakeLists.txt", "main.c", "webui-app"),
        Server::Cpp => ("CMakeLists.txt", "main.cpp", "webui-app"),
        Server::Node => ("package.json", "server/main.mjs", "webui-app"),
        Server::Bun => ("package.json", "server/main.ts", "webui-app"),
    };
    Config {
        schema: 1,
        version: env!("CARGO_PKG_VERSION").into(),
        server,
        app: "web".into(),
        entry: "index.html".into(),
        browser: Some("index.ts".into()),
        manifest: manifest.into(),
        target: target.into(),
        host: host.into(),
    }
}

pub(super) struct Project {
    pub root: PathBuf,
    pub config: Config,
}

impl Project {
    pub(super) fn load(path: &Path) -> Result<Self> {
        let root = std::fs::canonicalize(path).map_err(|source| native::io(path, source))?;
        let bytes = native::read_metadata(&root.join("webui.toml"))?;
        let text =
            std::str::from_utf8(&bytes).map_err(|error| NativeError::Config(error.to_string()))?;
        let config: Config =
            toml::from_str(text).map_err(|error| NativeError::Config(error.to_string()))?;
        if config.schema != 1 || config.version != env!("CARGO_PKG_VERSION") {
            return Err(NativeError::Config(format!(
                "expected schema 1 and WebUI {}, received schema {} and WebUI {}",
                env!("CARGO_PKG_VERSION"),
                config.schema,
                config.version
            ))
            .into());
        }
        for path in [
            &config.app,
            Path::new(&config.entry),
            &config.manifest,
            &config.host,
        ] {
            validate_relative(path)?;
        }
        if let Some(browser) = &config.browser {
            validate_relative(browser)?;
        }
        if config.target.is_empty()
            || config.target.starts_with('-')
            || config.target.contains(['/', '\\', '\0'])
        {
            return Err(NativeError::Config(
                "target must be one native target name, not a path or option".into(),
            )
            .into());
        }
        Ok(Self { root, config })
    }

    pub(super) fn source(&self) -> PathBuf {
        self.root.join(&self.config.app)
    }
}

pub(super) fn validate_relative(path: &Path) -> Result<(), NativeError> {
    if path.as_os_str().is_empty()
        || path
            .components()
            .any(|part| !matches!(part, Component::Normal(_) | Component::CurDir))
    {
        return Err(NativeError::Config(format!(
            "application paths must be project-relative without parent segments: {}",
            path.display()
        )));
    }
    Ok(())
}

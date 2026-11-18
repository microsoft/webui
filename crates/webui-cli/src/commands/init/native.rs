// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

use anyhow::Result;

use super::{InitArgs, Server};

pub(super) fn adapt(
    files: Vec<(String, String)>,
    args: &InitArgs,
) -> Result<Vec<(String, String)>> {
    let mut native = Vec::with_capacity(files.len());
    for (path, content) in files {
        let path = if let Some(source) = path.strip_prefix("src/") {
            format!("web/{source}")
        } else if let Some(host) = path.strip_prefix("server/") {
            match args.server {
                Server::Rust => host.to_owned(),
                Server::Go if host == "go.mod" => "go.mod".to_owned(),
                Server::Go => format!("cmd/server/{host}"),
                Server::C | Server::Cpp | Server::Dotnet => host.to_owned(),
                Server::Node | Server::Bun => path,
            }
        } else {
            path
        };
        native.push((path, content));
    }
    native.push((
        "webui.toml".into(),
        crate::commands::app::starter_configuration(args.server)?,
    ));
    native.push((
        "tsconfig.json".into(),
        include_str!("templates/native/tsconfig.json").into(),
    ));
    native.push((
        "README.md".into(),
        include_str!("templates/native/README.md").into(),
    ));
    Ok(native)
}

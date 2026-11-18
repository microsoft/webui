// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::Result;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use super::config::{validate_relative, Project};
use super::process;
use crate::commands::init::Server;
use crate::commands::native::control::Stop;
use crate::commands::native::NativeError;
use crate::commands::{native, sdk};

#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct Record {
    schema: u32,
    version: String,
    server: Server,
    executable: PathBuf,
    executable_hash: String,
    protocol_hash: String,
}

fn command(project: &Project, program: &str) -> Result<Command> {
    let mut command = Command::new(program);
    command.current_dir(&project.root);
    if matches!(project.config.server, Server::Go | Server::C | Server::Cpp) {
        sdk::configure(&mut command, &project.root)?;
    }
    if project.config.server == Server::Go {
        command.env("WEBUI_APP_ROOT", &project.root);
    }
    Ok(command)
}

pub(super) async fn check(project: &Project, stop: &mut Stop) -> Result<()> {
    match project.config.server {
        Server::Rust => {
            native::ensure_directory(&project.root, Path::new("target"))?;
            let mut command = command(project, "cargo")?;
            command
                .args(["check", "--all-targets", "--manifest-path"])
                .arg(project.root.join(&project.config.manifest))
                .arg("--target-dir")
                .arg(project.root.join("target"))
                .args(["--color", "never"]);
            process::run(command, stop).await
        }
        Server::Go => {
            restore_go(project, stop).await?;
            let mut command = command(project, "go")?;
            command.args(["test", "./..."]);
            process::run(command, stop).await
        }
        Server::Node => {
            let mut command = command(project, "node")?;
            command.arg("--check").arg(&project.config.host);
            process::run(command, stop).await
        }
        Server::Bun => {
            let mut command = command(project, "bun")?;
            command
                .args(["build", "--target=bun", "--outdir"])
                .arg(project.root.join(".webui/host/check"))
                .arg(&project.config.host);
            process::run(command, stop).await
        }
        Server::C | Server::Cpp | Server::Dotnet => build(project, stop).await.map(|_| ()),
    }
}

async fn restore_go(project: &Project, stop: &mut Stop) -> Result<()> {
    let mut command = command(project, "go")?;
    command.args(["mod", "download", "all"]);
    process::run(command, stop).await
}

pub(super) async fn build(project: &Project, stop: &mut Stop) -> Result<PathBuf> {
    native::ensure_directory(&project.root, Path::new(".webui/host"))?;
    let executable = match project.config.server {
        Server::Rust => {
            native::ensure_directory(&project.root, Path::new("target"))?;
            let mut command = command(project, "cargo")?;
            command
                .args(["build", "--release", "--manifest-path"])
                .arg(project.root.join(&project.config.manifest))
                .arg("--bin")
                .arg(&project.config.target)
                .arg("--target-dir")
                .arg(project.root.join("target"))
                .args([
                    "--message-format=json-render-diagnostics",
                    "--color",
                    "never",
                ]);
            process::cargo(command, &project.config.target, stop).await?
        }
        Server::Go => {
            restore_go(project, stop).await?;
            let executable = executable_name(project, ".webui/host");
            let mut command = command(project, "go")?;
            command.arg("build").arg("-o").arg(&executable).arg(format!(
                "./{}",
                project.config.host.to_string_lossy().replace('\\', "/")
            ));
            process::run(command, stop).await?;
            executable
        }
        Server::Dotnet => {
            let output = project.root.join(".webui/host/dotnet");
            let mut command = command(project, "dotnet")?;
            command
                .arg("build")
                .arg(&project.config.manifest)
                .args(["--configuration", "Release", "--output"])
                .arg(&output);
            process::run(command, stop).await?;
            output.join(format!("{}.dll", project.config.target))
        }
        Server::C | Server::Cpp => {
            let build = project.root.join(".webui/host/cmake");
            let output = project.root.join(".webui/host/native");
            let source = project
                .config
                .manifest
                .parent()
                .unwrap_or_else(|| Path::new("."));
            let mut configure = command(project, "cmake")?;
            configure
                .arg("-S")
                .arg(project.root.join(source))
                .arg("-B")
                .arg(&build)
                .arg("-DCMAKE_BUILD_TYPE=Release")
                .arg(format!(
                    "-DCMAKE_RUNTIME_OUTPUT_DIRECTORY={}",
                    output.display()
                ))
                .arg(format!(
                    "-DCMAKE_RUNTIME_OUTPUT_DIRECTORY_RELEASE={}",
                    output.display()
                ));
            process::run(configure, stop).await?;
            let mut compile = command(project, "cmake")?;
            compile
                .arg("--build")
                .arg(&build)
                .args(["--config", "Release", "--target"])
                .arg(&project.config.target);
            process::run(compile, stop).await?;
            executable_name(project, ".webui/host/native")
        }
        Server::Node | Server::Bun => project.root.join(&project.config.host),
    };
    contained_executable(project, &executable)
}

fn executable_name(project: &Project, directory: &str) -> PathBuf {
    project.root.join(directory).join(if cfg!(windows) {
        format!("{}.exe", project.config.target)
    } else {
        project.config.target.clone()
    })
}

fn contained_executable(project: &Project, executable: &Path) -> Result<PathBuf> {
    let path = fs::canonicalize(executable).map_err(|source| native::io(executable, source))?;
    if !path.starts_with(&project.root) || !path.is_file() {
        return Err(NativeError::Host(
            "native artifact must be a regular file inside the application".into(),
        )
        .into());
    }
    Ok(path)
}

pub(super) fn save(project: &Project, executable: &Path) -> Result<()> {
    let record = Record {
        schema: 1,
        version: env!("CARGO_PKG_VERSION").into(),
        server: project.config.server,
        executable: executable.strip_prefix(&project.root)?.to_owned(),
        executable_hash: file_hash(executable)?,
        protocol_hash: file_hash(&project.root.join("dist/protocol.bin"))?,
    };
    native::write_metadata(
        &project.root,
        Path::new(".webui/server-executable.json"),
        &record,
    )?;
    Ok(())
}

pub(super) fn start(project: &Project) -> Result<process::Server> {
    let path = project.root.join(".webui/server-executable.json");
    let bytes = native::read_metadata(&path)?;
    let record: Record = serde_json::from_slice(&bytes).map_err(|error| {
        NativeError::Host(format!(
            "invalid native build record; run webui app build: {error}"
        ))
    })?;
    if record.schema != 1
        || record.version != env!("CARGO_PKG_VERSION")
        || record.server != project.config.server
    {
        return Err(NativeError::Host(
            "native build record does not match webui.toml; run webui app build".into(),
        )
        .into());
    }
    validate_relative(&record.executable)?;
    let executable = contained_executable(project, &project.root.join(record.executable))?;
    if file_hash(&executable)? != record.executable_hash
        || file_hash(&project.root.join("dist/protocol.bin"))? != record.protocol_hash
    {
        return Err(NativeError::Host("native executable or protocol changed since the last complete build; run webui app build".into()).into());
    }
    let mut command = match project.config.server {
        Server::Dotnet => {
            let mut command = command(project, "dotnet")?;
            command.arg(executable);
            command
        }
        Server::Node | Server::Bun => {
            let mut command = command(project, project.config.server.name())?;
            command.arg(executable);
            command
        }
        _ => {
            let mut command = Command::new(executable);
            command.current_dir(&project.root);
            if matches!(project.config.server, Server::Go | Server::C | Server::Cpp) {
                sdk::configure(&mut command, &project.root)?;
            }
            command
        }
    };
    if project.config.server == Server::Go {
        command.env("WEBUI_APP_ROOT", &project.root);
    }
    process::Server::start(command)
}

fn file_hash(path: &Path) -> Result<String> {
    let mut file = fs::File::open(path).map_err(|source| native::io(path, source))?;
    let mut buffer = vec![0_u8; 16 * 1024];
    let mut hash = Sha256::new();
    loop {
        let length = file
            .read(&mut buffer)
            .map_err(|source| native::io(path, source))?;
        if length == 0 {
            break;
        }
        hash.update(&buffer[..length]);
    }
    let mut result = String::with_capacity(64);
    use std::fmt::Write;
    for byte in hash.finalize() {
        write!(result, "{byte:02x}")?;
    }
    Ok(result)
}

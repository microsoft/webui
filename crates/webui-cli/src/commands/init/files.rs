// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Component, Path, PathBuf};

use anyhow::{Context, Result};
use include_dir::{include_dir, Dir};

use super::{InitArgs, InitError, PackageManager};

static TEMPLATES: Dir<'_> = include_dir!("$CARGO_MANIFEST_DIR/src/commands/init/templates");

pub(super) fn destination(input: &Path) -> Result<PathBuf> {
    let absolute = if input.is_absolute() {
        input.to_owned()
    } else {
        std::env::current_dir()?.join(input)
    };
    let mut path = PathBuf::new();
    for component in absolute.components() {
        match component {
            Component::ParentDir => {
                return Err(InitError::Destination(absolute.display().to_string()).into());
            }
            Component::CurDir => continue,
            other => path.push(other.as_os_str()),
        }
        check_directory(&path)?;
    }
    if path.parent().is_none() {
        return Err(InitError::Destination(path.display().to_string()).into());
    }
    if path.exists() {
        for entry in
            fs::read_dir(&path).with_context(|| format!("failed to inspect {}", path.display()))?
        {
            if entry?.file_name() != ".git" {
                return Err(InitError::Conflict(path.display().to_string()).into());
            }
        }
    }
    Ok(path)
}

fn check_directory(path: &Path) -> Result<()> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.is_dir() && !metadata.file_type().is_symlink() => Ok(()),
        Ok(_) => Err(InitError::Destination(path.display().to_string()).into()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error).with_context(|| format!("failed to inspect {}", path.display())),
    }
}

pub(super) fn app_name(path: &Path) -> Result<String> {
    let raw = path
        .file_name()
        .and_then(|part| part.to_str())
        .ok_or_else(|| InitError::Name(path.display().to_string()))?;
    let mut name = String::with_capacity(raw.len());
    for byte in raw.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' => {
                name.push(char::from(byte.to_ascii_lowercase()));
            }
            b'-' | b'_' | b' ' if !name.is_empty() && !name.ends_with('-') => name.push('-'),
            b'-' | b'_' | b' ' => {}
            _ => return Err(InitError::Name(raw.to_owned()).into()),
        }
    }
    let name = name.trim_end_matches('-');
    let reserved = [
        "con",
        "prn",
        "aux",
        "nul",
        "com1",
        "com2",
        "com3",
        "com4",
        "com5",
        "com6",
        "com7",
        "com8",
        "com9",
        "lpt1",
        "lpt2",
        "lpt3",
        "lpt4",
        "lpt5",
        "lpt6",
        "lpt7",
        "lpt8",
        "lpt9",
        "node-modules",
    ];
    if name.is_empty()
        || name.len() > 64
        || !name.as_bytes()[0].is_ascii_alphabetic()
        || reserved.contains(&name)
    {
        return Err(InitError::Name(raw.to_owned()).into());
    }
    Ok(name.to_owned())
}

pub(super) fn generate(root: &Path, name: &str, args: &InitArgs) -> Result<Vec<String>> {
    let mut files = template_files(args.server.name())?;
    files.push(("package.json".to_owned(), manifest(name, args)?));
    if args.package_manager == PackageManager::Yarn {
        files.push((
            ".yarnrc.yml".to_owned(),
            "nodeLinker: node-modules\n".to_owned(),
        ));
    }
    files.sort_unstable_by(|a, b| a.0.cmp(&b.0));
    for pair in files.windows(2) {
        anyhow::ensure!(
            pair[0].0 != pair[1].0,
            "duplicate scaffold path {}",
            pair[0].0
        );
    }
    let mut created = Vec::with_capacity(files.len());
    for (relative, content) in files {
        let content = content
            .replace("__WEBUI_VERSION__", env!("CARGO_PKG_VERSION"))
            .replace("__RUST_VERSION__", env!("CARGO_PKG_RUST_VERSION"))
            .replace("__APP_NAME__", name)
            .replace("__SERVER__", args.server.name())
            .replace("__PACKAGE_MANAGER__", args.package_manager.name());
        write_new(root, &relative, content.as_bytes()).with_context(|| {
            format!(
                "scaffolding stopped after {} files in {}; existing files were preserved",
                created.len(),
                root.display()
            )
        })?;
        created.push(relative);
    }
    Ok(created)
}

fn template_files(server: &str) -> Result<Vec<(String, String)>> {
    let mut files = Vec::new();
    for prefix in ["common", server] {
        let directory = TEMPLATES
            .get_dir(prefix)
            .with_context(|| format!("embedded {prefix} starter templates are missing"))?;
        let mut pending = vec![directory];
        while let Some(directory) = pending.pop() {
            pending.extend(directory.dirs());
            for file in directory.files() {
                let relative = file.path().strip_prefix(prefix)?;
                let relative = relative.to_string_lossy().replace('\\', "/");
                let relative = relative.strip_suffix(".template").unwrap_or(&relative);
                let text = file
                    .contents_utf8()
                    .context("starter template is not UTF-8")?;
                files.push((relative.to_owned(), text.to_owned()));
            }
        }
    }
    Ok(files)
}

fn write_new(root: &Path, relative: &str, bytes: &[u8]) -> Result<()> {
    let path = root.join(relative);
    let parent = path.parent().context("starter file has no parent")?;
    let mut current = PathBuf::new();
    for part in parent.components() {
        anyhow::ensure!(
            !matches!(part, Component::ParentDir),
            "starter path escapes destination"
        );
        current.push(part.as_os_str());
        check_directory(&current)?;
        if !current.exists() {
            fs::create_dir(&current)
                .with_context(|| format!("failed to create {}", current.display()))?;
        }
    }
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&path)
        .with_context(|| format!("failed to create {}; file not replaced", path.display()))?;
    file.write_all(bytes)
        .with_context(|| format!("failed to write {}", path.display()))
}

fn manifest(name: &str, args: &InitArgs) -> Result<String> {
    use serde_json::{Map, Value};
    let mut manifest = Map::new();
    manifest.insert("name".into(), Value::from(name));
    manifest.insert("version".into(), Value::from("0.1.0"));
    manifest.insert("private".into(), Value::from(true));
    manifest.insert("type".into(), Value::from("module"));
    let mut scripts = Map::new();
    for action in ["dev", "build", "start", "check"] {
        scripts.insert(
            action.into(),
            Value::from(format!("node scripts/app.mjs {action}")),
        );
    }
    manifest.insert("scripts".into(), scripts.into());
    let mut dependencies = Map::new();
    dependencies.insert(
        "@microsoft/webui".into(),
        Value::from(env!("CARGO_PKG_VERSION")),
    );
    dependencies.insert(
        "@microsoft/webui-framework".into(),
        Value::from(env!("CARGO_PKG_VERSION")),
    );
    manifest.insert("dependencies".into(), dependencies.into());
    let mut dev_dependencies = Map::new();
    for (name, version) in [
        ("esbuild", "0.28.2"),
        ("typescript", "7.0.2"),
        ("terser", "5.51.2"),
    ] {
        dev_dependencies.insert(name.into(), Value::from(version));
    }
    manifest.insert("devDependencies".into(), dev_dependencies.into());
    let mut settings = Map::new();
    settings.insert("server".into(), Value::from(args.server.name()));
    manifest.insert("webui".into(), settings.into());
    Ok(serde_json::to_string_pretty(&manifest)? + "\n")
}

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
            Component::Prefix(prefix) => {
                path.push(prefix.as_os_str());
                continue;
            }
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
    let go_version = go_module_version(env!("CARGO_PKG_VERSION"))?;
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
            .replace("__WEBUI_GO_VERSION__", &go_version)
            .replace("__WEBUI_VERSION__", env!("CARGO_PKG_VERSION"))
            .replace("__RUST_VERSION__", env!("CARGO_PKG_RUST_VERSION"))
            .replace(
                "__WEBUI_GO_MODULE__",
                concat!(
                    "github.com/microsoft/webui/go/v",
                    env!("CARGO_PKG_VERSION_MAJOR")
                ),
            )
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

fn go_module_version(version: &str) -> Result<String> {
    let (core, hotfix) = match version.split_once('-') {
        Some((core, suffix)) => (
            core,
            Some(
                suffix
                    .strip_prefix("hotfix.")
                    .context("Go starters require a stable or hotfix.N WebUI release")?,
            ),
        ),
        None => (version, None),
    };
    let mut parts = core.split('.');
    for _ in 0..3 {
        release_number(
            parts
                .next()
                .context("WebUI release requires major.minor.patch")?,
        )?;
    }
    anyhow::ensure!(parts.next().is_none(), "invalid WebUI release version");
    let Some(hotfix) = hotfix else {
        return Ok(version.to_owned());
    };
    anyhow::ensure!(
        release_number(hotfix)? > 0,
        "WebUI hotfix number must be positive"
    );
    let (prefix, patch) = core
        .rsplit_once('.')
        .context("WebUI release requires a patch")?;
    let patch = release_number(patch)?
        .checked_add(1)
        .context("Go hotfix module version would overflow; choose a lower native patch")?;
    Ok(format!("{prefix}.{patch}-0.hotfix.{hotfix}"))
}

fn release_number(value: &str) -> Result<u64> {
    anyhow::ensure!(
        !value.is_empty()
            && value.bytes().all(|byte| byte.is_ascii_digit())
            && (value.len() == 1 || !value.starts_with('0')),
        "invalid WebUI release version number: {value}"
    );
    value
        .parse()
        .context("WebUI release version number exceeds u64")
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
                if relative == "scripts/sdk.mjs" && !matches!(server, "c" | "cpp" | "go") {
                    continue;
                }
                let relative = relative.strip_suffix(".template").unwrap_or(&relative);
                let text = file
                    .contents_utf8()
                    .context("starter template is not UTF-8")?;
                files.push((relative.to_owned(), text.to_owned()));
            }
        }
    }
    if server == "cpp" {
        for relative in ["server/host.h", "server/native.cmake"] {
            let file = TEMPLATES
                .get_file(format!("c/{relative}"))
                .context("embedded shared C/C++ transport template is missing")?;
            files.push((
                relative.to_owned(),
                file.contents_utf8()
                    .context("shared C/C++ template is not UTF-8")?
                    .to_owned(),
            ));
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
        if matches!(part, Component::Prefix(_)) {
            continue;
        }
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

#[cfg(test)]
mod tests {
    use super::go_module_version;

    #[test]
    fn go_requirement_maps_native_hotfixes_without_changing_sdk_identity() -> anyhow::Result<()> {
        for (native, module) in [
            ("3.1.2", "3.1.2"),
            ("3.1.2-hotfix.1", "3.1.3-0.hotfix.1"),
            ("3.1.2-hotfix.4", "3.1.3-0.hotfix.4"),
            ("3.1.9-hotfix.12", "3.1.10-0.hotfix.12"),
            ("12.34.56-hotfix.7", "12.34.57-0.hotfix.7"),
        ] {
            assert_eq!(go_module_version(native)?, module);
            let template = include_str!("templates/go/server/go.mod")
                .replace("__WEBUI_GO_VERSION__", &go_module_version(native)?);
            assert!(template.contains(&format!(" v{module}\n")));
            let sdk = include_str!("templates/common/scripts/sdk.mjs")
                .replace("__WEBUI_VERSION__", native);
            assert!(sdk.contains(&format!("const version = '{native}';")));
        }
        for invalid in [
            "3.1.18446744073709551615-hotfix.1",
            "3.1.2-hotfix.0",
            "3.1.2-hotfix.01",
            "3.1.2-alpha.1",
            "03.1.2",
            "3.1",
            "3.1.2.3",
            "3.1.2+build.1",
        ] {
            assert!(go_module_version(invalid).is_err(), "{invalid}");
        }
        Ok(())
    }
}

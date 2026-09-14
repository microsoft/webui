// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{Context, Result};
use webui_dev_server::WatchConfig;

use super::super::{watcher_ignore_paths, ServeArgs, ServePaths};

pub(in crate::commands::serve) fn input_path(path: &Path) -> Result<PathBuf> {
    match path.canonicalize() {
        Ok(path) => Ok(path),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            let parent = path
                .parent()
                .filter(|p| !p.as_os_str().is_empty())
                .unwrap_or(Path::new("."));
            let parent = parent.canonicalize().with_context(|| {
                format!(
                    "Input parent {} must exist before starting the server",
                    parent.display()
                )
            })?;
            let name = path.file_name().context("Input must name a file")?;
            Ok(parent.join(name))
        }
        Err(error) => Err(error).with_context(|| format!("Cannot resolve {}", path.display())),
    }
}

pub(super) fn module_path(args: &ServeArgs, paths: &ServePaths) -> Result<Option<PathBuf>> {
    let Some(module) = args.client_builder.as_ref() else {
        return Ok(None);
    };
    let module = expand_tilde::expand_tilde(module)?
        .canonicalize()
        .with_context(|| {
            format!(
                "Cannot resolve client builder {}; provide an existing ES module",
                module.display()
            )
        })?;
    anyhow::ensure!(module.is_file(), "Client builder must be an ES module file");
    if paths
        .serve_dir
        .as_ref()
        .is_some_and(|output| module.starts_with(output))
    {
        anyhow::bail!(
            "Client builder module must be outside --servedir; generated output is not an input"
        );
    }
    Ok(Some(module))
}

pub(super) fn client_entry(args: &ServeArgs, paths: &ServePaths) -> Result<Option<PathBuf>> {
    let Some(entry) = &args.client_entry else {
        return Ok(None);
    };
    let input = paths
        .app_dir
        .join(expand_tilde::expand_tilde(entry)?.as_ref());
    let input = input.canonicalize().with_context(|| {
        format!(
            "Cannot resolve client entry {}; add index.ts to APP or select --client-entry <FILE>",
            input.display(),
        )
    })?;
    anyhow::ensure!(input.is_file(), "Client entry must be a JS/TS file");
    anyhow::ensure!(
        matches!(
            input.extension().and_then(|value| value.to_str()),
            Some("js" | "mjs" | "cjs" | "ts" | "mts" | "cts" | "jsx" | "tsx")
        ),
        "Client entry must use a JavaScript or TypeScript extension",
    );
    anyhow::ensure!(
        !paths
            .serve_dir
            .as_ref()
            .is_some_and(|output| input.starts_with(output)),
        "Client entry must be outside --servedir output"
    );
    Ok(Some(input))
}

fn theme_path(theme: &str, app: &Path, output: &Path) -> Result<PathBuf> {
    match webui::resolve_theme_path(theme, app) {
        Ok(path) => Ok(path),
        Err(error) => {
            let candidate = expand_tilde::expand_tilde(Path::new(theme))?;
            if let Ok(path) = input_path(&candidate) {
                if !path.exists() && path.starts_with(output) {
                    return Ok(path);
                }
            }
            Err(error).context(
                "Cannot resolve --theme; choose an existing theme or generate it under --servedir",
            )
        }
    }
}

pub(super) fn watch_config(
    args: &ServeArgs,
    paths: &ServePaths,
    module: Option<&Path>,
    additional: &[PathBuf],
) -> Result<WatchConfig> {
    let output = paths.serve_dir.as_ref().context("Missing --servedir")?;
    let mut roots = vec![paths.app_dir.clone()];
    roots.extend(webui_discovery::collect_watch_paths(
        &args.app_args.components,
        &paths.app_dir,
    ));
    let mut files = args.app_args.projection_manifests.clone();
    if let Some(module) = module {
        files.push(module.to_path_buf());
    }
    if let Some(entry) = client_entry(args, paths)? {
        files.push(entry);
    }
    if let Some(state) = &paths.state_file {
        files.push(state.clone());
    }
    if let Some(theme) = &args.theme {
        files.push(theme_path(theme, &paths.app_dir, output)?);
    }
    for path in additional {
        let path = input_path(path)?;
        anyhow::ensure!(
            !path.starts_with(output),
            "Builder watchPaths must not include --servedir output"
        );
        if path.is_dir() {
            roots.push(path);
        } else {
            files.push(path);
        }
    }
    for path in &args.watch_paths {
        let path = input_path(
            &paths
                .app_dir
                .join(expand_tilde::expand_tilde(path)?.as_ref()),
        )?;
        anyhow::ensure!(
            !path.starts_with(output),
            "--watch-path must not include --servedir output"
        );
        if path.is_dir() {
            roots.push(path);
        } else {
            files.push(path);
        }
    }
    for root in &roots {
        anyhow::ensure!(
            !root.starts_with(output),
            "--servedir must not contain component or client input roots"
        );
    }
    let mut explicit_files = Vec::with_capacity(files.len());
    for file in files {
        let path = input_path(&expand_tilde::expand_tilde(&file)?)?;
        if !path.starts_with(output) {
            explicit_files.push(path);
        }
    }
    let mut ignore = watcher_ignore_paths(paths.metafile.as_deref());
    ignore.push(output.clone());
    Ok(WatchConfig {
        paths: roots,
        explicit_files,
        ignore,
        debounce: Duration::from_millis(20),
        retry_unchanged_when: None,
    })
}

#[cfg(test)]
#[allow(clippy::disallowed_methods)]
mod tests {
    use super::*;
    use clap::Parser;

    #[derive(Parser)]
    struct Options {
        #[command(flatten)]
        args: ServeArgs,
    }

    fn options(root: &Path) -> ServeArgs {
        let app = root.join("app");
        let output = root.join("output");
        std::fs::create_dir_all(&app).unwrap();
        std::fs::create_dir_all(&output).unwrap();
        std::fs::write(root.join("builder.mjs"), "export default () => {};").unwrap();
        Options::parse_from([
            std::ffi::OsStr::new("test"),
            app.as_os_str(),
            std::ffi::OsStr::new("--servedir"),
            output.as_os_str(),
            std::ffi::OsStr::new("--client-builder"),
            root.join("builder.mjs").as_os_str(),
        ])
        .args
    }

    #[test]
    fn generated_inputs_are_excluded_but_external_inputs_are_explicitly_watched() {
        let dir = tempfile::tempdir().unwrap();
        let mut args = options(dir.path());
        let external = dir.path().join("external.json");
        std::fs::write(&external, "{}").unwrap();
        args.state = Some(dir.path().join("output").join("state.json"));
        args.app_args.projection_manifests =
            vec![dir.path().join("output").join("projection.json")];
        let paths = ServePaths::from_args(&args).unwrap();
        let module = module_path(&args, &paths).unwrap().unwrap();
        let config = watch_config(&args, &paths, Some(&module), &[external.clone()]).unwrap();
        assert!(config.explicit_files.contains(&module));
        assert!(config
            .explicit_files
            .contains(&external.canonicalize().unwrap()));
        assert!(!config
            .explicit_files
            .iter()
            .any(|path| path.starts_with(paths.serve_dir.as_ref().unwrap())));
    }

    #[test]
    fn generated_outputs_cannot_be_declared_as_additional_inputs_or_builder_modules() {
        let dir = tempfile::tempdir().unwrap();
        let mut args = options(dir.path());
        let paths = ServePaths::from_args(&args).unwrap();
        let output = paths.serve_dir.as_ref().unwrap();
        let module = module_path(&args, &paths).unwrap().unwrap();
        assert!(watch_config(&args, &paths, Some(&module), &[output.join("index.js")]).is_err());
        let generated_module = output.join("builder.mjs");
        std::fs::write(&generated_module, "export default () => {};").unwrap();
        args.client_builder = Some(generated_module);
        assert!(module_path(&args, &paths).is_err());
    }

    #[test]
    fn builder_accepts_watch_off_and_preserves_single_entry_defaults() {
        let dir = tempfile::tempdir().unwrap();
        let args = options(dir.path());
        assert!(!args.watch);
        assert_eq!(args.app_args.entry, "index.html");
        assert_eq!(args.port, 3000);
        assert_eq!(args.client_build_timeout_ms, 120_000);
        assert!(Options::try_parse_from(["test", "--build-control", "stdio"]).is_err());
    }

    #[test]
    fn generated_themes_are_allowed_without_discarding_package_resolution_errors() {
        let directory = tempfile::tempdir().unwrap();
        let args = options(directory.path());
        let paths = ServePaths::from_args(&args).unwrap();
        let output = paths.serve_dir.as_ref().unwrap();
        let generated = output.join("tokens.json");
        assert_eq!(
            theme_path(generated.to_str().unwrap(), &paths.app_dir, output).unwrap(),
            generated
        );
        let error = theme_path("@webui-missing/fixture", &paths.app_dir, output).unwrap_err();
        assert!(format!("{error:#}").contains("Make sure the package is installed"));
    }
}

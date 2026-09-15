// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

use std::path::PathBuf;

use anyhow::{Context, Result};
use clap::Args;

use super::serve::{self, ServeArgs};
use crate::utils::output;

const OUTPUT_ROOT: &str = "node_modules/.cache/webui-dev";

/// First-class development server with built-in client bundling.
#[derive(Args)]
#[command(
    mut_arg("watch", |arg| arg.default_value("true").help("Watch inputs and reload browsers (enabled by default)")),
    mut_arg("plugin", |arg| arg.default_value("webui"))
)]
pub struct DevArgs {
    #[command(flatten)]
    pub server: ServeArgs,

    /// Build once without a file watcher or browser reload client.
    #[arg(long, conflicts_with = "watch")]
    pub no_watch: bool,
}

/// Start the default WebUI development workflow.
pub fn execute(args: &DevArgs) -> Result<()> {
    let server = prepare(args).inspect_err(output::error)?;
    let result = serve::execute(&server);
    if args.server.servedir.is_some() {
        return result;
    }
    let output = server
        .servedir
        .as_ref()
        .context("Managed development output is missing")?;
    let cleanup = std::fs::remove_dir_all(output)
        .with_context(|| {
            format!(
                "Cannot remove managed development output {}",
                output.display()
            )
        })
        .inspect_err(output::error);
    result.and(cleanup)
}

fn prepare(args: &DevArgs) -> Result<ServeArgs> {
    let mut server = args.server.clone();
    server.watch = !args.no_watch;
    if server.client_builder.is_none() && server.client_entry.is_none() {
        server.client_entry = Some(PathBuf::from("index.ts"));
    }
    if let Some(output) = &server.servedir {
        std::fs::create_dir_all(output)
            .with_context(|| format!("Cannot create development output {}", output.display()))?;
    } else {
        server.servedir = Some(create_output()?);
    }
    Ok(server)
}

fn create_output() -> Result<PathBuf> {
    let mut random = [0_u8; 16];
    getrandom::fill(&mut random).map_err(|error| {
        anyhow::anyhow!(
            "Cannot create isolated development output ({error}); select an explicit --servedir",
        )
    })?;
    let root = PathBuf::from(OUTPUT_ROOT);
    std::fs::create_dir_all(&root).context("Cannot create the development cache directory")?;
    let output = root.join(format!("{:032x}", u128::from_le_bytes(random)));
    std::fs::create_dir(&output).context("Cannot create isolated development output")?;
    output
        .canonicalize()
        .context("Cannot resolve managed development output")
}

#[cfg(test)]
#[allow(clippy::disallowed_methods)]
mod tests {
    use super::*;
    use clap::Parser;

    #[derive(Parser)]
    struct Options {
        #[command(flatten)]
        args: DevArgs,
    }

    #[test]
    fn default_dev_has_output_watch_and_webui_plugin_without_custom_module() {
        let options = Options::parse_from(["test"]);
        assert!(options.args.server.watch);
        assert!(options.args.server.servedir.is_none());
        assert!(options.args.server.app_args.plugin.is_some());
        assert!(options.args.server.client_builder.is_none());
    }

    #[test]
    fn one_time_development_build_creates_output_and_selects_standard_client_entry() {
        let dir = tempfile::tempdir().unwrap();
        let output = dir.path().join("nested").join("dev");
        let options = Options::parse_from([
            std::ffi::OsStr::new("test"),
            std::ffi::OsStr::new("--no-watch"),
            std::ffi::OsStr::new("--servedir"),
            output.as_os_str(),
        ]);
        let server = prepare(&options.args).unwrap();
        assert!(!server.watch);
        assert_eq!(server.client_entry, Some(PathBuf::from("index.ts")));
        assert!(output.is_dir());
    }

    #[test]
    fn custom_builder_is_an_escape_hatch_not_a_second_builtin_build() {
        let dir = tempfile::tempdir().unwrap();
        let options = Options::parse_from([
            std::ffi::OsStr::new("test"),
            std::ffi::OsStr::new("--client-builder"),
            std::ffi::OsStr::new("builder.mjs"),
            std::ffi::OsStr::new("--servedir"),
            dir.path().as_os_str(),
        ]);
        let server = prepare(&options.args).unwrap();
        assert_eq!(server.client_builder, Some(PathBuf::from("builder.mjs")));
        assert!(server.client_entry.is_none());
    }

    #[test]
    fn simultaneous_runs_have_distinct_managed_output_directories() {
        let first = create_output().unwrap();
        let second = create_output().unwrap();
        assert!(first.is_absolute());
        assert!(second.is_absolute());
        assert_ne!(first, second);
        std::fs::remove_dir(&first).unwrap();
        std::fs::remove_dir(&second).unwrap();
    }

    #[test]
    fn builtin_requires_the_persistent_context_api_with_actionable_guidance() {
        let directory = tempfile::tempdir().unwrap();
        let package = directory.path().join("node_modules").join("esbuild");
        let app = directory.path().join("src");
        std::fs::create_dir_all(&package).unwrap();
        std::fs::create_dir(&app).unwrap();
        std::fs::write(package.join("package.json"), r#"{"main":"index.js"}"#).unwrap();
        std::fs::write(package.join("index.js"), "module.exports = {};").unwrap();
        let context = serde_json::json!({
            "appDir": app, "outDir": directory.path().join("output"),
            "clientEntry": directory.path().join("src").join("index.ts"),
        });
        let script = format!(
            "{}\nconst assert = await import('node:assert/strict');\n\
             await assert.rejects(createBuiltinBuilder({context}), /does not support persistent contexts.*supported by @microsoft\\/webui/);",
            include_str!("serve/client_builder/builtin_builder.mjs"),
        );
        let output = std::process::Command::new("node")
            .args(["--input-type=module", "--eval"])
            .arg(script)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
}

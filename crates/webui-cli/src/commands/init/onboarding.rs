// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

use std::io::{self, IsTerminal, Write};
use std::path::Path;
use std::process::{Command, Stdio};

use anyhow::{Context, Result};

use super::{InitArgs, InitError, InitReport};
use crate::utils::output::{self, OutputFormat};

pub(super) fn run(args: &InitArgs, report: &mut InitReport) -> Result<()> {
    let interactive = !args.no_interactive
        && !matches!(output::format(), OutputFormat::Json)
        && io::stdin().is_terminal()
        && io::stderr().is_terminal();
    let install =
        args.install || (interactive && confirm("Install the generated package dependencies?")?);
    if install {
        report.dependencies = "failed";
        execute(
            args.package_manager.name(),
            &["install"],
            &report.destination,
        )?;
        report.dependencies = "installed";
        report.next_commands.remove(0);
    }
    let skill = args.ai_skill
        || (interactive && confirm("Install the existing WebUI reference skill in this project?")?);
    if skill {
        report.ai_skill = "failed";
        let mut arguments = vec![
            "--yes",
            "skills",
            "add",
            "microsoft/webui",
            "--skill",
            "webui-reference",
        ];
        if !interactive {
            arguments.push("--yes");
        }
        execute("npx", &arguments, &report.destination)?;
        report.ai_skill = "installed";
        if !install {
            output::field("Reference", &"Available after installing @microsoft/webui");
        }
    }
    Ok(())
}

fn confirm(question: &str) -> Result<bool> {
    eprint!("  {question} [y/N] ");
    io::stderr().flush()?;
    let mut input = String::new();
    io::stdin()
        .read_line(&mut input)
        .context("failed to read installation choice; help: rerun with --no-interactive")?;
    Ok(matches!(
        input.trim().to_ascii_lowercase().as_str(),
        "y" | "yes"
    ))
}

fn execute(program: &str, arguments: &[&str], directory: &Path) -> Result<()> {
    let display = format!("{program} {}", arguments.join(" "));
    #[cfg(windows)]
    if matches!(
        directory.components().next(),
        Some(std::path::Component::Prefix(prefix))
            if matches!(prefix.kind(), std::path::Prefix::UNC(..) | std::path::Prefix::VerbatimUNC(..))
    ) {
        return Err(InitError::Install(format!(
            "cannot run {display} in UNC directory {}; cmd.exe requires a drive-letter directory. \
             Move the app to a local drive or map the share to a drive letter",
            directory.display()
        ))
        .into());
    }
    let mut command = command(program);
    command.args(arguments);
    if matches!(output::format(), OutputFormat::Json) || !io::stdin().is_terminal() {
        command.stdin(Stdio::null());
    }
    let status = command
        .current_dir(directory)
        .stdout(Stdio::from(io::stderr()))
        .status()
        .map_err(|error| InitError::Install(format!("{display}: {error}")))?;
    if !status.success() {
        return Err(InitError::Install(format!("{display} exited with {status}")).into());
    }
    Ok(())
}

#[cfg(not(windows))]
fn command(program: &str) -> Command {
    Command::new(program)
}

#[cfg(windows)]
fn command(program: &str) -> Command {
    // Package-manager launchers are .cmd files. Every argument is a fixed
    // installer token, never a project path or user-provided shell fragment.
    let mut command = Command::new("cmd.exe");
    command.args(["/d", "/c", program]);
    command
}

#[cfg(all(test, windows))]
mod tests {
    use super::*;

    #[test]
    fn refuses_unc_installers_before_launching_cmd() -> Result<()> {
        for directory in [
            r"\\webui-nonexistent-host\share\my-app",
            r"\\?\UNC\webui-nonexistent-host\share\my-app",
        ] {
            let error = execute("echo", &["installer-must-not-run"], Path::new(directory))
                .err()
                .context("UNC installer must be rejected before cmd.exe can fall back")?;
            let init = error
                .downcast_ref::<InitError>()
                .context("typed installation failure")?;
            assert_eq!(init.code(), "init-install-failed");
            assert_eq!(init.exit_code(), 1);
            let message = init.to_string();
            assert!(message.contains("UNC directory"), "{message}");
            assert!(
                message.contains("map the share to a drive letter"),
                "{message}"
            );
            assert!(message.contains(directory), "{message}");
        }
        Ok(())
    }
}

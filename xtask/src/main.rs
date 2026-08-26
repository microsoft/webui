// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

mod build_examples;
mod build_wasm;
mod dev;
mod e2e;
mod e2e_approve;
mod license_headers;
mod process;
mod publish;
mod util;
mod version;
mod windows_local;

use std::process::ExitCode;
use std::time::Instant;
use util::{
    build_command, ensure_cargo_install, ensure_rustup_component, run_command, run_command_quiet,
    workspace_root,
};

fn main() -> ExitCode {
    let workspace_root = match workspace_root() {
        Ok(path) => path,
        Err(message) => {
            eprintln!("xtask error: {message}");
            return ExitCode::FAILURE;
        }
    };

    if let Err(error) = std::env::set_current_dir(&workspace_root) {
        eprintln!(
            "xtask error: failed to set current directory to workspace root {}: {}",
            workspace_root.display(),
            error
        );
        return ExitCode::FAILURE;
    }

    let args: Vec<String> = std::env::args().collect();
    let task = args.get(1).map(|s| s.as_str());

    match task {
        Some("check") => check(),
        Some("fmt") => run_steps(&[Step::FMT]),
        Some("clippy") => run_steps(&[Step::CLIPPY]),
        Some("deny") => run_steps(&[Step::DENY]),
        Some("test") => run_steps(&[Step::TEST]),
        Some("build") => run_steps(&[Step::BUILD, Step::BUILD_EXAMPLES]),
        Some("build-examples") => run_steps(&[Step::BUILD_EXAMPLES]),
        Some("build-wasm") => run_steps(&[Step::BUILD_WASM]),
        Some("docs") => run_steps(&[Step::DOCS]),
        Some("bench") => {
            let target = args.get(2).map(|s| s.as_str());
            let extra_args: Vec<&str> = args.iter().skip(3).map(String::as_str).collect();
            bench(target, &extra_args)
        }
        Some("run") => {
            let integration = args.get(2).map(|s| s.as_str());
            let app = args.get(3).map(|s| s.as_str());
            build_examples::run_integration_app(integration, app)
        }
        Some("dev") => {
            let app = args.get(2).map(|s| s.as_str());
            dev::run(app)
        }
        Some("e2e") => {
            let extra: Vec<String> = args.iter().skip(2).cloned().collect();
            e2e::run(&extra)
        }
        Some("e2e-approve") => {
            let run_id = args.get(2).map(|s| s.as_str());
            e2e_approve::run(run_id)
        }
        Some("version") => {
            let ver = args.get(2).map(|s| s.as_str());
            version::run(ver)
        }
        Some("publish-stage") => {
            let extra: Vec<String> = args.iter().skip(2).cloned().collect();
            publish::run_stage(&extra)
        }
        Some("publish-build") => {
            let extra: Vec<String> = args.iter().skip(2).cloned().collect();
            publish::run_build(&extra)
        }
        Some("build-windows-local") => {
            let extra: Vec<String> = args.iter().skip(2).cloned().collect();
            windows_local::run(&extra)
        }
        Some("license-headers") => {
            let fix = args.iter().any(|a| a == "--fix");
            if fix {
                match license_headers::fix() {
                    Ok(()) => ExitCode::SUCCESS,
                    Err(msg) => {
                        eprintln!("{msg}");
                        ExitCode::FAILURE
                    }
                }
            } else {
                run_steps(&[Step::LICENSE_HEADERS])
            }
        }
        Some("proto") => match proto_regenerate() {
            Ok(()) => {
                eprintln!(
                    "  {} proto regenerated (crates/webui-protocol/src/gen_webui.rs)",
                    console::style("✔").green(),
                );
                ExitCode::SUCCESS
            }
            Err(msg) => {
                eprintln!("proto regeneration failed: {msg}");
                ExitCode::FAILURE
            }
        },
        _ => usage(),
    }
}

fn usage() -> ExitCode {
    eprintln!(
        "Usage: cargo xtask <COMMAND>\n\n\
         Commands:\n  \
           check   Run all checks (fmt, clippy, deny, test, build, bench validate, docs)\n  \
           fmt     Check formatting\n  \
           clippy  Run clippy lints\n  \
           deny    Run cargo-deny license/advisory checks\n  \
           test    Run all tests\n  \
           build   Build the workspace\n  \
           build-examples  Build all example integrations and apps\n  \
           build-wasm  Build WASM playground module\n  \
           docs    Build the documentation site\n  \
           bench <target> [-- <extra>] [--save-baseline NAME | --baseline NAME]\n  \
                       Criterion: parser, handler, protocol, expressions, state, contact-book, streaming, all\n  \
                       Integration: node-addon, streaming-resource, streaming-e2e-ttfb, streaming-browser, lazy-hydration\n  \
                       Streaming suite: streaming-all/full\n  \
                       Baselines: --save-baseline NAME records, --baseline NAME compares\n  \
                       'all' runs each Criterion target separately so baselines are recorded per target\n  \
           dev <app>  Run example app in dev mode (server + client watch concurrently)\n  \
           e2e [--update-snapshots]  Run Playwright E2E tests for all example apps\n  \
           e2e-approve [run-id]  Download CI screenshot baselines and apply locally\n  \
           version <semver>  Update version across all Cargo.toml and package.json files\n  \
           publish-build --target <triple> [--profile release|debug] [--output <dir>] [--native-only|--python-only]  Build, stage, and optionally export one target's native artifacts and Python wheel\n  \
           publish-stage [--target <triple|all>] [--profile release] [--native-only|--pack-only]  Stage release artifacts into publish/\n  \
           build-windows-local [--target all|x64|arm64|<triple>]  Build and stage Windows MSVC artifacts locally with cargo-xwin\n  \
           license-headers [--fix]  Check (or fix) license headers in source files\n  \
           proto  Regenerate src/gen_webui.rs from proto/webui.proto"
    );
    ExitCode::SUCCESS
}

fn bench(target: Option<&str>, extra_args: &[&str]) -> ExitCode {
    // Parse our own --save-baseline NAME / --baseline NAME flags out of
    // the extra args. These map to:
    //   * criterion benches: passed through as `--save-baseline`/`--baseline`
    //   * resource & e2e-ttfb examples: `--save NAME` / `--compare NAME`
    //   * browser & Node benches: `WEBUI_BENCH_SAVE` / `WEBUI_BENCH_COMPARE` env vars
    let mut save_baseline: Option<String> = None;
    let mut compare_baseline: Option<String> = None;
    let mut criterion_args: Vec<&str> = Vec::with_capacity(extra_args.len());
    let mut iter = extra_args.iter();
    while let Some(&a) = iter.next() {
        match a {
            "--save-baseline" => {
                if let Some(name) = iter.next() {
                    save_baseline = Some((*name).to_string());
                } else {
                    eprintln!("--save-baseline requires a NAME");
                    return ExitCode::FAILURE;
                }
            }
            "--baseline" => {
                if let Some(name) = iter.next() {
                    compare_baseline = Some((*name).to_string());
                } else {
                    eprintln!("--baseline requires a NAME");
                    return ExitCode::FAILURE;
                }
            }
            other => criterion_args.push(other),
        }
    }

    match target {
        Some("streaming-resource") => bench_resource(save_baseline, compare_baseline),
        Some("streaming-e2e-ttfb") => bench_e2e_ttfb(save_baseline, compare_baseline),
        Some("streaming-browser") => bench_browser(save_baseline, compare_baseline),
        Some("lazy-hydration") => bench_lazy_hydration(save_baseline, compare_baseline),
        Some("node-addon") | Some("webui-node") | Some("microsoft-webui-node") => {
            bench_node_addon(save_baseline, compare_baseline)
        }
        Some("streaming-all") | Some("full") => {
            // The full bench suite: criterion micro + resource + e2e + browser.
            // Each phase passes through the baseline flags.
            type BenchPhase = fn(Option<String>, Option<String>) -> ExitCode;
            let phases: &[(&str, BenchPhase)] = &[
                ("criterion (microsoft-webui)", bench_webui_criterion_phase),
                ("streaming-resource", bench_resource),
                ("streaming-e2e-ttfb", bench_e2e_ttfb),
                ("streaming-browser", bench_browser),
            ];
            for (label, f) in phases {
                eprintln!(
                    "\n{} {}",
                    console::style("▸").cyan().bold(),
                    console::style(label).bold()
                );
                let rc = f(save_baseline.clone(), compare_baseline.clone());
                if rc != ExitCode::SUCCESS {
                    eprintln!(
                        "{} {} failed; aborting --full run",
                        console::style("✘").red().bold(),
                        label
                    );
                    return rc;
                }
            }
            ExitCode::SUCCESS
        }
        Some("all") | None => bench_all_criterion(&criterion_args, save_baseline, compare_baseline),
        _ => {
            // Criterion path (existing behaviour). Pass baseline flags
            // through as criterion's native flags.
            let mut args: Vec<String> = vec!["bench".to_string()];
            match target {
                Some("parser") | Some("webui-parser") | Some("microsoft-webui-parser") => {
                    args.push("-p".into());
                    args.push("microsoft-webui-parser".into());
                }
                Some("handler") | Some("webui-handler") | Some("microsoft-webui-handler") => {
                    args.push("-p".into());
                    args.push("microsoft-webui-handler".into());
                }
                Some("protocol") | Some("webui-protocol") | Some("microsoft-webui-protocol") => {
                    args.push("-p".into());
                    args.push("microsoft-webui-protocol".into());
                }
                Some("expressions")
                | Some("webui-expressions")
                | Some("microsoft-webui-expressions") => {
                    args.push("-p".into());
                    args.push("microsoft-webui-expressions".into());
                }
                Some("state") | Some("webui-state") | Some("microsoft-webui-state") => {
                    args.push("-p".into());
                    args.push("microsoft-webui-state".into());
                }
                Some("contact-book") => {
                    args.push("-p".into());
                    args.push("microsoft-webui".into());
                    args.push("--bench".into());
                    args.push("contact_book_bench".into());
                }
                Some("streaming") => {
                    args.push("-p".into());
                    args.push("microsoft-webui".into());
                    args.push("--bench".into());
                    args.push("streaming_bench".into());
                }
                Some("all") | None => {
                    // Handled by `bench_all_criterion` before reaching this arm.
                    eprintln!("bench target was not resolved");
                    return ExitCode::FAILURE;
                }
                Some(other) => {
                    eprintln!(
                        "Unknown bench target '{other}'.\n\
                         Criterion targets: parser, handler, protocol, expressions, state, \
                         contact-book, streaming, all.\n\
                         Integration targets: node-addon, streaming-resource, \
                         streaming-e2e-ttfb, streaming-browser, lazy-hydration, \
                         streaming-all (= full)."
                    );
                    return ExitCode::FAILURE;
                }
            }
            // Pass baseline flags through to criterion via `-- --save-baseline NAME`.
            // Use the Vec-indexed marker so we add `--` exactly once.
            let needs_dash_dash =
                save_baseline.is_some() || compare_baseline.is_some() || !criterion_args.is_empty();
            if needs_dash_dash {
                args.push("--".into());
            }
            for ea in &criterion_args {
                args.push((*ea).to_string());
            }
            if let Some(name) = save_baseline.as_ref() {
                args.push("--save-baseline".into());
                args.push(name.clone());
            }
            if let Some(name) = compare_baseline.as_ref() {
                args.push("--baseline".into());
                args.push(name.clone());
            }

            let arg_refs: Vec<&str> = args.iter().map(String::as_str).collect();
            match run_command("cargo", &arg_refs, None) {
                Ok(()) => ExitCode::SUCCESS,
                Err(message) => {
                    eprintln!("bench failed: {message}");
                    ExitCode::FAILURE
                }
            }
        }
    }
}

/// Every Criterion benchmark target in the workspace, as
/// `(cargo package, bench target)` pairs.
///
/// `cargo xtask bench all` walks this table and invokes each target
/// individually. Running `cargo bench --workspace` instead would forward
/// Criterion flags such as `--save-baseline` to *every* benchable target,
/// including the libtest unit-test harnesses of libs and binaries, which
/// reject them with `Unrecognized option: 'save-baseline'` before any
/// baseline is recorded.
const CRITERION_BENCHES: &[(&str, &str)] = &[
    ("microsoft-webui-parser", "parser_bench"),
    ("microsoft-webui-handler", "handler_bench"),
    ("microsoft-webui-handler", "bootstrap_state_bench"),
    ("microsoft-webui-handler", "streaming_hydration_bench"),
    ("microsoft-webui-protocol", "protocol_bench"),
    ("microsoft-webui-expressions", "expressions_bench"),
    ("microsoft-webui-state", "state_bench"),
    ("microsoft-webui-ffi", "protocol_bench"),
    ("microsoft-webui", "contact_book_bench"),
    ("microsoft-webui", "streaming_bench"),
    ("microsoft-webui", "component_assets_bench"),
    ("microsoft-webui", "server_request_bench"),
];

fn bench_all_criterion(
    criterion_args: &[&str],
    save: Option<String>,
    compare: Option<String>,
) -> ExitCode {
    for &(package, bench_name) in CRITERION_BENCHES {
        eprintln!(
            "\n{} {} / {}",
            console::style("▸").cyan().bold(),
            console::style(package).bold(),
            console::style(bench_name).bold()
        );
        if let Err(message) = run_criterion_bench(
            package,
            bench_name,
            criterion_args,
            save.as_deref(),
            compare.as_deref(),
        ) {
            eprintln!("bench failed: {message}");
            return ExitCode::FAILURE;
        }
    }
    ExitCode::SUCCESS
}

fn run_criterion_bench(
    package: &str,
    bench_name: &str,
    criterion_args: &[&str],
    save: Option<&str>,
    compare: Option<&str>,
) -> Result<(), String> {
    let args = criterion_bench_args(package, bench_name, criterion_args, save, compare);
    let arg_refs: Vec<&str> = args.iter().map(String::as_str).collect();
    run_command("cargo", &arg_refs, None)
}

/// Build the `cargo bench` argument vector for one Criterion target.
///
/// Kept separate from process spawning so the routing of baseline flags is
/// unit-testable.
fn criterion_bench_args(
    package: &str,
    bench_name: &str,
    criterion_args: &[&str],
    save: Option<&str>,
    compare: Option<&str>,
) -> Vec<String> {
    let mut args = vec![
        "bench".to_string(),
        "-p".to_string(),
        package.to_string(),
        "--bench".to_string(),
        bench_name.to_string(),
    ];
    if save.is_some() || compare.is_some() || !criterion_args.is_empty() {
        args.push("--".to_string());
    }
    args.extend(criterion_args.iter().map(|arg| (*arg).to_string()));
    if let Some(name) = save {
        args.push("--save-baseline".to_string());
        args.push(name.to_string());
    }
    if let Some(name) = compare {
        args.push("--baseline".to_string());
        args.push(name.to_string());
    }
    args
}

fn bench_webui_criterion_phase(save: Option<String>, compare: Option<String>) -> ExitCode {
    let mut args: Vec<String> = vec![
        "bench".into(),
        "-p".into(),
        "microsoft-webui".into(),
        "--bench".into(),
        "streaming_bench".into(),
    ];
    if save.is_some() || compare.is_some() {
        args.push("--".into());
        if let Some(name) = save {
            args.push("--save-baseline".into());
            args.push(name);
        }
        if let Some(name) = compare {
            args.push("--baseline".into());
            args.push(name);
        }
    }
    let arg_refs: Vec<&str> = args.iter().map(String::as_str).collect();
    match run_command("cargo", &arg_refs, None) {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            eprintln!("bench failed: {message}");
            ExitCode::FAILURE
        }
    }
}

fn bench_resource(save: Option<String>, compare: Option<String>) -> ExitCode {
    let mut args: Vec<String> = vec![
        "run".into(),
        "--release".into(),
        "--example".into(),
        "streaming_resource_bench".into(),
        "-p".into(),
        "microsoft-webui".into(),
    ];
    if save.is_some() || compare.is_some() {
        args.push("--".into());
        if let Some(name) = save {
            args.push("--save".into());
            args.push(name);
        }
        if let Some(name) = compare {
            args.push("--compare".into());
            args.push(name);
        }
    }
    let arg_refs: Vec<&str> = args.iter().map(String::as_str).collect();
    match run_command("cargo", &arg_refs, None) {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            eprintln!("streaming-resource bench failed: {message}");
            ExitCode::FAILURE
        }
    }
}

fn bench_e2e_ttfb(save: Option<String>, compare: Option<String>) -> ExitCode {
    let mut args: Vec<String> = vec![
        "run".into(),
        "--release".into(),
        "--example".into(),
        "streaming_e2e_ttfb_bench".into(),
        "-p".into(),
        "microsoft-webui".into(),
    ];
    if save.is_some() || compare.is_some() {
        args.push("--".into());
        if let Some(name) = save {
            args.push("--save".into());
            args.push(name);
        }
        if let Some(name) = compare {
            args.push("--compare".into());
            args.push(name);
        }
    }
    let arg_refs: Vec<&str> = args.iter().map(String::as_str).collect();
    match run_command("cargo", &arg_refs, None) {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            eprintln!("streaming-e2e-ttfb bench failed: {message}");
            ExitCode::FAILURE
        }
    }
}

fn bench_browser(save: Option<String>, compare: Option<String>) -> ExitCode {
    use std::process::Command;
    let bench_dir = std::path::PathBuf::from("examples")
        .join("integration")
        .join("streaming-browser-bench");
    if !bench_dir.join("package.json").exists() {
        eprintln!("streaming-browser bench: {} not found", bench_dir.display());
        return ExitCode::FAILURE;
    }
    let mut cmd = Command::new("pnpm");
    cmd.arg("test").current_dir(&bench_dir);
    if let Some(name) = save.as_ref() {
        cmd.env("WEBUI_BENCH_SAVE", name);
    }
    if let Some(name) = compare.as_ref() {
        cmd.env("WEBUI_BENCH_COMPARE", name);
    }
    match cmd.status() {
        Ok(status) if status.success() => ExitCode::SUCCESS,
        Ok(status) => {
            eprintln!("streaming-browser bench exited with {status}");
            ExitCode::FAILURE
        }
        Err(e) => {
            eprintln!("streaming-browser bench: failed to spawn pnpm: {e}");
            ExitCode::FAILURE
        }
    }
}

fn bench_lazy_hydration(save: Option<String>, compare: Option<String>) -> ExitCode {
    use std::process::Command;
    let bench_dir = std::path::PathBuf::from("examples")
        .join("integration")
        .join("streaming-browser-bench");
    if !bench_dir.join("package.json").exists() {
        eprintln!("lazy-hydration bench: {} not found", bench_dir.display());
        return ExitCode::FAILURE;
    }
    let mut cmd = Command::new("pnpm");
    cmd.arg("test:lazy-hydration").current_dir(&bench_dir);
    if let Some(name) = save.as_ref() {
        cmd.env("WEBUI_LAZY_HYDRATION_SAVE", name);
    }
    if let Some(name) = compare.as_ref() {
        cmd.env("WEBUI_LAZY_HYDRATION_COMPARE", name);
    }
    match cmd.status() {
        Ok(status) if status.success() => ExitCode::SUCCESS,
        Ok(status) => {
            eprintln!("lazy-hydration bench exited with {status}");
            ExitCode::FAILURE
        }
        Err(error) => {
            eprintln!("lazy-hydration bench: failed to spawn pnpm: {error}");
            ExitCode::FAILURE
        }
    }
}

fn bench_node_addon(save: Option<String>, compare: Option<String>) -> ExitCode {
    if save.is_some() && compare.is_some() {
        eprintln!("node-addon bench: save and compare modes are mutually exclusive");
        return ExitCode::FAILURE;
    }

    let bench_dir = std::path::PathBuf::from("examples")
        .join("integration")
        .join("node-addon-bench");
    if !bench_dir.join("package.json").exists() {
        eprintln!("node-addon bench: {} not found", bench_dir.display());
        return ExitCode::FAILURE;
    }

    eprintln!(
        "  {} building microsoft-webui-node (release)",
        console::style("•").dim()
    );
    if let Err(message) = run_command(
        "cargo",
        &["build", "--release", "-p", "microsoft-webui-node"],
        None,
    ) {
        eprintln!("node-addon bench: release addon build failed: {message}");
        return ExitCode::FAILURE;
    }

    eprintln!(
        "  {} building @microsoft/webui package",
        console::style("•").dim()
    );
    if let Err(message) = run_command("pnpm", &["--filter", "@microsoft/webui", "build"], None) {
        eprintln!("node-addon bench: package build failed: {message}");
        return ExitCode::FAILURE;
    }

    let addon_file = if cfg!(target_os = "windows") {
        "webui_node.dll"
    } else if cfg!(target_os = "macos") {
        "libwebui_node.dylib"
    } else {
        "libwebui_node.so"
    };
    let addon_path = match std::env::current_dir() {
        Ok(root) => root.join("target").join("release").join(addon_file),
        Err(error) => {
            eprintln!("node-addon bench: failed to resolve workspace directory: {error}");
            return ExitCode::FAILURE;
        }
    };
    if !addon_path.is_file() {
        eprintln!(
            "node-addon bench: release addon not found at {}",
            addon_path.display()
        );
        return ExitCode::FAILURE;
    }

    let mut cmd = build_command("pnpm", &["run", "bench"]);
    cmd.current_dir(&bench_dir)
        .env("WEBUI_ADDON_PATH", &addon_path)
        .env_remove("WEBUI_BENCH_SAVE")
        .env_remove("WEBUI_BENCH_COMPARE")
        .env_remove("WEBUI_BENCH_QUICK");
    if let Some(name) = save.as_ref() {
        cmd.env("WEBUI_BENCH_SAVE", name);
    }
    if let Some(name) = compare.as_ref() {
        cmd.env("WEBUI_BENCH_COMPARE", name);
    }

    match cmd.status() {
        Ok(status) if status.success() => ExitCode::SUCCESS,
        Ok(status) => {
            eprintln!("node-addon bench exited with {status}");
            ExitCode::FAILURE
        }
        Err(error) => {
            eprintln!("node-addon bench: failed to spawn pnpm: {error}");
            ExitCode::FAILURE
        }
    }
}

fn check() -> ExitCode {
    let total_start = Instant::now();

    // Phase 1: Sequential lint checks (fail-fast)
    eprintln!("\n{} Phase 1 — lint", console::style("▸").cyan().bold());
    if run_steps(&[
        Step::LICENSE_HEADERS,
        Step::FMT,
        Step::CLIPPY,
        Step::PROTO_CHECK,
    ]) != ExitCode::SUCCESS
    {
        return ExitCode::FAILURE;
    }

    // Phase 2: Parallel — deny + test
    if run_parallel(&[Step::DENY, Step::TEST]) != ExitCode::SUCCESS {
        return ExitCode::FAILURE;
    }

    // Phase 3: Parallel — build + build-wasm
    if run_parallel(&[Step::BUILD, Step::BUILD_WASM]) != ExitCode::SUCCESS {
        return ExitCode::FAILURE;
    }

    // Phase 4: Parallel — examples (each independent) + bench + docs
    if run_parallel(&[Step::BUILD_EXAMPLES, Step::BENCH_VALIDATE, Step::DOCS]) != ExitCode::SUCCESS
    {
        return ExitCode::FAILURE;
    }

    let total = total_start.elapsed().as_secs_f64();
    eprintln!(
        "\n{} All checks passed {}\n",
        console::style("✨").green(),
        console::style(format!("({total:.1}s)")).dim(),
    );
    ExitCode::SUCCESS
}

// ── Proto generation ────────────────────────────────────────────────────

/// Regenerate `crates/webui-protocol/src/gen_webui.rs` from `proto/webui.proto`.
fn proto_regenerate() -> Result<(), String> {
    run_command(
        "cargo",
        &[
            "build",
            "-p",
            "microsoft-webui-protocol",
            "--features",
            "regenerate-proto",
        ],
        None,
    )
}

/// Check that the committed `gen_webui.rs` matches what prost-build would generate.
fn proto_check() -> Result<(), String> {
    use std::fs;
    use std::path::PathBuf;

    let gen_path = PathBuf::from("crates/webui-protocol/src/gen_webui.rs");
    let before =
        fs::read_to_string(&gen_path).map_err(|e| format!("failed to read gen_webui.rs: {e}"))?;

    proto_regenerate()?;

    let after = fs::read_to_string(&gen_path)
        .map_err(|e| format!("failed to read gen_webui.rs after regeneration: {e}"))?;

    if before != after {
        // Restore the original so the working tree isn't modified by the check.
        let _ = fs::write(&gen_path, &before);
        return Err(
            "gen_webui.rs is out of date with proto/webui.proto. Run: cargo xtask proto"
                .to_string(),
        );
    }

    Ok(())
}

// ── Step runner ─────────────────────────────────────────────────────────

struct Step {
    name: &'static str,
    run: fn() -> Result<(), String>,
}

impl Step {
    const LICENSE_HEADERS: Self = Self {
        name: "license-headers",
        run: license_headers::check,
    };
    const FMT: Self = Self {
        name: "fmt",
        run: || {
            ensure_rustup_component("rustfmt")?;
            run_command_quiet("cargo", &["fmt", "--all", "--check"], None)
        },
    };
    const CLIPPY: Self = Self {
        name: "clippy",
        run: || {
            ensure_rustup_component("clippy")?;
            run_command_quiet(
                "cargo",
                &["clippy", "--workspace", "--", "-D", "warnings"],
                None,
            )
        },
    };
    const DENY: Self = Self {
        name: "deny",
        run: || {
            ensure_cargo_install("cargo-deny", "cargo-deny")?;
            run_command_quiet("cargo", &["deny", "check"], None)
        },
    };
    const PROTO_CHECK: Self = Self {
        name: "proto (drift check)",
        run: proto_check,
    };
    const TEST: Self = Self {
        name: "test",
        run: || run_command_quiet("cargo", &["test", "--workspace"], None),
    };
    const BUILD: Self = Self {
        name: "build",
        run: || {
            run_command_quiet(
                "cargo",
                &["build", "--workspace", "--exclude", "xtask"],
                None,
            )
        },
    };
    const BUILD_EXAMPLES: Self = Self {
        name: "build (examples)",
        run: build_examples::run_example_builds,
    };
    const BUILD_WASM: Self = Self {
        name: "build (wasm)",
        run: build_wasm::run,
    };
    const DOCS: Self = Self {
        name: "docs",
        run: || {
            // Build the standalone webui-press binary
            run_command_quiet("cargo", &["build", "-p", "microsoft-webui-press"], None)?;
            // `@webui/docs...` builds the docs package AND all of its workspace
            // dependencies (e.g. `@microsoft/webui-framework`'s tsc compile)
            // so that esbuild can resolve their `exports` to real .js files.
            // Without this, a fresh checkout would fail with
            // "Could not resolve @microsoft/webui-framework".
            run_command_quiet("pnpm", &["--filter", "@webui/docs...", "build"], None)
        },
    };
    const BENCH_VALIDATE: Self = Self {
        name: "bench (validate)",
        run: || {
            // Use the dev profile (not the default bench profile) for the
            // criterion `--test` smoke run. The bench profile inherits the
            // release profile's `lto = "thin", codegen-units = 1`, which still
            // compiles the full optimized graph just to assert criterion's
            // `--test` entry point runs once. The dev profile reuses the
            // already-built unit-test artifacts and finishes in seconds. Real
            // benchmark measurements (via `cargo xtask bench`) continue to
            // use the release-derived bench profile so their numbers stay accurate.
            run_command_quiet(
                "cargo",
                &[
                    "bench",
                    "-p",
                    "microsoft-webui",
                    "--bench",
                    "contact_book_bench",
                    "--profile=dev",
                    "--",
                    "--test",
                ],
                None,
            )
        },
    };
}

fn run_steps(steps: &[Step]) -> ExitCode {
    for step in steps {
        let start = Instant::now();
        match (step.run)() {
            Ok(()) => {
                let elapsed = start.elapsed().as_secs_f64();
                eprintln!(
                    "  {} {} {}",
                    console::style("✔").green(),
                    step.name,
                    console::style(format!("({elapsed:.1}s)")).dim(),
                );
            }
            Err(message) => {
                let elapsed = start.elapsed().as_secs_f64();
                eprintln!(
                    "  {} {} {}",
                    console::style("✘").red().bold(),
                    step.name,
                    console::style(format!("({elapsed:.1}s)")).dim(),
                );
                print_failure_output(&message);
                return ExitCode::FAILURE;
            }
        }
    }
    ExitCode::SUCCESS
}

/// Run multiple steps in parallel threads. Waits for all to complete,
/// then reports results. Fails if any step failed.
fn run_parallel(steps: &[Step]) -> ExitCode {
    use std::thread;

    let names: Vec<&str> = steps.iter().map(|s| s.name).collect();
    eprintln!(
        "\n{} {}",
        console::style("▸").cyan().bold(),
        names.join(" + "),
    );

    let handles: Vec<_> = steps
        .iter()
        .map(|step| {
            let name = step.name;
            let run = step.run;
            thread::spawn(move || {
                let start = Instant::now();
                let result = run();
                let elapsed = start.elapsed().as_secs_f64();
                (name, result, elapsed)
            })
        })
        .collect();

    let mut all_ok = true;
    let mut failures: Vec<(&str, String)> = Vec::new();

    for handle in handles {
        match handle.join() {
            Ok((name, Ok(()), elapsed)) => {
                eprintln!(
                    "  {} {} {}",
                    console::style("✔").green(),
                    name,
                    console::style(format!("({elapsed:.1}s)")).dim(),
                );
            }
            Ok((name, Err(message), elapsed)) => {
                eprintln!(
                    "  {} {} {}",
                    console::style("✘").red().bold(),
                    name,
                    console::style(format!("({elapsed:.1}s)")).dim(),
                );
                failures.push((name, message));
                all_ok = false;
            }
            Err(_) => {
                eprintln!("  {} (thread panicked)", console::style("✘").red().bold());
                all_ok = false;
            }
        }
    }

    for (name, output) in &failures {
        print_failure_output_with_name(name, output);
    }

    if all_ok {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    }
}

fn print_failure_output(output: &str) {
    let separator = console::style("─".repeat(60)).dim();
    eprintln!("    {separator}");
    for line in output.lines().take(30) {
        eprintln!("    {line}");
    }
    let total = output.lines().count();
    if total > 30 {
        eprintln!(
            "    {} ({} more lines)",
            console::style("...").dim(),
            total - 30,
        );
    }
    eprintln!("    {separator}");
}

fn print_failure_output_with_name(name: &str, output: &str) {
    let separator = console::style("─".repeat(60)).dim();
    eprintln!(
        "\n    {} {} output:",
        console::style("✘").red().bold(),
        name,
    );
    eprintln!("    {separator}");
    for line in output.lines().take(30) {
        eprintln!("    {line}");
    }
    let total = output.lines().count();
    if total > 30 {
        eprintln!(
            "    {} ({} more lines)",
            console::style("...").dim(),
            total - 30,
        );
    }
    eprintln!("    {separator}");
}

#[cfg(test)]
mod tests {
    use super::{criterion_bench_args, CRITERION_BENCHES};

    #[test]
    fn criterion_bench_args_target_one_bench_binary() {
        let args = criterion_bench_args("microsoft-webui-parser", "parser_bench", &[], None, None);
        assert_eq!(
            args,
            vec![
                "bench",
                "-p",
                "microsoft-webui-parser",
                "--bench",
                "parser_bench"
            ]
        );
        // `--workspace` would also sweep libtest harnesses, which reject
        // Criterion's baseline flags.
        assert!(!args.iter().any(|arg| arg == "--workspace"));
    }

    #[test]
    fn criterion_bench_args_forward_save_baseline_after_separator() {
        let args = criterion_bench_args(
            "microsoft-webui-state",
            "state_bench",
            &[],
            Some("before"),
            None,
        );
        assert_eq!(
            args,
            vec![
                "bench",
                "-p",
                "microsoft-webui-state",
                "--bench",
                "state_bench",
                "--",
                "--save-baseline",
                "before"
            ]
        );
    }

    #[test]
    fn criterion_bench_args_forward_compare_baseline_after_separator() {
        let args = criterion_bench_args(
            "microsoft-webui-state",
            "state_bench",
            &[],
            None,
            Some("before"),
        );
        assert_eq!(
            args,
            vec![
                "bench",
                "-p",
                "microsoft-webui-state",
                "--bench",
                "state_bench",
                "--",
                "--baseline",
                "before"
            ]
        );
    }

    #[test]
    fn criterion_bench_args_keep_extra_args_before_baseline_flags() {
        let args = criterion_bench_args(
            "microsoft-webui",
            "streaming_bench",
            &["--quick"],
            Some("after"),
            None,
        );
        assert_eq!(
            args,
            vec![
                "bench",
                "-p",
                "microsoft-webui",
                "--bench",
                "streaming_bench",
                "--",
                "--quick",
                "--save-baseline",
                "after"
            ]
        );
    }

    #[test]
    fn criterion_bench_args_omit_separator_without_passthrough_args() {
        let args = criterion_bench_args("microsoft-webui-ffi", "protocol_bench", &[], None, None);
        assert!(!args.iter().any(|arg| arg == "--"));
    }

    #[test]
    fn criterion_bench_table_is_non_empty_and_unique() {
        assert!(!CRITERION_BENCHES.is_empty());
        let mut seen: Vec<(&str, &str)> = CRITERION_BENCHES.to_vec();
        seen.sort_unstable();
        let before = seen.len();
        seen.dedup();
        assert_eq!(
            before,
            seen.len(),
            "CRITERION_BENCHES has duplicate entries"
        );
    }

    #[test]
    fn criterion_bench_table_targets_exist_on_disk() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .map(std::path::Path::to_path_buf)
            .unwrap_or_default();
        for &(package, bench_name) in CRITERION_BENCHES {
            let crate_dir = package.replace("microsoft-", "");
            let path = root
                .join("crates")
                .join(&crate_dir)
                .join("benches")
                .join(format!("{bench_name}.rs"));
            assert!(
                path.is_file(),
                "CRITERION_BENCHES entry ({package}, {bench_name}) has no harness at {}",
                path.display()
            );
        }
    }
}

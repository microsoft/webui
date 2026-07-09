// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

//! Benchmark runner: dispatches `cargo xtask bench <target>` to the right
//! harness and threads baseline record/compare flags through in whatever
//! dialect each harness expects.
//!
//! Target families:
//!   * criterion micro-benches (`parser`, `handler`, `protocol`, `expressions`,
//!     `state`, `contact-book`, `streaming`, `all`) — run via `cargo bench`
//!     with `--save-baseline`/`--baseline`.
//!   * example harnesses (`streaming-resource`, `streaming-e2e-ttfb`,
//!     `state-cpu`) — run via `cargo run --release --example …` with
//!     `--save`/`--compare`.
//!   * `ffi-cpu` — build the Node native addon, then run the
//!     `process.cpuUsage()` harness (`--save`/`--compare`).
//!   * `streaming-browser` — Playwright suite driven by `pnpm`, with
//!     baselines selected through `WEBUI_BENCH_SAVE`/`WEBUI_BENCH_COMPARE`.
//!   * `streaming-all` / `full` — run the criterion + streaming phases in
//!     sequence, aborting on the first failure.

use std::path::PathBuf;
use std::process::{Command, ExitCode};

use crate::util::{run_command, run_command_quiet};

/// Baseline record/compare selection, parsed once from the CLI and then
/// rendered into whichever flag dialect a given harness expects.
#[derive(Clone, Default)]
struct Baseline {
    /// `--save-baseline`/`--save`/`WEBUI_BENCH_SAVE` name, if recording.
    save: Option<String>,
    /// `--baseline`/`--compare`/`WEBUI_BENCH_COMPARE` name, if comparing.
    compare: Option<String>,
}

impl Baseline {
    /// Whether either a save or compare baseline was requested.
    fn is_set(&self) -> bool {
        self.save.is_some() || self.compare.is_some()
    }

    /// Criterion dialect: `--save-baseline NAME` / `--baseline NAME`.
    fn push_criterion(&self, args: &mut Vec<String>) {
        if let Some(name) = &self.save {
            args.push("--save-baseline".into());
            args.push(name.clone());
        }
        if let Some(name) = &self.compare {
            args.push("--baseline".into());
            args.push(name.clone());
        }
    }

    /// Example/Node dialect: `--save NAME` / `--compare NAME`.
    fn push_example(&self, args: &mut Vec<String>) {
        if let Some(name) = &self.save {
            args.push("--save".into());
            args.push(name.clone());
        }
        if let Some(name) = &self.compare {
            args.push("--compare".into());
            args.push(name.clone());
        }
    }

    /// Browser dialect: environment variables on the spawned command.
    fn apply_env(&self, cmd: &mut Command) {
        if let Some(name) = &self.save {
            cmd.env("WEBUI_BENCH_SAVE", name);
        }
        if let Some(name) = &self.compare {
            cmd.env("WEBUI_BENCH_COMPARE", name);
        }
    }
}

/// Dispatch `cargo xtask bench <target> [-- <extra>] [--save-baseline NAME | --baseline NAME]`.
pub fn run(target: Option<&str>, extra_args: &[&str]) -> ExitCode {
    // Split our own baseline flags out of the passthrough args. Whatever is
    // left is forwarded to criterion targets verbatim.
    let (baseline, criterion_args) = match parse_flags(extra_args) {
        Ok(parsed) => parsed,
        Err(message) => {
            eprintln!("{message}");
            return ExitCode::FAILURE;
        }
    };

    match target {
        Some("streaming-resource") => run_example("streaming_resource_bench", &baseline),
        Some("streaming-e2e-ttfb") => run_example("streaming_e2e_ttfb_bench", &baseline),
        Some("state-cpu") => run_example("state_cpu_bench", &baseline),
        Some("streaming-browser") => run_browser(&baseline),
        Some("ffi-cpu") => run_ffi_cpu(&baseline),
        Some("streaming-all") | Some("full") => run_full(&baseline),
        other => run_criterion_target(other, &baseline, &criterion_args),
    }
}

/// Smoke-test that the criterion harness compiles and its entry point runs.
/// Used by `cargo xtask check`.
///
/// Uses the dev profile (not the default bench profile) for the criterion
/// `--test` smoke run. The bench profile inherits the release profile's
/// `lto = true, codegen-units = 1`, which spends ~40s compiling the full graph
/// just to assert criterion's `--test` entry point runs once. The dev profile
/// reuses the already-built unit-test artifacts and finishes in seconds. Real
/// benchmark measurements (via `cargo xtask bench`) continue to use the
/// unchanged bench profile so their numbers stay accurate.
pub fn validate() -> Result<(), String> {
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
}

/// Parse `--save-baseline NAME` / `--baseline NAME` out of the passthrough
/// args, returning the baseline selection plus the remaining criterion args.
fn parse_flags<'a>(extra_args: &[&'a str]) -> Result<(Baseline, Vec<&'a str>), String> {
    let mut baseline = Baseline::default();
    let mut criterion_args: Vec<&str> = Vec::with_capacity(extra_args.len());
    let mut iter = extra_args.iter();
    while let Some(&arg) = iter.next() {
        match arg {
            "--save-baseline" => {
                let name = iter
                    .next()
                    .ok_or_else(|| "--save-baseline requires a NAME".to_string())?;
                baseline.save = Some((*name).to_string());
            }
            "--baseline" => {
                let name = iter
                    .next()
                    .ok_or_else(|| "--baseline requires a NAME".to_string())?;
                baseline.compare = Some((*name).to_string());
            }
            other => criterion_args.push(other),
        }
    }
    Ok((baseline, criterion_args))
}

/// Run one of the release example harnesses
/// (`cargo run --release --example <name> -p microsoft-webui`).
fn run_example(example: &str, baseline: &Baseline) -> ExitCode {
    let mut args: Vec<String> = vec![
        "run".into(),
        "--release".into(),
        "--example".into(),
        example.into(),
        "-p".into(),
        "microsoft-webui".into(),
    ];
    if baseline.is_set() {
        args.push("--".into());
        baseline.push_example(&mut args);
    }
    cargo(&args, &format!("{example} bench"))
}

/// Run a criterion bench target (`cargo bench <scope> [--bench <name>]`).
fn run_criterion(
    scope: &[&str],
    bench: Option<&str>,
    baseline: &Baseline,
    extra: &[&str],
) -> ExitCode {
    let mut args: Vec<String> = vec!["bench".into()];
    for part in scope {
        args.push((*part).to_string());
    }
    if let Some(name) = bench {
        args.push("--bench".into());
        args.push(name.into());
    }
    // Criterion flags live after `--`. Add it once if we have anything to pass.
    if baseline.is_set() || !extra.is_empty() {
        args.push("--".into());
    }
    for arg in extra {
        args.push((*arg).to_string());
    }
    baseline.push_criterion(&mut args);
    cargo(&args, "bench")
}

/// Resolve a criterion target alias to its cargo scope and run it.
fn run_criterion_target(target: Option<&str>, baseline: &Baseline, extra: &[&str]) -> ExitCode {
    // Cargo scope + optional `--bench` name. `--workspace` runs every crate's
    // benches; `-p <crate>` scopes to one.
    let (scope, bench): (&[&str], Option<&str>) = match target {
        Some("parser" | "webui-parser" | "microsoft-webui-parser") => {
            (&["-p", "microsoft-webui-parser"], None)
        }
        Some("handler" | "webui-handler" | "microsoft-webui-handler") => {
            (&["-p", "microsoft-webui-handler"], None)
        }
        Some("protocol" | "webui-protocol" | "microsoft-webui-protocol") => {
            (&["-p", "microsoft-webui-protocol"], None)
        }
        Some("expressions" | "webui-expressions" | "microsoft-webui-expressions") => {
            (&["-p", "microsoft-webui-expressions"], None)
        }
        Some("state" | "webui-state" | "microsoft-webui-state") => {
            (&["-p", "microsoft-webui-state"], None)
        }
        Some("contact-book") => (&["-p", "microsoft-webui"], Some("contact_book_bench")),
        Some("streaming") => (&["-p", "microsoft-webui"], Some("streaming_bench")),
        Some("all") | None => (&["--workspace"], None),
        Some(other) => {
            eprintln!(
                "Unknown bench target '{other}'.\n\
                 Criterion targets: parser, handler, protocol, expressions, state, \
                 contact-book, streaming, all.\n\
                 Non-criterion targets: streaming-resource, streaming-e2e-ttfb, \
                 streaming-browser, state-cpu, ffi-cpu, streaming-all (= full)."
            );
            return ExitCode::FAILURE;
        }
    };

    run_criterion(scope, bench, baseline, extra)
}

/// The full bench suite: criterion micro + resource + e2e-ttfb + browser.
/// Each phase receives the same baseline selection; the run aborts on the
/// first failure.
fn run_full(baseline: &Baseline) -> ExitCode {
    type Phase = fn(&Baseline) -> ExitCode;
    let phases: &[(&str, Phase)] = &[
        ("criterion (microsoft-webui)", |b| {
            run_criterion(&["-p", "microsoft-webui"], Some("streaming_bench"), b, &[])
        }),
        ("streaming-resource", |b| {
            run_example("streaming_resource_bench", b)
        }),
        ("streaming-e2e-ttfb", |b| {
            run_example("streaming_e2e_ttfb_bench", b)
        }),
        ("streaming-browser", run_browser),
    ];
    for (label, phase) in phases {
        eprintln!(
            "\n{} {}",
            console::style("▸").cyan().bold(),
            console::style(label).bold()
        );
        let rc = phase(baseline);
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

/// FFI CPU benchmark: build the Node native addon (release) and run the
/// `process.cpuUsage()`-based harness against it. Measures the full
/// per-request FFI cost — napi string marshalling + parse + render +
/// per-chunk callback — which a pure-Rust bench cannot see.
fn run_ffi_cpu(baseline: &Baseline) -> ExitCode {
    // Build the addon first (produces target/release/webui_node.<dll|so|dylib>).
    eprintln!(
        "{} building native addon (microsoft-webui-node, release)…",
        console::style("▸").cyan().bold()
    );
    if let Err(message) = run_command(
        "cargo",
        &["build", "-p", "microsoft-webui-node", "--release"],
        None,
    ) {
        eprintln!("ffi-cpu bench: addon build failed: {message}");
        return ExitCode::FAILURE;
    }

    let script = PathBuf::from("crates")
        .join("webui-node")
        .join("bench")
        .join("ffi_cpu_bench.mjs");
    if !script.exists() {
        eprintln!("ffi-cpu bench: {} not found", script.display());
        return ExitCode::FAILURE;
    }

    // `--expose-gc` lets the harness force GC around its memory probe so the
    // reported RSS/heap working set is stable rather than GC-timing noise.
    let mut args: Vec<String> = vec![
        "--expose-gc".to_string(),
        script.to_string_lossy().into_owned(),
    ];
    baseline.push_example(&mut args);
    let arg_refs: Vec<&str> = args.iter().map(String::as_str).collect();
    match run_command("node", &arg_refs, None) {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            eprintln!("ffi-cpu bench failed: {message}");
            eprintln!(
                "  (requires Node.js on PATH; the addon is built to \
                 target/release/webui_node by this step)"
            );
            ExitCode::FAILURE
        }
    }
}

/// Streaming browser benchmark: run the Playwright suite via `pnpm test`.
fn run_browser(baseline: &Baseline) -> ExitCode {
    let bench_dir = PathBuf::from("examples")
        .join("integration")
        .join("streaming-browser-bench");
    if !bench_dir.join("package.json").exists() {
        eprintln!("streaming-browser bench: {} not found", bench_dir.display());
        return ExitCode::FAILURE;
    }
    let mut cmd = Command::new("pnpm");
    cmd.arg("test").current_dir(&bench_dir);
    baseline.apply_env(&mut cmd);
    match cmd.status() {
        Ok(status) if status.success() => ExitCode::SUCCESS,
        Ok(status) => {
            eprintln!("streaming-browser bench exited with {status}");
            ExitCode::FAILURE
        }
        Err(error) => {
            eprintln!("streaming-browser bench: failed to spawn pnpm: {error}");
            ExitCode::FAILURE
        }
    }
}

/// Shared cargo invocation: run `cargo <args>`, reporting `<label> failed` on
/// error.
fn cargo(args: &[String], label: &str) -> ExitCode {
    let arg_refs: Vec<&str> = args.iter().map(String::as_str).collect();
    match run_command("cargo", &arg_refs, None) {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            eprintln!("{label} failed: {message}");
            ExitCode::FAILURE
        }
    }
}

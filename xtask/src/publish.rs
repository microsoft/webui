// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

//! `cargo xtask publish-stage` — stage all release artifacts into `publish/`.
//!
//! Copies native binaries into npm and NuGet package directories (existing behavior),
//! then assembles a consolidated `publish/` folder with:
//! - `publish/native/`  — CLI binaries per platform
//! - `publish/npm/`     — `.tgz` tarballs from `pnpm pack`
//! - `publish/nuget/`   — `.nupkg` and `.snupkg` files from `dotnet pack`
//! - `publish/crates/`  — `.crate` files from `cargo package`
//! - `publish/wasm/`    — WASM modules + JS glue
//! - `publish/standalone/` — legacy direct-download native and WASM assets
//! - `publish/python/`  — pre-staged wheels plus the generated source distribution

use crate::util::{build_command, run_command_quiet};
use crate::version;
use std::fs;
use std::path::{Component, Path, PathBuf};
use std::process::ExitCode;

// ── Platform mapping ────────────────────────────────────────────────────

/// Mapping from Rust target triple to platform identifiers and binary filenames.
struct PlatformEntry {
    triple: &'static str,
    npm_package: &'static str,
    nuget_rid: &'static str,
    ffi_lib: &'static str,
    node_addon: &'static str,
    cli_binary: &'static str,
    /// Suffix appended to CLI binary in `publish/native/` (e.g. `"darwin-arm64"`).
    platform_suffix: &'static str,
    /// Exact platform tag emitted by the pinned maturin build for this target.
    python_platform_tag: &'static str,
    /// `MACOSX_DEPLOYMENT_TARGET` to pin when cross-compiling this target with
    /// `cargo zigbuild` / `maturin --zig`, so the embedded minimum OS version
    /// matches `python_platform_tag` regardless of toolchain defaults.
    /// `None` for non-Darwin targets, which never use that backend.
    macos_deployment_target: Option<&'static str>,
}

const PLATFORMS: &[PlatformEntry] = &[
    PlatformEntry {
        triple: "x86_64-unknown-linux-gnu",
        npm_package: "webui-linux-x64",
        nuget_rid: "linux-x64",
        ffi_lib: "libwebui_ffi.so",
        node_addon: "libwebui_node.so",
        cli_binary: "webui",
        platform_suffix: "linux-x64",
        python_platform_tag: "manylinux_2_17_x86_64.manylinux2014_x86_64",
        macos_deployment_target: None,
    },
    PlatformEntry {
        triple: "aarch64-unknown-linux-gnu",
        npm_package: "webui-linux-arm64",
        nuget_rid: "linux-arm64",
        ffi_lib: "libwebui_ffi.so",
        node_addon: "libwebui_node.so",
        cli_binary: "webui",
        platform_suffix: "linux-arm64",
        python_platform_tag: "manylinux_2_17_aarch64.manylinux2014_aarch64",
        macos_deployment_target: None,
    },
    PlatformEntry {
        triple: "x86_64-pc-windows-msvc",
        npm_package: "webui-win32-x64",
        nuget_rid: "win-x64",
        ffi_lib: "webui_ffi.dll",
        node_addon: "webui_node.dll",
        cli_binary: "webui.exe",
        platform_suffix: "win32-x64",
        python_platform_tag: "win_amd64",
        macos_deployment_target: None,
    },
    PlatformEntry {
        triple: "aarch64-pc-windows-msvc",
        npm_package: "webui-win32-arm64",
        nuget_rid: "win-arm64",
        ffi_lib: "webui_ffi.dll",
        node_addon: "webui_node.dll",
        cli_binary: "webui.exe",
        platform_suffix: "win32-arm64",
        python_platform_tag: "win_arm64",
        macos_deployment_target: None,
    },
    PlatformEntry {
        triple: "x86_64-apple-darwin",
        npm_package: "webui-darwin-x64",
        nuget_rid: "osx-x64",
        ffi_lib: "libwebui_ffi.dylib",
        node_addon: "libwebui_node.dylib",
        cli_binary: "webui",
        platform_suffix: "darwin-x64",
        python_platform_tag: "macosx_10_12_x86_64",
        macos_deployment_target: Some("10.12"),
    },
    PlatformEntry {
        triple: "aarch64-apple-darwin",
        npm_package: "webui-darwin-arm64",
        nuget_rid: "osx-arm64",
        ffi_lib: "libwebui_ffi.dylib",
        node_addon: "libwebui_node.dylib",
        cli_binary: "webui",
        platform_suffix: "darwin-arm64",
        python_platform_tag: "macosx_11_0_arm64",
        macos_deployment_target: Some("11.0"),
    },
];

// ── Cross-compilation backend selection ─────────────────────────────────

/// Host operating system running `publish-build`, distinct from the target
/// triple being built.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum HostOs {
    Linux,
    MacOs,
    Windows,
    Other,
}

fn current_host_os() -> HostOs {
    if cfg!(target_os = "linux") {
        HostOs::Linux
    } else if cfg!(target_os = "macos") {
        HostOs::MacOs
    } else if cfg!(target_os = "windows") {
        HostOs::Windows
    } else {
        HostOs::Other
    }
}

/// Cargo wrapper used to build a target triple's native artifacts.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Backend {
    /// Plain `cargo build`, for a host's own target and for Linux targets.
    Cargo,
    /// `cargo xwin build`, cross-compiles the two Windows MSVC targets using
    /// an embedded MSVC-compatible toolchain (headers, import libs, CRT).
    CargoXwin,
    /// `cargo zigbuild`, cross-compiles the two Apple Darwin targets using
    /// Zig as the C/Obj-C cross linker.
    CargoZigbuild,
}

impl Backend {
    /// The `cargo` subcommand inserted before `build`, e.g. `cargo xwin
    /// build`. `None` for plain `cargo build`.
    fn subcommand(self) -> Option<&'static str> {
        match self {
            Backend::Cargo => None,
            Backend::CargoXwin => Some("xwin"),
            Backend::CargoZigbuild => Some("zigbuild"),
        }
    }
}

/// Choose the cargo backend for building `triple` on `host`.
///
/// Linux is the primary cross-compilation host: it builds its own Linux
/// targets with native `cargo build`, reaches Windows MSVC through `cargo
/// xwin build`, and reaches Apple Darwin through `cargo zigbuild`. A host
/// building its own native target family — a macOS runner building
/// `*-apple-darwin`, or a Windows runner building `*-pc-windows-msvc` —
/// always uses plain `cargo build` there instead, since no cross toolchain
/// is required. This mirrors the existing `build-windows-local` behavior
/// (macOS host, Windows MSVC target, `cargo-xwin`) as one case of the same
/// rule, so both entry points can share it.
fn select_backend_for_host(host: HostOs, triple: &str) -> Backend {
    match host {
        HostOs::MacOs if triple.ends_with("-apple-darwin") => Backend::Cargo,
        HostOs::Windows if triple.ends_with("-pc-windows-msvc") => Backend::Cargo,
        _ if triple.ends_with("-linux-gnu") => Backend::Cargo,
        _ if triple.ends_with("-pc-windows-msvc") => Backend::CargoXwin,
        _ if triple.ends_with("-apple-darwin") => Backend::CargoZigbuild,
        _ => Backend::Cargo,
    }
}

pub(crate) fn select_backend(triple: &str) -> Backend {
    select_backend_for_host(current_host_os(), triple)
}

/// The `MACOSX_DEPLOYMENT_TARGET` env value to pin for `triple`, when
/// cross-compiling with `cargo zigbuild` (native build) or `maturin --zig`
/// (Python wheel). Both call sites derive the same value from `PLATFORMS`,
/// so the two build paths cannot drift apart. `None` for non-Darwin targets.
fn macos_deployment_target_env(triple: &str) -> Option<(&'static str, &'static str)> {
    PLATFORMS
        .iter()
        .find(|platform| platform.triple == triple)
        .and_then(|platform| platform.macos_deployment_target)
        .map(|version| ("MACOSX_DEPLOYMENT_TARGET", version))
}

/// Subdirectories created inside `publish/`.
const PUBLISH_SUBDIRS: &[&str] = &[
    "native",
    "npm",
    "nuget",
    "crates",
    "wasm",
    "standalone",
    "python",
];

/// Distribution name used for `microsoft-webui` Python artifact filenames.
///
/// Wheel/sdist filenames normalize the distribution name by replacing runs of
/// `-`, `_`, and `.` with a single `_` (PEP 427 / PEP 625), so `microsoft-webui`
/// becomes `microsoft_webui` on disk.
const PYTHON_DISTRIBUTION_NAME: &str = "microsoft_webui";
const PYTHON_INTERPRETER_TAG: &str = "cp311";
const PYTHON_ABI_TAG: &str = "abi3";
const WASM_VARIANT_DIRS: &[&str] = &["all", "handler", "parser"];

const STANDALONE_RELEASE_FILES: &[(&str, &str)] = &[
    ("native/webui-darwin-arm64", "webui-darwin-arm64"),
    ("native/webui-darwin-x64", "webui-darwin-x64"),
    ("native/webui-linux-arm64", "webui-linux-arm64"),
    ("native/webui-linux-x64", "webui-linux-x64"),
    ("native/webui-win32-arm64.exe", "webui-win32-arm64.exe"),
    ("native/webui-win32-x64.exe", "webui-win32-x64.exe"),
    ("wasm/all/package.json", "package.json"),
    ("wasm/all/README.md", "README.md"),
    ("wasm/all/webui_wasm_all.d.ts", "webui_wasm_all.d.ts"),
    ("wasm/all/webui_wasm_all.js", "webui_wasm_all.js"),
    ("wasm/all/webui_wasm_all_bg.wasm", "webui_wasm_all_bg.wasm"),
    (
        "wasm/all/webui_wasm_all_bg.wasm.d.ts",
        "webui_wasm_all_bg.wasm.d.ts",
    ),
    (
        "wasm/handler/webui_wasm_handler.d.ts",
        "webui_wasm_handler.d.ts",
    ),
    (
        "wasm/handler/webui_wasm_handler.js",
        "webui_wasm_handler.js",
    ),
    (
        "wasm/handler/webui_wasm_handler_bg.wasm",
        "webui_wasm_handler_bg.wasm",
    ),
    (
        "wasm/handler/webui_wasm_handler_bg.wasm.d.ts",
        "webui_wasm_handler_bg.wasm.d.ts",
    ),
    (
        "wasm/parser/webui_wasm_parser.d.ts",
        "webui_wasm_parser.d.ts",
    ),
    ("wasm/parser/webui_wasm_parser.js", "webui_wasm_parser.js"),
    (
        "wasm/parser/webui_wasm_parser_bg.wasm",
        "webui_wasm_parser_bg.wasm",
    ),
    (
        "wasm/parser/webui_wasm_parser_bg.wasm.d.ts",
        "webui_wasm_parser_bg.wasm.d.ts",
    ),
];

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum StageMode {
    Full,
    NativeOnly,
    PackOnly,
}

#[derive(Debug, Eq, PartialEq)]
struct StageOptions {
    target_triple: Option<String>,
    profile: String,
    mode: StageMode,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum BuildMode {
    Full,
    NativeOnly,
    PythonOnly,
}

impl BuildMode {
    fn includes_native(self) -> bool {
        self != Self::PythonOnly
    }

    fn includes_python(self) -> bool {
        self != Self::NativeOnly
    }
}

#[derive(Debug, Eq, PartialEq)]
struct BuildOptions {
    target_triple: String,
    profile: String,
    output_root: Option<PathBuf>,
    mode: BuildMode,
}

// ── Public entry point ──────────────────────────────────────────────────

/// Build and stage native release artifacts for one target triple.
///
/// Usage: `cargo xtask publish-build --target <triple> [--profile release|debug] [--output <dir>]`
/// Build and stage one target's release artifacts.
///
/// Usage: `cargo xtask publish-build --target <triple> [--profile release|debug] [--output <dir>] [--native-only|--python-only]`
///
/// Produces the native binaries (CLI, FFI library, Node addon) *and* the
/// `microsoft-webui` wheel for that target, so one command covers a release
/// leg. The two mode flags exist for a single reason: Linux wheels must be
/// linked against an old glibc inside a `manylinux` container, while the native
/// binaries build on the host. Those legs run `--native-only` on the host and
/// `--python-only` in the container; every other target uses one full run.
pub fn run_build(args: &[String]) -> ExitCode {
    let root = match std::env::current_dir() {
        Ok(path) => path,
        Err(error) => {
            eprintln!(
                "  {} Failed to read current directory: {error}",
                console::style("✘").red().bold(),
            );
            return ExitCode::FAILURE;
        }
    };
    let options = match parse_build_options(args) {
        Ok(options) => options,
        Err(error) => {
            eprintln!("  {} {error}", console::style("✘").red().bold());
            return ExitCode::FAILURE;
        }
    };

    if options.mode.includes_native() {
        eprintln!(
            "\n{} Building native release artifacts for {}",
            console::style("▸").cyan().bold(),
            console::style(&options.target_triple).bold(),
        );
        if let Err(error) = build_native_target(&root, &options.target_triple, &options.profile) {
            eprintln!(
                "  {} Failed to build {}: {error}",
                console::style("✘").red().bold(),
                options.target_triple,
            );
            return ExitCode::FAILURE;
        }

        if let Err(error) = stage_native_targets(
            &root,
            std::iter::once(options.target_triple.as_str()),
            &options.profile,
        ) {
            eprintln!(
                "  {} Failed to stage native release artifacts: {error}",
                console::style("✘").red().bold(),
            );
            return ExitCode::FAILURE;
        }
    }

    if options.mode.includes_python() {
        eprintln!(
            "\n{} Building Python wheel for {}",
            console::style("▸").cyan().bold(),
            console::style(&options.target_triple).bold(),
        );
        let python_out = root.join("publish").join("python");
        match build_python_wheel(&root, &options.target_triple, &python_out) {
            Ok(wheel) => eprintln!(
                "  {} Built {}",
                console::style("✔").green(),
                console::style(wheel).bold(),
            ),
            Err(error) => {
                eprintln!(
                    "  {} Failed to build the Python wheel for {}: {error}",
                    console::style("✘").red().bold(),
                    options.target_triple,
                );
                return ExitCode::FAILURE;
            }
        }
    }

    if let Some(output_root) = &options.output_root {
        if let Err(error) = export_target(&root, output_root, &options.target_triple, options.mode)
        {
            eprintln!(
                "  {} Failed to export release artifacts: {error}",
                console::style("✘").red().bold(),
            );
            return ExitCode::FAILURE;
        }
    }

    eprintln!(
        "\n{} Release artifacts built and staged\n",
        console::style("✨").green(),
    );
    ExitCode::SUCCESS
}

/// Stage release artifacts into `publish/` and package directories.
///
/// Usage: `cargo xtask publish-stage [--target <triple|all>] [--profile release] [--native-only|--pack-only]`
///
/// Pass `--target all` to stage every platform whose build artifacts exist.
/// If `--target` is omitted, detects the current host platform.
///
/// Steps:
///   1. Stage native binaries into npm/NuGet package directories (existing behavior).
///   2. Copy CLI binaries into `publish/native/` with platform suffixes.
///   3. Build WASM artifacts into the main npm package and `publish/wasm/`.
///   4. Pack npm tarballs into `publish/npm/`.
///   5. Pack NuGet packages into `publish/nuget/`.
///   6. Pack publishable Rust crates into `publish/crates/`.
///   7. Build the Python sdist and validate all pre-staged wheels.
pub fn run_stage(args: &[String]) -> ExitCode {
    let root = match std::env::current_dir() {
        Ok(p) => p,
        Err(e) => {
            eprintln!(
                "  {} Failed to read current directory: {e}",
                console::style("✘").red().bold(),
            );
            return ExitCode::FAILURE;
        }
    };

    let options = match parse_stage_options(args) {
        Ok(options) => options,
        Err(e) => {
            eprintln!("  {} {}", console::style("✘").red().bold(), e,);
            return ExitCode::FAILURE;
        }
    };

    // Read workspace version
    let ver = match version::read_version() {
        Ok(v) => v,
        Err(e) => {
            eprintln!(
                "  {} Failed to read version: {e}",
                console::style("✘").red().bold(),
            );
            return ExitCode::FAILURE;
        }
    };

    eprintln!(
        "\n{} publish-stage v{}\n",
        console::style("▸").cyan().bold(),
        console::style(&ver).bold(),
    );

    // Create publish/ directory tree
    if let Err(e) = prepare_publish_dirs(&root, options.mode) {
        eprintln!(
            "  {} Failed to create publish/ directories: {e}",
            console::style("✘").red().bold(),
        );
        return ExitCode::FAILURE;
    }

    // Phase 1: Stage native binaries (existing behavior + publish/native/)
    if options.mode != StageMode::PackOnly {
        let stage_result = match options.target_triple.as_deref() {
            Some("all") => stage_all_platforms(&root, &options.profile),
            Some(triple) => stage_one_platform(&root, triple, &options.profile),
            None => {
                let host = detect_host_triple();
                eprintln!(
                    "  {} No --target specified, using host: {}",
                    console::style("▸").cyan().bold(),
                    console::style(&host).bold(),
                );
                stage_one_platform(&root, &host, &options.profile)
            }
        };

        if stage_result != ExitCode::SUCCESS {
            return stage_result;
        }

        if options.mode == StageMode::NativeOnly {
            eprintln!(
                "\n{} Native artifacts staged in {}\n",
                console::style("✨").green(),
                console::style("publish/native").bold(),
            );
            return ExitCode::SUCCESS;
        }
    }

    // Phase 2: Stage WASM before packing the main npm package.
    eprintln!(
        "\n{} Staging WASM artifacts",
        console::style("▸").cyan().bold(),
    );
    if let Err(e) = stage_wasm_artifacts(&root) {
        eprintln!(
            "  {} WASM staging failed: {e}",
            console::style("✘").red().bold(),
        );
        return ExitCode::FAILURE;
    }

    // Phase 3: Pack npm tarballs
    eprintln!(
        "\n{} Packing npm tarballs",
        console::style("▸").cyan().bold(),
    );
    if let Err(e) = pack_npm_tarballs(&root) {
        eprintln!(
            "  {} npm pack failed: {e}",
            console::style("✘").red().bold(),
        );
        return ExitCode::FAILURE;
    }

    // Phase 4: Pack NuGet packages
    eprintln!(
        "\n{} Packing NuGet packages",
        console::style("▸").cyan().bold(),
    );
    if let Err(e) = pack_nuget_packages(&root) {
        eprintln!(
            "  {} NuGet pack failed: {e}",
            console::style("✘").red().bold(),
        );
        return ExitCode::FAILURE;
    }

    // Phase 5: Pack Rust crates
    eprintln!(
        "\n{} Packing Rust crates",
        console::style("▸").cyan().bold(),
    );
    if let Err(e) = pack_rust_crates(&root) {
        eprintln!(
            "  {} Rust crate pack failed: {e}",
            console::style("✘").red().bold(),
        );
        return ExitCode::FAILURE;
    }

    // Phase 6: Build the Python sdist (wheels are built out-of-band per
    // platform and pre-staged into `publish/python/` before this runs).
    eprintln!(
        "\n{} Packing Python distribution",
        console::style("▸").cyan().bold(),
    );
    if let Err(e) = pack_python_package(&root) {
        eprintln!(
            "  {} Python packaging failed: {e}",
            console::style("✘").red().bold(),
        );
        return ExitCode::FAILURE;
    }

    if let Err(e) = validate_release_artifact_counts(&root, &ver) {
        eprintln!(
            "  {} Release artifact validation failed: {e}",
            console::style("✘").red().bold(),
        );
        return ExitCode::FAILURE;
    }

    // Summary
    eprintln!(
        "\n{} All artifacts staged in {}\n",
        console::style("✨").green(),
        console::style("publish/").bold(),
    );

    ExitCode::SUCCESS
}

/// Stage already-built native artifacts for specific targets after one clean.
pub(crate) fn stage_native_targets<'a, I>(
    root: &Path,
    triples: I,
    profile: &str,
) -> Result<(), String>
where
    I: IntoIterator<Item = &'a str>,
{
    prepare_publish_dirs(root, StageMode::NativeOnly)?;

    for triple in triples {
        if stage_one_platform(root, triple, profile) != ExitCode::SUCCESS {
            return Err(format!("failed to stage native artifacts for {triple}"));
        }
    }

    Ok(())
}

// ── Publish directory setup ─────────────────────────────────────────────

/// Create the `publish/` directory tree with the preservation rules for `mode`.
fn prepare_publish_dirs(root: &Path, mode: StageMode) -> Result<(), String> {
    let publish_dir = root.join("publish");

    match mode {
        StageMode::Full => {
            // Wheels are built out-of-band even for a full stage. Preserve the
            // pre-staged Python directory while cleaning every other output.
            fs::create_dir_all(&publish_dir)
                .map_err(|e| format!("failed to create publish/: {e}"))?;
            let python_dir = publish_dir.join("python");
            let entries =
                fs::read_dir(&publish_dir).map_err(|e| format!("failed to read publish/: {e}"))?;
            for entry in entries {
                let entry = entry.map_err(|e| format!("failed to read publish/ entry: {e}"))?;
                let path = entry.path();
                if path != python_dir {
                    remove_publish_path(&path)?;
                }
            }

            for subdir in PUBLISH_SUBDIRS {
                fs::create_dir_all(publish_dir.join(subdir))
                    .map_err(|e| format!("failed to create publish/{subdir}: {e}"))?;
            }
        }
        StageMode::NativeOnly => {
            if publish_dir.exists() {
                fs::remove_dir_all(&publish_dir)
                    .map_err(|e| format!("failed to clean publish/: {e}"))?;
            }

            for subdir in PUBLISH_SUBDIRS {
                fs::create_dir_all(publish_dir.join(subdir))
                    .map_err(|e| format!("failed to create publish/{subdir}: {e}"))?;
            }
        }
        StageMode::PackOnly => {
            // `native/` and `python/` are populated by a prior stage/build step
            // (e.g. downloaded matrix-build pipeline artifacts) and must be
            // preserved, not cleaned, when only packing/validating.
            for subdir in ["native", "python"] {
                fs::create_dir_all(publish_dir.join(subdir))
                    .map_err(|e| format!("failed to create publish/{subdir}: {e}"))?;
            }

            for subdir in ["npm", "nuget", "crates", "wasm", "standalone"] {
                let path = publish_dir.join(subdir);
                if path.exists() {
                    fs::remove_dir_all(&path)
                        .map_err(|e| format!("failed to clean publish/{subdir}: {e}"))?;
                }
                fs::create_dir_all(&path)
                    .map_err(|e| format!("failed to create publish/{subdir}: {e}"))?;
            }
        }
    }

    Ok(())
}

fn remove_publish_path(path: &Path) -> Result<(), String> {
    let metadata = fs::symlink_metadata(path)
        .map_err(|e| format!("failed to inspect {}: {e}", path.display()))?;
    if metadata.is_dir() {
        fs::remove_dir_all(path).map_err(|e| format!("failed to clean {}: {e}", path.display()))
    } else {
        fs::remove_file(path).map_err(|e| format!("failed to clean {}: {e}", path.display()))
    }
}

fn parse_stage_options(args: &[String]) -> Result<StageOptions, String> {
    let mut target_triple = None;
    let mut profile = String::from("release");
    let mut mode = StageMode::Full;
    let mut i = 0;

    while i < args.len() {
        match args[i].as_str() {
            "--target" => {
                i += 1;
                if i >= args.len() {
                    return Err("missing value for --target".to_string());
                }
                target_triple = Some(args[i].clone());
            }
            "--profile" => {
                i += 1;
                if i >= args.len() {
                    return Err("missing value for --profile".to_string());
                }
                profile = args[i].clone();
            }
            "--native-only" => {
                mode = set_stage_mode(mode, StageMode::NativeOnly)?;
            }
            "--pack-only" => {
                mode = set_stage_mode(mode, StageMode::PackOnly)?;
            }
            _ => {}
        }
        i += 1;
    }

    Ok(StageOptions {
        target_triple,
        profile,
        mode,
    })
}

/// Locate a CPython to run `maturin` with.
///
/// This selects the interpreter that *executes* maturin, not the interpreter
/// the wheel is built for: `abi3-py311` fixes the ABI, so maturin needs no
/// target interpreter and must not be told to look for one when cross
/// compiling. Prefer an explicit `python3.11` (what the `manylinux` images
/// expose) before falling back to whatever the host calls Python.
fn python_interpreter() -> String {
    if let Ok(interpreter) = std::env::var("WEBUI_PYTHON") {
        if !interpreter.is_empty() {
            return interpreter;
        }
    }
    for candidate in ["python3.11", "python3", "python"] {
        if run_command_quiet(candidate, &["--version"], None).is_ok() {
            return candidate.to_string();
        }
    }
    "python".to_string()
}

/// Assemble the `maturin build` arguments for one target.
///
/// Deliberately omits `--interpreter`. The crate is abi3, so maturin derives
/// the ABI tag from the feature and the platform tag from `--target`. Naming an
/// interpreter makes maturin match the host interpreter against the target
/// architecture and skip it, which breaks every cross build.
///
/// `backend` adds the maturin flag for the cross toolchain it maps to
/// (`--xwin` for `CargoXwin`, `--zig` for `CargoZigbuild`); `Backend::Cargo`
/// adds nothing, since that is either a native build or a Linux target,
/// where the `manylinux` container's own cross toolchain applies instead.
fn maturin_build_args<'a>(
    manifest: &'a str,
    triple: &'a str,
    out: &'a str,
    backend: Backend,
) -> Vec<&'a str> {
    let mut args = vec![
        "-m",
        "maturin",
        "build",
        "--release",
        "--locked",
        "--manifest-path",
        manifest,
        "--target",
        triple,
        "--out",
        out,
    ];
    // Linux wheels must declare the manylinux baseline they were built against;
    // every other target derives its platform tag from the triple alone.
    if triple.ends_with("-linux-gnu") {
        args.extend_from_slice(&["--compatibility", "manylinux_2_17"]);
    }
    match backend {
        Backend::Cargo => {}
        Backend::CargoXwin => args.push("--xwin"),
        Backend::CargoZigbuild => args.push("--zig"),
    }
    args
}

fn build_python_wheel(root: &Path, triple: &str, out_dir: &Path) -> Result<String, String> {
    let platform = PLATFORMS
        .iter()
        .find(|platform| platform.triple == triple)
        .ok_or_else(|| format!("unknown target triple: {triple}"))?;

    fs::create_dir_all(out_dir)
        .map_err(|e| format!("failed to create {}: {e}", out_dir.display()))?;

    let manifest_path = root.join("crates").join("webui-python").join("Cargo.toml");
    if !manifest_path.is_file() {
        return Err(format!(
            "Python package manifest not found at {} (crates/webui-python must exist before packaging)",
            manifest_path.display()
        ));
    }

    let backend = select_backend(triple);
    preflight_backend(backend)?;

    let interpreter = python_interpreter();
    let manifest_arg = manifest_path.to_string_lossy().into_owned();
    let out_arg = out_dir.to_string_lossy().into_owned();
    let maturin_args = maturin_build_args(&manifest_arg, triple, &out_arg, backend);
    let env = macos_deployment_target_env(triple);

    run_command_quiet_with_env(&interpreter, &maturin_args, root, env.as_slice())
        .map_err(|e| format!("maturin build failed: {e}"))?;

    let expected = format!(
        "{PYTHON_DISTRIBUTION_NAME}-{}-{}.whl",
        crate::version::read_version()
            .map_err(|e| format!("failed to read the workspace version: {e}"))?,
        expected_python_wheel_tag(platform)
    );
    if !out_dir.join(&expected).is_file() {
        return Err(format!(
            "maturin did not produce {expected} in {}",
            out_dir.display()
        ));
    }
    Ok(expected)
}

fn parse_build_options(args: &[String]) -> Result<BuildOptions, String> {
    let mut target_triple = None;
    let mut profile = String::from("release");
    let mut output_root = None;
    let mut mode = BuildMode::Full;
    let mut i = 0;

    while i < args.len() {
        match args[i].as_str() {
            "--target" => {
                i += 1;
                let Some(triple) = args.get(i) else {
                    return Err("missing value for --target".to_string());
                };
                if triple == "all" {
                    return Err(
                        "publish-build requires explicit target triples; --target all is not supported"
                            .to_string(),
                    );
                }
                if !PLATFORMS.iter().any(|platform| platform.triple == triple) {
                    return Err(format!("unknown target triple: {triple}"));
                }
                if target_triple.is_some() {
                    return Err(
                        "publish-build accepts exactly one --target; use separate jobs for each target"
                            .to_string(),
                    );
                }
                target_triple = Some(triple.clone());
            }
            "--profile" => {
                i += 1;
                let Some(value) = args.get(i) else {
                    return Err("missing value for --profile".to_string());
                };
                if value != "release" && value != "debug" {
                    return Err(format!(
                        "unsupported publish-build profile: {value}; expected release or debug"
                    ));
                }
                profile.clone_from(value);
            }
            "--output" => {
                i += 1;
                let Some(value) = args.get(i) else {
                    return Err("missing value for --output".to_string());
                };
                output_root = Some(PathBuf::from(value));
            }
            "--native-only" => mode = set_build_mode(mode, BuildMode::NativeOnly)?,
            "--python-only" => mode = set_build_mode(mode, BuildMode::PythonOnly)?,
            argument => return Err(format!("unknown publish-build argument: {argument}")),
        }
        i += 1;
    }

    let target_triple =
        target_triple.ok_or_else(|| "publish-build requires one --target".to_string())?;

    Ok(BuildOptions {
        target_triple,
        profile,
        output_root,
        mode,
    })
}

fn set_build_mode(current: BuildMode, requested: BuildMode) -> Result<BuildMode, String> {
    if current != BuildMode::Full && current != requested {
        return Err("cannot combine --native-only and --python-only".to_string());
    }

    Ok(requested)
}

fn build_native_target(root: &Path, triple: &str, profile: &str) -> Result<(), String> {
    let backend = select_backend(triple);
    preflight_backend(backend)?;

    let args = native_build_args(triple, profile, backend)?;
    let env = macos_deployment_target_env(triple);
    run_command_with_env("cargo", &args, root, env.as_slice())
}

/// Assemble the `cargo build` arguments for one target, prefixed with the
/// backend's cargo subcommand (e.g. `xwin`, `zigbuild`) when it needs one.
///
/// `pub(crate)` so `windows_local::build_target` can share it for the
/// `CargoXwin` backend instead of duplicating the argument list.
pub(crate) fn native_build_args<'a>(
    triple: &'a str,
    profile: &str,
    backend: Backend,
) -> Result<Vec<&'a str>, String> {
    let mut args = Vec::with_capacity(14);
    if let Some(subcommand) = backend.subcommand() {
        args.push(subcommand);
    }
    args.push("build");
    match profile {
        "release" => args.push("--release"),
        "debug" => {}
        _ => return Err(format!("unsupported native build profile: {profile}")),
    }
    args.extend_from_slice(&[
        "--target",
        triple,
        "-p",
        "microsoft-webui-cli",
        "-p",
        "microsoft-webui-ffi",
        "-p",
        "microsoft-webui-node",
    ]);
    Ok(args)
}

/// Actionable preflight checks for the backend chosen for a target, so a
/// missing or mis-pinned cross toolchain fails immediately with install
/// guidance instead of a confusing linker error partway through the build.
/// Reuses `build-windows-local`'s cargo-xwin/LLVM checks rather than
/// duplicating them, since both entry points depend on the same toolchain.
fn preflight_backend(backend: Backend) -> Result<(), String> {
    match backend {
        Backend::Cargo => Ok(()),
        Backend::CargoXwin => {
            crate::windows_local::ensure_cargo_xwin()?;
            crate::windows_local::ensure_llvm_tools()
        }
        Backend::CargoZigbuild => {
            ensure_cargo_zigbuild()?;
            ensure_zig()?;
            ensure_sdkroot_for_zigbuild(current_host_os())
        }
    }
}

/// Pinned `cargo-zigbuild` version, matching the `cargo-xwin` pin in
/// `windows_local.rs` so both cross toolchains stay reproducible.
const CARGO_ZIGBUILD_VERSION: &str = "0.23.0";

fn ensure_cargo_zigbuild() -> Result<(), String> {
    match installed_cargo_zigbuild_version()? {
        Some(found) if found == CARGO_ZIGBUILD_VERSION => Ok(()),
        Some(found) => Err(format!(
            "cargo-zigbuild {CARGO_ZIGBUILD_VERSION} is required, found {found}.\n  help: install the pinned version with: cargo install cargo-zigbuild --version {CARGO_ZIGBUILD_VERSION} --locked"
        )),
        None => Err(format!(
            "cargo-zigbuild {CARGO_ZIGBUILD_VERSION} is required but was not found on PATH.\n  help: install it with: cargo install cargo-zigbuild --version {CARGO_ZIGBUILD_VERSION} --locked"
        )),
    }
}

fn installed_cargo_zigbuild_version() -> Result<Option<String>, String> {
    let output = match std::process::Command::new("cargo-zigbuild")
        .arg("--version")
        .output()
    {
        Ok(output) => output,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(format!("failed to run cargo-zigbuild --version: {error}")),
    };

    if !output.status.success() {
        return Err(format!(
            "cargo-zigbuild --version failed with {}",
            output.status
        ));
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    Ok(cargo_zigbuild_version(&stdout).map(str::to_string))
}

fn cargo_zigbuild_version(output: &str) -> Option<&str> {
    let mut parts = output.split_whitespace();
    if parts.next() == Some("cargo-zigbuild") {
        return parts.next();
    }
    None
}

/// Pinned Zig version, matching the Azure release pipeline's toolchain pin
/// so `cargo zigbuild`'s cross-linking behavior stays reproducible between
/// a Linux developer machine and CI.
const ZIG_VERSION: &str = "0.13.0";

/// Environment variable naming an alternate `zig` executable, for machines
/// where the pinned Zig isn't the one on `PATH` (e.g. a version manager or
/// a toolchain cache directory). Mirrors `cargo-zigbuild`'s own lookup.
const CARGO_ZIGBUILD_ZIG_PATH_VAR: &str = "CARGO_ZIGBUILD_ZIG_PATH";

/// Resolve which `zig` executable to check/run: a non-empty
/// `CARGO_ZIGBUILD_ZIG_PATH` override, or the default `zig` on `PATH`.
///
/// Pure and independent of `std::env` so it can be unit-tested with mock
/// inputs; `zig_executable` below is the real entry point that reads the
/// actual environment variable.
fn resolve_zig_executable(env_value: Option<&str>) -> &str {
    match env_value {
        Some(value) if !value.trim().is_empty() => value,
        _ => "zig",
    }
}

fn zig_executable() -> String {
    let value = std::env::var(CARGO_ZIGBUILD_ZIG_PATH_VAR).ok();
    resolve_zig_executable(value.as_deref()).to_string()
}

/// Verify Zig is on the resolved executable path and matches [`ZIG_VERSION`].
///
/// `cargo zigbuild` shells out to `zig cc`/`zig c++` as its cross linker, so
/// a missing or mismatched Zig produces confusing link-time errors rather
/// than an actionable message; this fails fast with install guidance instead.
fn ensure_zig() -> Result<(), String> {
    let executable = zig_executable();
    match installed_zig_version(&executable)? {
        Some(found) if found == ZIG_VERSION => Ok(()),
        Some(found) => Err(format!(
            "Zig {ZIG_VERSION} is required for cargo zigbuild (matching the Azure release pipeline's pin), found {found} via {executable}.\n  help: install Zig {ZIG_VERSION} from https://ziglang.org/download/, or set {CARGO_ZIGBUILD_ZIG_PATH_VAR} to a Zig {ZIG_VERSION} binary"
        )),
        None => Err(format!(
            "Zig {ZIG_VERSION} is required for cargo zigbuild but {executable} was not found.\n  help: install Zig {ZIG_VERSION} from https://ziglang.org/download/, or set {CARGO_ZIGBUILD_ZIG_PATH_VAR} to a Zig {ZIG_VERSION} binary"
        )),
    }
}

fn installed_zig_version(executable: &str) -> Result<Option<String>, String> {
    let output = match std::process::Command::new(executable)
        .arg("version")
        .output()
    {
        Ok(output) => output,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(format!("failed to run {executable} version: {error}")),
    };

    if !output.status.success() {
        return Err(format!(
            "{executable} version failed with {}",
            output.status
        ));
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    Ok(zig_version(&stdout).map(str::to_string))
}

/// Parse `zig version` output, which — unlike `cargo-xwin`/`cargo-zigbuild`
/// `--version` — prints only the bare version number (e.g. `0.13.0`), with
/// no leading binary name to strip. Pure so it can be unit-tested directly.
fn zig_version(output: &str) -> Option<&str> {
    output.lines().next()?.split_whitespace().next()
}

/// Pure SDKROOT validation for the `CargoZigbuild` backend, so tests can
/// exercise every case (missing, empty, non-directory, valid) with mock
/// inputs instead of mutating the real process environment — `std::env`
/// mutation is unsafe and races across tests that run in parallel.
///
/// Unlike a native macOS toolchain, `cargo zigbuild` neither vendors nor
/// lazily downloads an Apple SDK: without a real `SDKROOT`, it silently
/// links against Zig's minimal libc/libSystem stubs instead of failing.
/// Public PRs validate with a direct `cargo check`/`cargo build`, so
/// `publish-build` must fail loudly here rather than let that happen.
/// xtask itself never downloads an SDK; trusted CI sets `SDKROOT` to a
/// vendored one explicitly. Native macOS hosts don't reach this backend
/// (see `select_backend_for_host`), so the check only gates the actual
/// cross case.
fn validate_sdkroot(
    host: HostOs,
    backend: Backend,
    sdkroot: Option<&str>,
    is_directory: impl Fn(&str) -> bool,
) -> Result<(), String> {
    if backend != Backend::CargoZigbuild || host == HostOs::MacOs {
        return Ok(());
    }

    match sdkroot {
        None => Err(
            "SDKROOT is required to cross-compile Apple targets with cargo zigbuild from a non-macOS host.\n  help: set SDKROOT to an extracted macOS SDK directory, e.g. SDKROOT=/opt/MacOSX14.sdk"
                .to_string(),
        ),
        Some(path) if path.trim().is_empty() => Err(
            "SDKROOT is set but empty; cargo zigbuild requires it to point at a real macOS SDK directory".to_string(),
        ),
        Some(path) if !is_directory(path) => Err(format!(
            "SDKROOT={path} does not name an existing directory.\n  help: point SDKROOT at an extracted macOS SDK directory"
        )),
        Some(_) => Ok(()),
    }
}

fn ensure_sdkroot_for_zigbuild(host: HostOs) -> Result<(), String> {
    let sdkroot = std::env::var("SDKROOT").ok();
    validate_sdkroot(host, Backend::CargoZigbuild, sdkroot.as_deref(), |path| {
        Path::new(path).is_dir()
    })
}

/// Like [`crate::util::run_command`], but also sets extra environment
/// variables (e.g. `MACOSX_DEPLOYMENT_TARGET` for the `cargo zigbuild`
/// backend; see `macos_deployment_target_env`). Every other backend passes
/// an empty slice and behaves exactly like `run_command`.
fn run_command_with_env(
    cmd: &str,
    args: &[&str],
    cwd: &Path,
    env: &[(&str, &str)],
) -> Result<(), String> {
    let mut command = build_command(cmd, args);
    command.current_dir(cwd);
    for (key, value) in env {
        command.env(key, value);
    }

    match command.status() {
        Ok(status) if status.success() => Ok(()),
        Ok(status) => Err(format!("exit code {}", status.code().unwrap_or(1))),
        Err(error) => Err(error.to_string()),
    }
}

/// Like [`crate::util::run_command_quiet`], but also sets extra environment
/// variables. Used for the `maturin --zig` backend; see
/// `run_command_with_env`.
fn run_command_quiet_with_env(
    cmd: &str,
    args: &[&str],
    cwd: &Path,
    env: &[(&str, &str)],
) -> Result<(), String> {
    use std::process::Stdio;

    let mut command = build_command(cmd, args);
    command.stdout(Stdio::piped()).stderr(Stdio::piped());
    command.current_dir(cwd);
    for (key, value) in env {
        command.env(key, value);
    }

    match command.output() {
        Ok(output) if output.status.success() => Ok(()),
        Ok(output) => {
            let mut msg = String::new();
            if let Ok(s) = String::from_utf8(output.stdout) {
                msg.push_str(&s);
            }
            if let Ok(s) = String::from_utf8(output.stderr) {
                msg.push_str(&s);
            }
            if msg.is_empty() {
                msg = format!("exit code {}", output.status.code().unwrap_or(1));
            }
            Err(msg)
        }
        Err(error) => Err(error.to_string()),
    }
}

/// Copy this target's freshly built artifacts into an external output root.
///
/// Each mode cleans and rewrites only the subtrees it owns, so a Linux leg can
/// export natives from the host and then export the wheel from the container
/// without the second run erasing the first.
fn export_target(
    root: &Path,
    output_root: &Path,
    triple: &str,
    mode: BuildMode,
) -> Result<(), String> {
    let safe_output_root = validate_export_output_root(root, output_root)?;
    let platform = PLATFORMS
        .iter()
        .find(|platform| platform.triple == triple)
        .ok_or_else(|| format!("unknown target triple: {triple}"))?;

    if mode.includes_native() {
        for (source, destination) in [
            (
                root.join("publish").join("native"),
                safe_output_root.join("publish").join("native"),
            ),
            (
                root.join("packages").join(platform.npm_package),
                safe_output_root.join("packages").join(platform.npm_package),
            ),
            (
                root.join("dotnet")
                    .join("runtimes")
                    .join(platform.nuget_rid),
                safe_output_root
                    .join("dotnet")
                    .join("runtimes")
                    .join(platform.nuget_rid),
            ),
        ] {
            clean_directory(&destination)?;
            copy_directory_contents(&source, &destination)?;
        }
    }

    if mode.includes_python() {
        let destination = safe_output_root.join("publish").join("python");
        clean_directory(&destination)?;
        copy_directory_contents(&root.join("publish").join("python"), &destination)?;
    }

    Ok(())
}

fn clean_directory(path: &Path) -> Result<(), String> {
    if path.exists() {
        fs::remove_dir_all(path)
            .map_err(|error| format!("failed to clean {}: {error}", path.display()))?;
    }
    Ok(())
}

fn validate_export_output_root(root: &Path, output_root: &Path) -> Result<PathBuf, String> {
    let output_is_absolute = output_root.is_absolute();
    let absolute_output = if output_is_absolute {
        output_root.to_path_buf()
    } else {
        root.join(output_root)
    };
    let normalized_output = normalize_path(&absolute_output);
    let normalized_root = normalize_path(root);

    if !output_is_absolute && !normalized_output.starts_with(&normalized_root) {
        return Err(format!(
            "relative export output must remain within the workspace: {}",
            output_root.display()
        ));
    }
    if normalized_output.parent().is_none()
        || normalized_output == normalized_root
        || normalized_root.starts_with(&normalized_output)
    {
        return Err(format!(
            "refusing to clean unsafe export output directory: {}",
            normalized_output.display()
        ));
    }
    let symlink_boundary = if output_is_absolute {
        normalized_root
            .ancestors()
            .find(|ancestor| normalized_output.starts_with(ancestor))
            .unwrap_or(normalized_root.as_path())
    } else {
        normalized_root.as_path()
    };
    if let Some(symlink) = find_symlinked_path_component(&normalized_output, symlink_boundary)? {
        return Err(format!(
            "refusing to clean export output through symlinked path component: {}",
            symlink.display()
        ));
    }
    if normalized_output.is_file() {
        return Err(format!(
            "export output path is a file: {}",
            normalized_output.display()
        ));
    }

    Ok(normalized_output)
}

fn find_symlinked_path_component(
    path: &Path,
    trusted_boundary: &Path,
) -> Result<Option<PathBuf>, String> {
    for component_path in path.ancestors() {
        if component_path == trusted_boundary {
            break;
        }
        match component_path.symlink_metadata() {
            Ok(metadata) if metadata.file_type().is_symlink() => {
                return Ok(Some(component_path.to_path_buf()));
            }
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => {
                return Err(format!(
                    "failed to inspect export output path {}: {error}",
                    component_path.display()
                ));
            }
        }
    }
    Ok(None)
}

fn normalize_path(path: &Path) -> PathBuf {
    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                normalized.pop();
            }
            Component::Prefix(prefix) => normalized.push(prefix.as_os_str()),
            Component::RootDir => normalized.push(component.as_os_str()),
            Component::Normal(value) => normalized.push(value),
        }
    }
    normalized
}

fn set_stage_mode(current: StageMode, requested: StageMode) -> Result<StageMode, String> {
    if current != StageMode::Full && current != requested {
        return Err("cannot combine --native-only and --pack-only".to_string());
    }

    Ok(requested)
}

// ── Phase 1: Native binary staging ──────────────────────────────────────

/// Stage all platforms whose build artifacts exist under target/.
fn stage_all_platforms(root: &Path, profile: &str) -> ExitCode {
    eprintln!(
        "{} Staging all available platforms ({})",
        console::style("▸").cyan().bold(),
        console::style(profile).dim(),
    );

    let host = detect_host_triple();
    let mut staged = 0u32;
    let mut skipped = 0u32;
    let mut failed = 0u32;

    for platform in PLATFORMS {
        let build_dir = if platform.triple == host {
            resolve_build_dir(root, platform.triple, profile)
        } else {
            root.join("target").join(platform.triple).join(profile)
        };

        let has_ffi = build_dir.join(platform.ffi_lib).exists();
        let has_cli = build_dir.join(platform.cli_binary).exists();
        let has_addon = build_dir.join(platform.node_addon).exists();

        if !has_ffi && !has_cli && !has_addon {
            skipped += 1;
            continue;
        }

        eprintln!(
            "\n  {} {}",
            console::style("▸").cyan(),
            console::style(platform.triple).bold(),
        );

        if stage_platform(root, platform, &build_dir) {
            staged += 1;
        } else {
            failed += 1;
        }
    }

    eprintln!();
    if staged > 0 {
        eprintln!(
            "  {} Staged {} platform(s)",
            console::style("✔").green(),
            console::style(staged).bold(),
        );
    }
    if skipped > 0 {
        eprintln!(
            "  {} Skipped {} platform(s) (no build artifacts found)",
            console::style("·").dim(),
            skipped,
        );
    }
    if failed > 0 {
        eprintln!(
            "  {} Failed {} platform(s)",
            console::style("✘").red().bold(),
            failed,
        );
        return ExitCode::FAILURE;
    }
    if staged == 0 {
        eprintln!(
            "  {} No build artifacts found. Build first:\n    {}",
            console::style("⚠").yellow(),
            console::style("cargo build --release -p microsoft-webui-ffi -p microsoft-webui-node -p microsoft-webui-cli").dim(),
        );
        return ExitCode::FAILURE;
    }

    ExitCode::SUCCESS
}

/// Stage a single platform by triple name.
fn stage_one_platform(root: &Path, triple: &str, profile: &str) -> ExitCode {
    let Some(platform) = PLATFORMS.iter().find(|p| p.triple == triple) else {
        eprintln!(
            "  {} Unknown target triple: {}",
            console::style("✘").red().bold(),
            triple,
        );
        eprintln!("  Supported targets (or use 'all'):");
        for p in PLATFORMS {
            eprintln!(
                "    {} → npm: {}, nuget: {}",
                p.triple, p.npm_package, p.nuget_rid
            );
        }
        return ExitCode::FAILURE;
    };

    let build_dir = resolve_build_dir(root, triple, profile);

    eprintln!(
        "{} Staging native binaries for {} ({})",
        console::style("▸").cyan().bold(),
        console::style(triple).bold(),
        console::style(profile).dim(),
    );

    if stage_platform(root, platform, &build_dir) {
        eprintln!(
            "\n  {} All binaries staged for {}",
            console::style("✔").green(),
            console::style(platform.triple).bold(),
        );
        ExitCode::SUCCESS
    } else {
        eprintln!(
            "\n  {} Some binaries could not be staged (see errors above)",
            console::style("⚠").yellow(),
        );
        ExitCode::FAILURE
    }
}

/// Copy all artifacts for a single platform. Returns true if all found files staged.
fn stage_platform(root: &Path, platform: &PlatformEntry, build_dir: &Path) -> bool {
    let mut ok = true;

    // NuGet: FFI library → dotnet/runtimes/{rid}/native/
    ok &= stage_file(&CopySpec {
        src: &build_dir.join(platform.ffi_lib),
        dest_dir: &root
            .join("dotnet/runtimes")
            .join(platform.nuget_rid)
            .join("native"),
        dest_name: platform.ffi_lib,
        label: "nuget",
    });

    // npm: CLI binary → packages/webui-{platform}/bin/
    ok &= stage_file(&CopySpec {
        src: &build_dir.join(platform.cli_binary),
        dest_dir: &root.join("packages").join(platform.npm_package).join("bin"),
        dest_name: platform.cli_binary,
        label: "npm cli",
    });

    // npm: Node addon (renamed to webui.node)
    ok &= stage_file(&CopySpec {
        src: &build_dir.join(platform.node_addon),
        dest_dir: &root.join("packages").join(platform.npm_package),
        dest_name: "webui.node",
        label: "npm addon",
    });

    // publish/native/: CLI binary with platform suffix for direct download
    let native_name = native_binary_name(platform);
    ok &= stage_file(&CopySpec {
        src: &build_dir.join(platform.cli_binary),
        dest_dir: &root.join("publish").join("native"),
        dest_name: &native_name,
        label: "native",
    });

    ok
}

/// Build a platform-suffixed CLI binary name (e.g. `webui-darwin-arm64`, `webui-win32-x64.exe`).
fn native_binary_name(platform: &PlatformEntry) -> String {
    if platform.cli_binary.ends_with(".exe") {
        format!("webui-{}.exe", platform.platform_suffix)
    } else {
        format!("webui-{}", platform.platform_suffix)
    }
}

// ── Phase 2: npm packaging ──────────────────────────────────────────────

/// Run `pnpm pack` in each `packages/*` directory and move tarballs to `publish/npm/`.
fn pack_npm_tarballs(root: &Path) -> Result<(), String> {
    let packages_dir = root.join("packages");
    let npm_out = root.join("publish").join("npm");

    // Build packages that have build scripts first
    for pkg_name in &["webui", "webui-framework", "webui-router"] {
        let pkg_dir = packages_dir.join(pkg_name);
        if !pkg_dir.join("package.json").exists() {
            continue;
        }
        eprintln!(
            "  {} Building {}",
            console::style("·").dim(),
            console::style(pkg_name).bold(),
        );
        run_command_quiet(
            "pnpm",
            &["--filter", &format!("@microsoft/{pkg_name}"), "build"],
            None,
        )
        .map_err(|e| format!("pnpm build @microsoft/{pkg_name} failed: {e}"))?;
    }

    // Pack each package
    let entries =
        fs::read_dir(&packages_dir).map_err(|e| format!("failed to read packages/: {e}"))?;

    let mut count = 0u32;
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        if !path.join("package.json").exists() {
            continue;
        }

        let pkg_name = entry.file_name().to_string_lossy().to_string();

        // Skip private packages — they must not be published
        if is_private_package(&path) {
            eprintln!(
                "  {} [npm] @microsoft/{} (private, skipped)",
                console::style("·").dim(),
                console::style(&pkg_name).bold(),
            );
            continue;
        }

        // Run pnpm pack in the package directory
        let mut cmd = build_command(
            "pnpm",
            &["pack", "--pack-destination", &npm_out.to_string_lossy()],
        );
        cmd.current_dir(&path);
        cmd.stdout(std::process::Stdio::piped());
        cmd.stderr(std::process::Stdio::piped());

        let output = cmd
            .output()
            .map_err(|e| format!("pnpm pack failed for {pkg_name}: {e}"))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            let stdout = String::from_utf8_lossy(&output.stdout);
            return Err(format!(
                "pnpm pack failed for {pkg_name}:\n{stdout}{stderr}"
            ));
        }

        eprintln!(
            "  {} [npm] @microsoft/{}",
            console::style("✔").green(),
            console::style(&pkg_name).bold(),
        );
        count += 1;
    }

    eprintln!(
        "  {} Packed {} npm package(s)",
        console::style("✔").green(),
        console::style(count).bold(),
    );
    Ok(())
}

/// Returns `true` if the `package.json` in `pkg_dir` has `"private": true`.
fn is_private_package(pkg_dir: &Path) -> bool {
    let pkg_json_path = pkg_dir.join("package.json");
    let Ok(contents) = fs::read_to_string(&pkg_json_path) else {
        return false;
    };
    let Ok(json) = serde_json::from_str::<serde_json::Value>(&contents) else {
        return false;
    };
    json.get("private").and_then(serde_json::Value::as_bool) == Some(true)
}

// ── Phase 3: NuGet packaging ────────────────────────────────────────────

/// Run `dotnet pack` and write `.nupkg`/`.snupkg` files to `publish/nuget/`.
fn pack_nuget_packages(root: &Path) -> Result<(), String> {
    let dotnet_dir = root.join("dotnet");
    let nuget_out = root.join("publish").join("nuget");

    if !dotnet_dir.exists() {
        eprintln!(
            "  {} dotnet/ directory not found, skipping NuGet packaging",
            console::style("·").dim(),
        );
        return Ok(());
    }

    let solution = dotnet_dir.join("Microsoft.WebUI.sln");
    if !solution.exists() {
        return Err("dotnet/Microsoft.WebUI.sln not found".to_string());
    }

    let solution_arg = solution.to_string_lossy();
    let nuget_out_arg = nuget_out.to_string_lossy();

    // Pack all packable projects (Directory.Build.props controls versioning)
    run_command_quiet(
        "dotnet",
        &[
            "pack",
            solution_arg.as_ref(),
            "--configuration",
            "Release",
            "--output",
            nuget_out_arg.as_ref(),
        ],
        None,
    )
    .map_err(|e| format!("dotnet pack failed: {e}"))?;

    // Count produced packages
    let package_count = count_files_with_extension(&nuget_out, "nupkg");
    let symbol_count = count_files_with_extension(&nuget_out, "snupkg");
    eprintln!(
        "  {} Packed {} NuGet package(s) and {} symbol package(s)",
        console::style("✔").green(),
        console::style(package_count).bold(),
        console::style(symbol_count).bold(),
    );
    Ok(())
}

// ── Phase 4: Rust crate packaging ───────────────────────────────────────

/// Discover publishable crates by scanning `crates/*/Cargo.toml`.
///
/// A crate is publishable if it has a `[package]` section with a `name` field
/// and does not contain `publish = false`.
fn discover_publishable_crates(root: &Path) -> Result<Vec<String>, String> {
    let crates_dir = root.join("crates");
    if !crates_dir.exists() {
        return Err("crates/ directory not found".to_string());
    }

    let mut entries: Vec<_> = fs::read_dir(&crates_dir)
        .map_err(|e| format!("failed to read crates/: {e}"))?
        .flatten()
        .filter(|e| e.path().join("Cargo.toml").is_file())
        .collect();
    entries.sort_by_key(|e| e.file_name());

    let mut crates = Vec::new();
    for entry in entries {
        let toml_path = entry.path().join("Cargo.toml");
        let content = fs::read_to_string(&toml_path)
            .map_err(|e| format!("failed to read {}: {e}", toml_path.display()))?;

        // Skip crates with publish = false
        if content.lines().any(|l| {
            let t = l.trim();
            t == "publish = false"
        }) {
            continue;
        }

        // Extract name from [package] section
        let mut in_package = false;
        for line in content.lines() {
            let trimmed = line.trim();
            if trimmed == "[package]" {
                in_package = true;
            } else if trimmed.starts_with('[') {
                in_package = false;
            }
            if in_package && trimmed.starts_with("name") && trimmed.contains('=') {
                if let Some(start) = trimmed.find('"') {
                    if let Some(end) = trimmed[start + 1..].find('"') {
                        crates.push(trimmed[start + 1..start + 1 + end].to_string());
                        break;
                    }
                }
            }
        }
    }

    if crates.is_empty() {
        return Err("no publishable crates found in crates/".to_string());
    }

    Ok(crates)
}

/// Package all workspace crates together and copy `.crate` files to `publish/crates/`.
///
/// Discovers publishable crates dynamically and uses a single `cargo package`
/// invocation so that inter-crate path dependencies resolve against each other
/// without requiring crates.io.
fn pack_rust_crates(root: &Path) -> Result<(), String> {
    let crates_out = root.join("publish").join("crates");

    let publishable = discover_publishable_crates(root)?;

    // Build args: cargo package -p A -p B -p C ... --no-verify --allow-dirty
    let mut args: Vec<&str> = vec!["package"];
    for crate_name in &publishable {
        args.push("-p");
        args.push(crate_name);
    }
    args.push("--no-verify");
    args.push("--allow-dirty");

    run_command_quiet("cargo", &args, None).map_err(|e| format!("cargo package failed: {e}"))?;

    for crate_name in &publishable {
        eprintln!(
            "  {} [crate] {}",
            console::style("✔").green(),
            console::style(crate_name).bold(),
        );
    }

    // Copy .crate files from target/package/ to publish/crates/
    let package_dir = root.join("target").join("package");
    if package_dir.exists() {
        copy_files_with_extension(&package_dir, &crates_out, "crate")?;
    }

    let count = count_files_with_extension(&crates_out, "crate");
    eprintln!(
        "  {} Packed {} Rust crate(s)",
        console::style("✔").green(),
        console::style(count).bold(),
    );
    Ok(())
}

// ── Python distribution ─────────────────────────────────────────────────

/// Build the `microsoft-webui` sdist into `publish/python/`.
///
/// Platform wheels are *not* built here: they are produced out-of-band, one
/// per target triple, by dedicated CI matrix legs using `maturin build`
/// (each leg's own toolchain and, for `manylinux`, container), then staged
/// directly into `publish/python/` before `publish-stage` runs. Both full and
/// `--pack-only` staging preserve those wheels.
/// This function only builds the single source distribution, which requires
/// nothing beyond the local `crates/webui-python` manifest.
fn pack_python_package(root: &Path) -> Result<(), String> {
    let python_out = root.join("publish").join("python");
    fs::create_dir_all(&python_out)
        .map_err(|e| format!("failed to create {}: {e}", python_out.display()))?;

    let manifest_path = root.join("crates").join("webui-python").join("Cargo.toml");
    if !manifest_path.is_file() {
        return Err(format!(
            "Python package manifest not found at {} (crates/webui-python must exist before packaging)",
            manifest_path.display()
        ));
    }

    let manifest_arg = manifest_path.to_string_lossy().into_owned();
    let out_arg = python_out.to_string_lossy().into_owned();
    run_command_quiet(
        "maturin",
        &["sdist", "--manifest-path", &manifest_arg, "--out", &out_arg],
        None,
    )
    .map_err(|e| format!("maturin sdist failed: {e}"))?;

    let sdist_count = count_files_with_suffix(&python_out, ".tar.gz");
    let wheel_count = count_files_with_extension(&python_out, "whl");
    eprintln!(
        "  {} Packed {} Python sdist(s), found {} pre-staged wheel(s)",
        console::style("✔").green(),
        console::style(sdist_count).bold(),
        console::style(wheel_count).bold(),
    );
    Ok(())
}

/// Validate that `publish/python/` contains exactly the expected `microsoft-webui`
/// release artifacts: one `cp311-abi3` wheel per [`PLATFORMS`] entry (six total)
/// and exactly one sdist, all matching `version`.
fn validate_python_release_artifacts(publish: &Path, version: &str) -> Result<(), String> {
    let python_dir = publish.join("python");
    let entries = fs::read_dir(&python_dir)
        .map_err(|e| format!("failed to read {}: {e}", python_dir.display()))?;

    let wheel_prefix = format!("{PYTHON_DISTRIBUTION_NAME}-{version}-");
    let mut wheel_tags: Vec<String> = Vec::new();
    let mut sdist_names: Vec<String> = Vec::new();

    for entry in entries {
        let entry =
            entry.map_err(|e| format!("failed to read {} entry: {e}", python_dir.display()))?;
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
            continue;
        };

        if let Some(tag) = name
            .strip_prefix(wheel_prefix.as_str())
            .and_then(|rest| rest.strip_suffix(".whl"))
        {
            wheel_tags.push(tag.to_string());
        } else if name.ends_with(".whl") {
            return Err(format!(
                "unexpected Python wheel filename: {name}; expected \
                 {wheel_prefix}{PYTHON_INTERPRETER_TAG}-{PYTHON_ABI_TAG}-<platform>.whl. \
                 Remove stale or differently-versioned wheels from {}",
                python_dir.display()
            ));
        } else if name.ends_with(".tar.gz") {
            sdist_names.push(name.to_string());
        }
    }

    validate_python_wheel_tags(&wheel_tags)?;

    let expected_sdist = format!("{PYTHON_DISTRIBUTION_NAME}-{version}.tar.gz");
    if sdist_names.len() != 1 {
        return Err(format!(
            "expected 1 Python sdist named {expected_sdist}, found {}; \
             publish-stage builds this sdist, so remove stale .tar.gz files from {} and retry",
            sdist_names.len(),
            python_dir.display()
        ));
    }
    if sdist_names[0] != expected_sdist {
        return Err(format!(
            "unexpected Python sdist filename: {}; expected {expected_sdist}. \
             Remove the stale sdist from {} and retry",
            sdist_names[0],
            python_dir.display()
        ));
    }

    Ok(())
}

fn validate_python_wheel_tags(wheel_tags: &[String]) -> Result<(), String> {
    let mut seen_platforms = vec![false; PLATFORMS.len()];

    for tag in wheel_tags {
        let Some((interpreter_tag, remainder)) = tag.split_once('-') else {
            return Err(malformed_python_wheel_tag_error(tag));
        };
        let Some((abi_tag, platform_tag)) = remainder.split_once('-') else {
            return Err(malformed_python_wheel_tag_error(tag));
        };
        if interpreter_tag != PYTHON_INTERPRETER_TAG {
            return Err(format!(
                "unsupported Python wheel interpreter tag `{interpreter_tag}` in `{tag}`; \
                 expected `{PYTHON_INTERPRETER_TAG}` for the CPython 3.11+ stable-ABI release"
            ));
        }
        if abi_tag != PYTHON_ABI_TAG {
            return Err(format!(
                "unsupported Python wheel ABI tag `{abi_tag}` in `{tag}`; \
                 expected `{PYTHON_ABI_TAG}` for the CPython 3.11+ stable-ABI release"
            ));
        }
        let Some(platform_index) = PLATFORMS
            .iter()
            .position(|platform| platform.python_platform_tag == platform_tag)
        else {
            return Err(format!(
                "unsupported Python wheel platform tag `{platform_tag}` in `{tag}`; \
                 expected one of: {}",
                expected_python_platform_tags()
            ));
        };
        if seen_platforms[platform_index] {
            return Err(format!(
                "duplicate Python wheel for platform `{platform_tag}`; \
                 keep exactly one {PYTHON_INTERPRETER_TAG}-{PYTHON_ABI_TAG} wheel per platform"
            ));
        }
        seen_platforms[platform_index] = true;
    }

    if wheel_tags.len() != PLATFORMS.len() {
        let missing_tags = PLATFORMS
            .iter()
            .zip(&seen_platforms)
            .filter(|(_, seen)| !**seen)
            .map(|(platform, _)| expected_python_wheel_tag(platform))
            .collect::<Vec<_>>()
            .join(", ");
        return Err(format!(
            "expected {} Python wheels, found {}; pre-stage one wheel per supported target in \
             publish/python/ before running publish-stage (default and --pack-only preserve \
             pre-staged wheels). Missing platform/ABI tags: {missing_tags}",
            PLATFORMS.len(),
            wheel_tags.len()
        ));
    }

    Ok(())
}

fn malformed_python_wheel_tag_error(tag: &str) -> String {
    format!(
        "malformed Python wheel tag `{tag}`; expected \
         `{PYTHON_INTERPRETER_TAG}-{PYTHON_ABI_TAG}-<platform>`"
    )
}

fn expected_python_wheel_tag(platform: &PlatformEntry) -> String {
    format!(
        "{PYTHON_INTERPRETER_TAG}-{PYTHON_ABI_TAG}-{}",
        platform.python_platform_tag
    )
}

fn expected_python_platform_tags() -> String {
    PLATFORMS
        .iter()
        .map(|platform| platform.python_platform_tag)
        .collect::<Vec<_>>()
        .join(", ")
}

/// Count files in a directory whose name ends with a given suffix.
///
/// Unlike [`count_files_with_extension`], this matches on the full filename
/// suffix rather than [`Path::extension`], which only returns the final
/// dot-segment (`"gz"` for `foo.tar.gz`, not `"tar.gz"`).
fn count_files_with_suffix(dir: &Path, suffix: &str) -> u32 {
    let mut count = 0;
    if let Ok(entries) = fs::read_dir(dir) {
        for entry in entries.flatten() {
            if entry
                .path()
                .file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|name| name.ends_with(suffix))
            {
                count += 1;
            }
        }
    }
    count
}

// ── Phase 5: WASM artifacts ─────────────────────────────────────────────

/// Build WASM variants and stage them for npm packaging and direct inspection.
fn stage_wasm_artifacts(root: &Path) -> Result<(), String> {
    crate::build_wasm::run()?;

    let wasm_source = root.join(crate::build_wasm::WASM_OUTPUT_DIR);
    let copied = stage_built_wasm_artifacts(root, &wasm_source)?;
    let standalone = stage_standalone_release_assets(root)?;
    eprintln!(
        "  {} [wasm] staged {} file(s) for npm and publish output; {} standalone asset(s)",
        console::style("✔").green(),
        console::style(copied).bold(),
        console::style(standalone).bold(),
    );

    Ok(())
}

fn stage_built_wasm_artifacts(root: &Path, wasm_source: &Path) -> Result<u32, String> {
    let package_out = root.join("packages").join("webui").join("wasm");
    fs::create_dir_all(&package_out)
        .map_err(|e| format!("failed to create {}: {e}", package_out.display()))?;
    for variant in WASM_VARIANT_DIRS {
        let generated_dir = package_out.join(variant);
        if generated_dir.exists() {
            fs::remove_dir_all(&generated_dir)
                .map_err(|e| format!("failed to clean {}: {e}", generated_dir.display()))?;
        }
    }

    let copied = copy_directory_contents(wasm_source, &package_out)?;
    let publish_out = root.join("publish").join("wasm");
    copy_directory_contents(wasm_source, &publish_out)?;
    Ok(copied)
}

fn stage_standalone_release_assets(root: &Path) -> Result<u32, String> {
    let publish = root.join("publish");
    let output = publish.join("standalone");
    if output.exists() {
        fs::remove_dir_all(&output)
            .map_err(|e| format!("failed to clean {}: {e}", output.display()))?;
    }
    fs::create_dir_all(&output)
        .map_err(|e| format!("failed to create {}: {e}", output.display()))?;

    for (source, destination) in STANDALONE_RELEASE_FILES {
        let source = publish.join(source);
        let destination = output.join(destination);
        fs::copy(&source, &destination).map_err(|e| {
            format!(
                "failed to copy standalone asset {} to {}: {e}",
                source.display(),
                destination.display()
            )
        })?;
    }

    u32::try_from(STANDALONE_RELEASE_FILES.len())
        .map_err(|_| "standalone release asset count exceeds u32".to_string())
}

// ── Shared helpers ──────────────────────────────────────────────────────

struct CopySpec<'a> {
    src: &'a Path,
    dest_dir: &'a Path,
    dest_name: &'a str,
    label: &'a str,
}

fn stage_file(spec: &CopySpec<'_>) -> bool {
    if !spec.src.exists() {
        eprintln!(
            "  {} [{}] not found: {}",
            console::style("⚠").yellow(),
            spec.label,
            console::style(spec.src.display()).dim(),
        );
        return false;
    }

    if let Err(e) = fs::create_dir_all(spec.dest_dir) {
        eprintln!(
            "  {} [{}] failed to create {}: {}",
            console::style("✘").red().bold(),
            spec.label,
            spec.dest_dir.display(),
            e,
        );
        return false;
    }

    let dest = spec.dest_dir.join(spec.dest_name);
    if let Err(e) = fs::copy(spec.src, &dest) {
        eprintln!(
            "  {} [{}] copy failed: {} → {}: {}",
            console::style("✘").red().bold(),
            spec.label,
            spec.src.display(),
            dest.display(),
            e,
        );
        return false;
    }

    let rel = dest
        .strip_prefix(std::env::current_dir().as_deref().unwrap_or(Path::new("")))
        .unwrap_or(&dest);
    eprintln!(
        "  {} [{}] {}",
        console::style("✔").green(),
        spec.label,
        console::style(rel.display()).bold(),
    );
    true
}

fn resolve_build_dir(root: &Path, triple: &str, profile: &str) -> PathBuf {
    let cross = root.join("target").join(triple).join(profile);
    if cross.exists() {
        return cross;
    }
    root.join("target").join(profile)
}

fn detect_host_triple() -> String {
    let arch = if cfg!(target_arch = "x86_64") {
        "x86_64"
    } else if cfg!(target_arch = "aarch64") {
        "aarch64"
    } else {
        "unknown"
    };

    let os = if cfg!(target_os = "linux") {
        "unknown-linux-gnu"
    } else if cfg!(target_os = "macos") {
        "apple-darwin"
    } else if cfg!(target_os = "windows") {
        "pc-windows-msvc"
    } else {
        "unknown"
    };

    format!("{arch}-{os}")
}

/// Count files in a directory matching a given extension.
fn count_files_with_extension(dir: &Path, ext: &str) -> u32 {
    let mut count = 0;
    if let Ok(entries) = fs::read_dir(dir) {
        for entry in entries.flatten() {
            if entry.path().extension().is_some_and(|e| e == ext) {
                count += 1;
            }
        }
    }
    count
}

fn validate_release_artifact_counts(root: &Path, version: &str) -> Result<(), String> {
    let publish = root.join("publish");
    validate_artifact_count(
        count_files_with_extension(&publish.join("npm"), "tgz"),
        9,
        "npm packages",
    )?;
    validate_artifact_count(
        count_files_with_extension(&publish.join("crates"), "crate"),
        15,
        "crate packages",
    )?;
    validate_artifact_count(
        count_files_with_extension(&publish.join("nuget"), "nupkg"),
        8,
        "NuGet packages",
    )?;
    validate_artifact_count(
        count_files_with_extension(&publish.join("nuget"), "snupkg"),
        2,
        "NuGet symbol packages",
    )?;
    validate_artifact_count(
        count_regular_files(&publish.join("standalone")),
        20,
        "standalone release assets",
    )?;
    validate_python_release_artifacts(&publish, version)
}

fn validate_artifact_count(actual: u32, expected: u32, kind: &str) -> Result<(), String> {
    if actual == expected {
        Ok(())
    } else {
        Err(format!("expected {expected} {kind}, found {actual}"))
    }
}

fn count_regular_files(dir: &Path) -> u32 {
    let Ok(entries) = fs::read_dir(dir) else {
        return 0;
    };
    let count = entries
        .flatten()
        .filter(|entry| entry.path().is_file())
        .count();
    u32::try_from(count).unwrap_or(u32::MAX)
}

/// Copy all files with a given extension from `src_dir` to `dest_dir`.
fn copy_files_with_extension(src_dir: &Path, dest_dir: &Path, ext: &str) -> Result<(), String> {
    let entries =
        fs::read_dir(src_dir).map_err(|e| format!("failed to read {}: {e}", src_dir.display()))?;

    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().is_some_and(|e| e == ext) {
            if let Some(name) = path.file_name() {
                let dest = dest_dir.join(name);
                fs::copy(&path, &dest).map_err(|e| {
                    format!(
                        "failed to copy {} → {}: {e}",
                        path.display(),
                        dest.display()
                    )
                })?;
            }
        }
    }
    Ok(())
}

/// Copy all files under `src_dir` into `dest_dir`, preserving subdirectories.
fn copy_directory_contents(src_dir: &Path, dest_dir: &Path) -> Result<u32, String> {
    if !src_dir.exists() {
        return Err(format!(
            "source directory not found at {}",
            src_dir.display()
        ));
    }

    let mut copied = 0;
    let mut stack = Vec::with_capacity(4);
    stack.push((src_dir.to_path_buf(), dest_dir.to_path_buf()));

    while let Some((current_src, current_dest)) = stack.pop() {
        fs::create_dir_all(&current_dest)
            .map_err(|e| format!("failed to create {}: {e}", current_dest.display()))?;
        let entries = fs::read_dir(&current_src)
            .map_err(|e| format!("failed to read {}: {e}", current_src.display()))?;

        for entry in entries {
            let entry = entry
                .map_err(|e| format!("failed to read {} entry: {e}", current_src.display()))?;
            let path = entry.path();
            let dest = current_dest.join(entry.file_name());
            if path.is_dir() {
                stack.push((path, dest));
            } else if path.is_file() {
                fs::copy(&path, &dest).map_err(|e| {
                    format!(
                        "failed to copy {} → {}: {e}",
                        path.display(),
                        dest.display()
                    )
                })?;
                copied += 1;
            }
        }
    }

    Ok(copied)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse_build(args: &[&str]) -> Result<BuildOptions, String> {
        let args = args.iter().map(|arg| arg.to_string()).collect::<Vec<_>>();
        parse_build_options(&args)
    }

    fn parse(args: &[&str]) -> Result<StageOptions, String> {
        let args = args.iter().map(|arg| arg.to_string()).collect::<Vec<_>>();
        parse_stage_options(&args)
    }

    #[test]
    fn parse_stage_options_defaults_to_full_mode() {
        let options = parse(&[]).expect("default options should parse");

        assert_eq!(options.target_triple, None);
        assert_eq!(options.profile, "release");
        assert_eq!(options.mode, StageMode::Full);
    }

    #[test]
    fn parse_stage_options_supports_pack_only_mode() {
        let options = parse(&["--target", "all", "--profile", "debug", "--pack-only"])
            .expect("pack-only options should parse");

        assert_eq!(options.target_triple.as_deref(), Some("all"));
        assert_eq!(options.profile, "debug");
        assert_eq!(options.mode, StageMode::PackOnly);
    }

    #[test]
    fn parse_stage_options_rejects_conflicting_modes() {
        let error =
            parse(&["--native-only", "--pack-only"]).expect_err("conflicting modes should fail");

        assert!(error.contains("cannot combine"));
    }

    #[test]
    fn parse_build_options_supports_one_target() {
        let options = parse_build(&[
            "--target",
            "aarch64-unknown-linux-gnu",
            "--profile",
            "debug",
        ])
        .expect("publish build options should parse");

        assert_eq!(options.target_triple, "aarch64-unknown-linux-gnu");
        assert_eq!(options.profile, "debug");
        assert_eq!(options.output_root, None);
        // A plain run covers a whole release leg: natives plus the wheel.
        assert_eq!(options.mode, BuildMode::Full);
    }

    #[test]
    fn parse_build_options_supports_split_environment_modes() {
        let native = parse_build(&["--target", "x86_64-unknown-linux-gnu", "--native-only"])
            .expect("native-only should parse");
        assert_eq!(native.mode, BuildMode::NativeOnly);
        assert!(native.mode.includes_native());
        assert!(!native.mode.includes_python());

        let python = parse_build(&["--target", "x86_64-unknown-linux-gnu", "--python-only"])
            .expect("python-only should parse");
        assert_eq!(python.mode, BuildMode::PythonOnly);
        assert!(!python.mode.includes_native());
        assert!(python.mode.includes_python());
    }

    #[test]
    fn maturin_build_args_never_name_an_interpreter() {
        for platform in PLATFORMS {
            let args = maturin_build_args("Cargo.toml", platform.triple, "out", Backend::Cargo);

            assert!(
                !args.contains(&"--interpreter"),
                "abi3 cross builds must not pin an interpreter for {}",
                platform.triple
            );
            assert!(args.contains(&"--locked"));
            assert!(args.contains(&platform.triple));

            let manylinux = args.contains(&"manylinux_2_17");
            assert_eq!(
                manylinux,
                platform.triple.ends_with("-linux-gnu"),
                "only Linux wheels declare a manylinux baseline ({})",
                platform.triple
            );
        }
    }

    #[test]
    fn parse_build_options_rejects_conflicting_modes() {
        let error = parse_build(&[
            "--target",
            "x86_64-unknown-linux-gnu",
            "--native-only",
            "--python-only",
        ])
        .expect_err("conflicting modes should fail");

        assert!(error.contains("cannot combine --native-only and --python-only"));
    }

    #[test]
    fn parse_build_options_rejects_missing_target() {
        let error = parse_build(&[]).expect_err("a target should be required");

        assert!(error.contains("requires one --target"));
    }

    #[test]
    fn parse_build_options_rejects_multiple_targets() {
        let error = parse_build(&[
            "--target",
            "x86_64-unknown-linux-gnu",
            "--target",
            "aarch64-unknown-linux-gnu",
        ])
        .expect_err("multiple targets should require separate jobs");

        assert!(error.contains("exactly one --target"));
    }

    #[test]
    fn parse_build_options_supports_output_root() {
        let options = parse_build(&[
            "--target",
            "x86_64-unknown-linux-gnu",
            "--output",
            "artifacts/stage-linux",
        ])
        .expect("output options should parse");

        assert_eq!(
            options.output_root,
            Some(PathBuf::from("artifacts/stage-linux"))
        );
    }

    #[test]
    fn parse_build_options_rejects_unknown_target() {
        let error =
            parse_build(&["--target", "unknown-target"]).expect_err("unknown targets should fail");

        assert!(error.contains("unknown target triple"));
    }

    #[test]
    fn parse_build_options_rejects_all_target() {
        let error =
            parse_build(&["--target", "all"]).expect_err("all should require explicit targets");

        assert!(error.contains("--target all is not supported"));
    }

    #[test]
    fn parse_build_options_rejects_unsupported_profile() {
        let error = parse_build(&["--target", "x86_64-unknown-linux-gnu", "--profile", "dev"])
            .expect_err("unsupported profiles should fail");

        assert!(error.contains("expected release or debug"));
    }

    #[test]
    fn native_build_args_map_debug_to_cargo_dev_profile() {
        let args = native_build_args("x86_64-unknown-linux-gnu", "debug", Backend::Cargo)
            .expect("debug profile should be supported");

        assert_eq!(args[0], "build");
        assert!(!args.contains(&"--profile"));
        assert!(!args.contains(&"--release"));
    }

    #[test]
    fn native_build_args_map_release_to_release_flag() {
        let args = native_build_args("x86_64-unknown-linux-gnu", "release", Backend::Cargo)
            .expect("release profile should be supported");

        assert!(args.contains(&"--release"));
    }

    #[test]
    fn native_build_args_prefix_backend_subcommand() {
        let cargo = native_build_args("x86_64-unknown-linux-gnu", "release", Backend::Cargo)
            .expect("cargo backend should be supported");
        assert_eq!(cargo[0], "build");

        let xwin = native_build_args("x86_64-pc-windows-msvc", "release", Backend::CargoXwin)
            .expect("xwin backend should be supported");
        assert_eq!(&xwin[..2], &["xwin", "build"]);

        let zigbuild = native_build_args("aarch64-apple-darwin", "release", Backend::CargoZigbuild)
            .expect("zigbuild backend should be supported");
        assert_eq!(&zigbuild[..2], &["zigbuild", "build"]);

        // Every backend still builds the same three native packages.
        for args in [&cargo, &xwin, &zigbuild] {
            assert!(args.contains(&"microsoft-webui-cli"));
            assert!(args.contains(&"microsoft-webui-ffi"));
            assert!(args.contains(&"microsoft-webui-node"));
        }
    }

    #[test]
    fn select_backend_for_host_uses_native_cargo_for_each_host_family() {
        for platform in PLATFORMS {
            let backend = select_backend_for_host(HostOs::Linux, platform.triple);
            let expected = if platform.triple.ends_with("-linux-gnu") {
                Backend::Cargo
            } else if platform.triple.ends_with("-pc-windows-msvc") {
                Backend::CargoXwin
            } else {
                Backend::CargoZigbuild
            };
            assert_eq!(
                backend, expected,
                "Linux host backend mismatch for {}",
                platform.triple
            );
        }
    }

    #[test]
    fn select_backend_for_host_linux_accepts_every_platform_entry() {
        // A Linux host must have a defined (non-panicking) backend for all
        // six release targets: this is the primary cross-compilation host.
        for platform in PLATFORMS {
            let backend = select_backend_for_host(HostOs::Linux, platform.triple);
            assert_ne!(
                format!("{backend:?}"),
                "",
                "Linux host should accept {}",
                platform.triple
            );
        }
    }

    #[test]
    fn select_backend_for_host_macos_builds_its_own_darwin_targets_natively() {
        assert_eq!(
            select_backend_for_host(HostOs::MacOs, "x86_64-apple-darwin"),
            Backend::Cargo
        );
        assert_eq!(
            select_backend_for_host(HostOs::MacOs, "aarch64-apple-darwin"),
            Backend::Cargo
        );
        // macOS still needs cargo-xwin to reach Windows MSVC.
        assert_eq!(
            select_backend_for_host(HostOs::MacOs, "x86_64-pc-windows-msvc"),
            Backend::CargoXwin
        );
    }

    #[test]
    fn select_backend_for_host_windows_builds_its_own_msvc_targets_natively() {
        assert_eq!(
            select_backend_for_host(HostOs::Windows, "x86_64-pc-windows-msvc"),
            Backend::Cargo
        );
        assert_eq!(
            select_backend_for_host(HostOs::Windows, "aarch64-pc-windows-msvc"),
            Backend::Cargo
        );
    }

    #[test]
    fn backend_subcommand_matches_expected_cargo_plugin() {
        assert_eq!(Backend::Cargo.subcommand(), None);
        assert_eq!(Backend::CargoXwin.subcommand(), Some("xwin"));
        assert_eq!(Backend::CargoZigbuild.subcommand(), Some("zigbuild"));
    }

    #[test]
    fn maturin_build_args_add_backend_flag_only_for_cross_toolchains() {
        let cargo = maturin_build_args(
            "Cargo.toml",
            "x86_64-unknown-linux-gnu",
            "out",
            Backend::Cargo,
        );
        assert!(!cargo.contains(&"--xwin"));
        assert!(!cargo.contains(&"--zig"));

        let xwin = maturin_build_args(
            "Cargo.toml",
            "x86_64-pc-windows-msvc",
            "out",
            Backend::CargoXwin,
        );
        assert!(xwin.contains(&"--xwin"));
        assert!(!xwin.contains(&"--zig"));

        let zigbuild = maturin_build_args(
            "Cargo.toml",
            "aarch64-apple-darwin",
            "out",
            Backend::CargoZigbuild,
        );
        assert!(zigbuild.contains(&"--zig"));
        assert!(!zigbuild.contains(&"--xwin"));
    }

    #[test]
    fn maturin_build_args_preserve_abi3_target_output_and_manylinux() {
        for platform in PLATFORMS {
            let backend = select_backend_for_host(HostOs::Linux, platform.triple);
            let args = maturin_build_args("Cargo.toml", platform.triple, "out", backend);

            // abi3: no --interpreter is ever pinned (see maturin_build_args docs).
            assert!(!args.contains(&"--interpreter"));
            assert!(args.contains(&"--target"));
            assert!(args.contains(&platform.triple));
            assert!(args.contains(&"--out"));
            assert!(args.contains(&"out"));
            assert_eq!(
                args.contains(&"manylinux_2_17"),
                platform.triple.ends_with("-linux-gnu")
            );
        }
    }

    #[test]
    fn macos_deployment_target_env_matches_platform_metadata() {
        assert_eq!(
            macos_deployment_target_env("x86_64-apple-darwin"),
            Some(("MACOSX_DEPLOYMENT_TARGET", "10.12"))
        );
        assert_eq!(
            macos_deployment_target_env("aarch64-apple-darwin"),
            Some(("MACOSX_DEPLOYMENT_TARGET", "11.0"))
        );
        assert_eq!(
            macos_deployment_target_env("x86_64-unknown-linux-gnu"),
            None
        );
        assert_eq!(macos_deployment_target_env("unknown-target"), None);
    }

    #[test]
    fn cargo_zigbuild_version_parses_expected_output() {
        assert_eq!(
            cargo_zigbuild_version("cargo-zigbuild 0.23.0\n"),
            Some(CARGO_ZIGBUILD_VERSION)
        );
        assert_eq!(cargo_zigbuild_version("cargo 1.93.0\n"), None);
    }

    #[test]
    fn ensure_cargo_zigbuild_error_mentions_pinned_version_when_missing() {
        // `cargo-zigbuild` is not expected to be on PATH in the test
        // environment, so this exercises the "not found" branch's message.
        if crate::util::which_exists("cargo-zigbuild") {
            return;
        }

        let error = ensure_cargo_zigbuild().expect_err("cargo-zigbuild should be missing in CI");
        assert!(error.contains(CARGO_ZIGBUILD_VERSION));
        assert!(error.contains("cargo install cargo-zigbuild"));
    }

    #[test]
    fn resolve_zig_executable_prefers_non_empty_override() {
        assert_eq!(
            resolve_zig_executable(Some("/opt/zig-0.13.0/zig")),
            "/opt/zig-0.13.0/zig"
        );
    }

    #[test]
    fn resolve_zig_executable_falls_back_to_default_when_unset_or_blank() {
        assert_eq!(resolve_zig_executable(None), "zig");
        assert_eq!(resolve_zig_executable(Some("")), "zig");
        assert_eq!(resolve_zig_executable(Some("   ")), "zig");
    }

    #[test]
    fn zig_version_parses_bare_version_number() {
        assert_eq!(zig_version("0.13.0\n"), Some(ZIG_VERSION));
        assert_eq!(zig_version("0.13.0"), Some("0.13.0"));
        assert_eq!(zig_version(""), None);
        assert_eq!(zig_version("\n"), None);
    }

    #[test]
    fn ensure_zig_error_mentions_pinned_version_and_override_var_when_missing() {
        // Zig is not expected to be on PATH in the test environment, so
        // this exercises the "not found" branch's message.
        if crate::util::which_exists("zig") {
            return;
        }

        let error = ensure_zig().expect_err("zig should be missing in CI");
        assert!(error.contains(ZIG_VERSION));
        assert!(error.contains(CARGO_ZIGBUILD_ZIG_PATH_VAR));
    }

    #[test]
    fn validate_sdkroot_requires_sdkroot_for_zigbuild_on_non_macos_hosts() {
        let error = validate_sdkroot(HostOs::Linux, Backend::CargoZigbuild, None, |_| true)
            .expect_err("missing SDKROOT should fail");
        assert!(error.contains("SDKROOT"));
        assert!(error.contains("cargo zigbuild"));
    }

    #[test]
    fn validate_sdkroot_rejects_empty_value() {
        let error = validate_sdkroot(HostOs::Linux, Backend::CargoZigbuild, Some("   "), |_| true)
            .expect_err("empty SDKROOT should fail");
        assert!(error.contains("SDKROOT"));
    }

    #[test]
    fn validate_sdkroot_rejects_nonexistent_directory() {
        let error = validate_sdkroot(
            HostOs::Linux,
            Backend::CargoZigbuild,
            Some("/does/not/exist.sdk"),
            |_| false,
        )
        .expect_err("a SDKROOT that is not a real directory should fail");
        assert!(error.contains("/does/not/exist.sdk"));
        assert!(error.contains("existing directory"));
    }

    #[test]
    fn validate_sdkroot_accepts_a_real_directory() {
        validate_sdkroot(
            HostOs::Linux,
            Backend::CargoZigbuild,
            Some("/opt/MacOSX14.sdk"),
            |_| true,
        )
        .expect("an existing SDKROOT directory should pass");
    }

    #[test]
    fn validate_sdkroot_skips_check_on_macos_host_and_non_zigbuild_backends() {
        // Native macOS hosts never reach the CargoZigbuild backend (see
        // select_backend_for_host), so the check is a no-op there even
        // without SDKROOT.
        validate_sdkroot(HostOs::MacOs, Backend::CargoZigbuild, None, |_| false)
            .expect("macOS host should skip the SDKROOT check");

        // Only the CargoZigbuild backend needs an Apple SDK.
        validate_sdkroot(HostOs::Linux, Backend::Cargo, None, |_| false)
            .expect("Cargo backend should not require SDKROOT");
        validate_sdkroot(HostOs::Linux, Backend::CargoXwin, None, |_| false)
            .expect("CargoXwin backend should not require SDKROOT");
    }

    #[test]
    fn test_native_binary_name_unix() {
        let p = PlatformEntry {
            triple: "aarch64-apple-darwin",
            npm_package: "webui-darwin-arm64",
            nuget_rid: "osx-arm64",
            ffi_lib: "libwebui_ffi.dylib",
            node_addon: "libwebui_node.dylib",
            cli_binary: "webui",
            platform_suffix: "darwin-arm64",
            python_platform_tag: "macosx_11_0_arm64",
            macos_deployment_target: Some("11.0"),
        };
        assert_eq!(native_binary_name(&p), "webui-darwin-arm64");
    }

    #[test]
    fn test_native_binary_name_windows() {
        let p = PlatformEntry {
            triple: "x86_64-pc-windows-msvc",
            npm_package: "webui-win32-x64",
            nuget_rid: "win-x64",
            ffi_lib: "webui_ffi.dll",
            node_addon: "webui_node.dll",
            cli_binary: "webui.exe",
            platform_suffix: "win32-x64",
            python_platform_tag: "win_amd64",
            macos_deployment_target: None,
        };
        assert_eq!(native_binary_name(&p), "webui-win32-x64.exe");
    }

    #[test]
    fn test_create_publish_dirs() {
        let dir = tempfile::TempDir::new().unwrap();
        prepare_publish_dirs(dir.path(), StageMode::Full).unwrap();

        for subdir in PUBLISH_SUBDIRS {
            assert!(
                dir.path().join("publish").join(subdir).is_dir(),
                "publish/{subdir} should exist"
            );
        }
    }
    #[test]
    fn test_create_publish_dirs_cleans_existing() {
        let dir = tempfile::TempDir::new().unwrap();
        let publish = dir.path().join("publish");
        fs::create_dir_all(publish.join("stale")).unwrap();
        fs::write(publish.join("stale").join("old.txt"), "old").unwrap();

        prepare_publish_dirs(dir.path(), StageMode::Full).unwrap();

        assert!(!publish.join("stale").exists(), "stale/ should be removed");
        for subdir in PUBLISH_SUBDIRS {
            assert!(publish.join(subdir).is_dir());
        }
    }

    #[test]
    fn test_full_stage_preserves_python_wheels_while_cleaning_other_outputs() {
        let dir = tempfile::TempDir::new().expect("root should be created");
        let publish = dir.path().join("publish");
        let python = publish.join("python");
        fs::create_dir_all(&python).expect("publish/python should be created");
        fs::create_dir_all(publish.join("stale")).expect("stale output should be created");
        let wheel = python.join(concat!(
            "microsoft_webui-1.0.0-cp311-abi3-",
            "manylinux_2_17_x86_64.manylinux2014_x86_64.whl"
        ));
        fs::write(&wheel, "wheel").expect("wheel fixture should be written");
        fs::write(publish.join("stale").join("old.txt"), "old")
            .expect("stale fixture should be written");

        prepare_publish_dirs(dir.path(), StageMode::Full)
            .expect("full stage directories should be prepared");

        assert!(wheel.is_file(), "full mode must preserve pre-staged wheels");
        assert!(
            !publish.join("stale").exists(),
            "full mode must clean non-Python outputs"
        );
    }

    #[test]
    fn test_pack_only_preserves_native_outputs() {
        let dir = tempfile::TempDir::new().unwrap();
        let publish = dir.path().join("publish");
        let native = publish.join("native");
        let npm = publish.join("npm");

        fs::create_dir_all(&native).unwrap();
        fs::create_dir_all(&npm).unwrap();
        fs::write(native.join("webui-win32-x64.exe"), "bin").unwrap();
        fs::write(npm.join("stale.tgz"), "stale").unwrap();

        prepare_publish_dirs(dir.path(), StageMode::PackOnly).unwrap();

        assert!(native.join("webui-win32-x64.exe").exists());
        assert!(!npm.join("stale.tgz").exists());
        assert!(publish.join("nuget").is_dir());
        assert!(publish.join("crates").is_dir());
        assert!(publish.join("wasm").is_dir());
        assert!(publish.join("standalone").is_dir());
    }

    #[test]
    fn test_pack_only_preserves_python_wheels() {
        let dir = tempfile::TempDir::new().unwrap();
        let publish = dir.path().join("publish");
        let python = publish.join("python");

        fs::create_dir_all(&python).unwrap();
        fs::write(
            python.join("microsoft_webui-1.0.0-cp311-abi3-win_amd64.whl"),
            "wheel",
        )
        .unwrap();

        prepare_publish_dirs(dir.path(), StageMode::PackOnly).unwrap();

        assert!(python
            .join("microsoft_webui-1.0.0-cp311-abi3-win_amd64.whl")
            .exists());
    }

    #[test]
    fn test_native_only_cleans_python_wheels() {
        let dir = tempfile::TempDir::new().expect("root should be created");
        let publish = dir.path().join("publish");
        let python = publish.join("python");
        fs::create_dir_all(&python).expect("publish/python should be created");
        fs::write(
            python.join("microsoft_webui-1.0.0-cp311-abi3-win_amd64.whl"),
            "wheel",
        )
        .expect("wheel fixture should be written");

        prepare_publish_dirs(dir.path(), StageMode::NativeOnly)
            .expect("native-only directories should be prepared");

        assert_eq!(
            fs::read_dir(python)
                .expect("publish/python should exist")
                .count(),
            0,
            "native-only mode starts a fresh per-target artifact stage"
        );
    }

    #[test]
    fn test_count_files_with_extension() {
        let dir = tempfile::TempDir::new().unwrap();
        fs::write(dir.path().join("a.crate"), "").unwrap();
        fs::write(dir.path().join("b.crate"), "").unwrap();
        fs::write(dir.path().join("a.snupkg"), "").unwrap();
        fs::write(dir.path().join("c.txt"), "").unwrap();
        assert_eq!(count_files_with_extension(dir.path(), "crate"), 2);
        assert_eq!(count_files_with_extension(dir.path(), "snupkg"), 1);
        assert_eq!(count_files_with_extension(dir.path(), "txt"), 1);
        assert_eq!(count_files_with_extension(dir.path(), "nupkg"), 0);
    }

    #[test]
    fn test_count_files_with_suffix() {
        let dir = tempfile::TempDir::new().unwrap();
        fs::write(dir.path().join("microsoft_webui-1.0.0.tar.gz"), "").unwrap();
        fs::write(dir.path().join("other-1.0.0.tar.gz"), "").unwrap();
        fs::write(dir.path().join("microsoft_webui-1.0.0-cp311.whl"), "").unwrap();
        assert_eq!(count_files_with_suffix(dir.path(), ".tar.gz"), 2);
        assert_eq!(count_files_with_suffix(dir.path(), ".whl"), 1);
        assert_eq!(count_files_with_suffix(dir.path(), ".zip"), 0);
    }

    #[test]
    fn pack_python_package_errors_when_manifest_missing() {
        let root = tempfile::TempDir::new().expect("root should be created");

        let error = pack_python_package(root.path())
            .expect_err("packaging should fail without crates/webui-python");

        assert!(error.contains("crates/webui-python"));
    }

    #[test]
    fn validate_python_release_artifacts_accepts_maturin_compressed_manylinux_tags() {
        let root = tempfile::TempDir::new().expect("root should be created");
        let publish = root.path().join("publish");
        let python_dir = publish.join("python");
        fs::create_dir_all(&python_dir).expect("publish/python should be created");
        for tag in [
            "cp311-abi3-manylinux_2_17_x86_64.manylinux2014_x86_64",
            "cp311-abi3-manylinux_2_17_aarch64.manylinux2014_aarch64",
            "cp311-abi3-win_amd64",
            "cp311-abi3-win_arm64",
            "cp311-abi3-macosx_10_12_x86_64",
            "cp311-abi3-macosx_11_0_arm64",
        ] {
            fs::write(
                python_dir.join(format!("{PYTHON_DISTRIBUTION_NAME}-1.2.3-{tag}.whl")),
                "wheel",
            )
            .expect("wheel fixture should be written");
        }
        fs::write(
            python_dir.join(format!("{PYTHON_DISTRIBUTION_NAME}-1.2.3.tar.gz")),
            "sdist",
        )
        .expect("sdist fixture should be written");

        assert!(validate_python_release_artifacts(&publish, "1.2.3").is_ok());
    }

    #[test]
    fn validate_python_release_artifacts_accepts_expected_wheels_and_sdist() {
        let root = tempfile::TempDir::new().expect("root should be created");
        let publish = root.path().join("publish");
        write_python_release_fixtures(&publish.join("python"), "1.2.3");

        assert!(validate_python_release_artifacts(&publish, "1.2.3").is_ok());
    }

    #[test]
    fn validate_python_release_artifacts_rejects_wrong_wheel_count() {
        let root = tempfile::TempDir::new().expect("root should be created");
        let python_dir = root.path().join("publish").join("python");
        fs::create_dir_all(&python_dir).expect("publish/python should be created");
        // Only stage 5 of the 6 expected wheels, plus a matching sdist.
        for platform in &PLATFORMS[..5] {
            fs::write(
                python_dir.join(format!(
                    "{PYTHON_DISTRIBUTION_NAME}-1.2.3-{}.whl",
                    expected_python_wheel_tag(platform)
                )),
                "wheel",
            )
            .expect("wheel fixture should be written");
        }
        fs::write(
            python_dir.join(format!("{PYTHON_DISTRIBUTION_NAME}-1.2.3.tar.gz")),
            "sdist",
        )
        .expect("sdist fixture should be written");

        let error = validate_python_release_artifacts(&root.path().join("publish"), "1.2.3")
            .expect_err("missing wheel should fail validation");

        assert!(error.contains("expected 6 Python wheels, found 5"));
        assert!(error.contains("pre-stage one wheel per supported target"));
        assert!(error.contains("macosx_11_0_arm64"));
    }

    #[test]
    fn validate_python_release_artifacts_rejects_unknown_tag() {
        let root = tempfile::TempDir::new().expect("root should be created");
        let python_dir = root.path().join("publish").join("python");
        fs::create_dir_all(&python_dir).expect("publish/python should be created");
        for platform in &PLATFORMS[1..] {
            fs::write(
                python_dir.join(format!(
                    "{PYTHON_DISTRIBUTION_NAME}-1.2.3-{}.whl",
                    expected_python_wheel_tag(platform)
                )),
                "wheel",
            )
            .expect("wheel fixture should be written");
        }
        // Replace the first platform's wheel with one bearing an unexpected tag.
        fs::write(
            python_dir.join(format!(
                "{PYTHON_DISTRIBUTION_NAME}-1.2.3-cp311-abi3-linux_x86_64.whl"
            )),
            "wheel",
        )
        .expect("wheel fixture should be written");
        fs::write(
            python_dir.join(format!("{PYTHON_DISTRIBUTION_NAME}-1.2.3.tar.gz")),
            "sdist",
        )
        .expect("sdist fixture should be written");

        let error = validate_python_release_artifacts(&root.path().join("publish"), "1.2.3")
            .expect_err("unexpected tag should fail validation");

        assert!(error.contains("unsupported Python wheel platform tag"));
        assert!(error.contains("expected one of"));
    }

    #[test]
    fn validate_python_release_artifacts_rejects_wrong_abi() {
        let root = tempfile::TempDir::new().expect("root should be created");
        let publish = root.path().join("publish");
        let python_dir = publish.join("python");
        write_python_release_fixtures(&python_dir, "1.2.3");
        let windows_x64 = PLATFORMS
            .iter()
            .find(|platform| platform.python_platform_tag == "win_amd64")
            .expect("Windows x64 platform should exist");
        fs::remove_file(python_dir.join(format!(
            "{PYTHON_DISTRIBUTION_NAME}-1.2.3-{}.whl",
            expected_python_wheel_tag(windows_x64)
        )))
        .expect("expected wheel should be removed");
        fs::write(
            python_dir.join(format!(
                "{PYTHON_DISTRIBUTION_NAME}-1.2.3-cp311-cp311-win_amd64.whl"
            )),
            "wheel",
        )
        .expect("wrong-ABI wheel fixture should be written");

        let error = validate_python_release_artifacts(&publish, "1.2.3")
            .expect_err("wrong ABI should fail validation");

        assert!(error.contains("unsupported Python wheel ABI tag `cp311`"));
        assert!(error.contains("expected `abi3`"));
    }

    #[test]
    fn validate_python_wheel_tags_rejects_duplicate_platforms() {
        let mut tags = PLATFORMS
            .iter()
            .map(expected_python_wheel_tag)
            .collect::<Vec<_>>();
        let duplicate = tags[0].clone();
        tags.push(duplicate);

        let error = validate_python_wheel_tags(&tags).expect_err("duplicate platform should fail");

        assert!(error.contains("duplicate Python wheel for platform"));
        assert!(error.contains("keep exactly one"));
    }

    #[test]
    fn validate_python_release_artifacts_rejects_version_mismatch() {
        let root = tempfile::TempDir::new().expect("root should be created");
        write_python_release_fixtures(&root.path().join("publish").join("python"), "1.2.3");

        let error = validate_python_release_artifacts(&root.path().join("publish"), "9.9.9")
            .expect_err("mismatched version should fail validation");

        assert!(error.contains("unexpected Python wheel filename"));
    }

    #[test]
    fn validate_python_release_artifacts_rejects_wrong_sdist_name() {
        let root = tempfile::TempDir::new().expect("root should be created");
        let python_dir = root.path().join("publish").join("python");
        fs::create_dir_all(&python_dir).expect("publish/python should be created");
        for platform in PLATFORMS {
            fs::write(
                python_dir.join(format!(
                    "{PYTHON_DISTRIBUTION_NAME}-1.2.3-{}.whl",
                    expected_python_wheel_tag(platform)
                )),
                "wheel",
            )
            .expect("wheel fixture should be written");
        }
        // Wrong sdist filename (stale version) alongside correctly-versioned wheels.
        fs::write(
            python_dir.join(format!("{PYTHON_DISTRIBUTION_NAME}-1.2.2.tar.gz")),
            "sdist",
        )
        .expect("sdist fixture should be written");

        let error = validate_python_release_artifacts(&root.path().join("publish"), "1.2.3")
            .expect_err("stale sdist version should fail validation");

        assert!(error.contains("unexpected Python sdist filename"));
    }

    #[test]
    fn validate_release_artifact_counts_accepts_expected_layout() {
        let root = tempfile::TempDir::new().expect("root should be created");
        for directory in ["npm", "crates", "nuget", "standalone", "python"] {
            fs::create_dir_all(root.path().join("publish").join(directory))
                .expect("publish directory should be created");
        }
        write_numbered_files(root.path().join("publish/npm"), 9, "tgz");
        write_numbered_files(root.path().join("publish/crates"), 15, "crate");
        write_numbered_files(root.path().join("publish/nuget"), 8, "nupkg");
        write_numbered_files(root.path().join("publish/nuget"), 2, "snupkg");
        write_numbered_files(root.path().join("publish/standalone"), 20, "asset");
        write_python_release_fixtures(&root.path().join("publish/python"), "1.2.3");

        assert!(validate_release_artifact_counts(root.path(), "1.2.3").is_ok());
    }

    #[test]
    fn validate_release_artifact_counts_rejects_missing_package() {
        let root = tempfile::TempDir::new().expect("root should be created");
        for directory in ["npm", "crates", "nuget", "standalone"] {
            fs::create_dir_all(root.path().join("publish").join(directory))
                .expect("publish directory should be created");
        }
        write_numbered_files(root.path().join("publish/npm"), 8, "tgz");
        write_numbered_files(root.path().join("publish/crates"), 15, "crate");
        write_numbered_files(root.path().join("publish/nuget"), 8, "nupkg");
        write_numbered_files(root.path().join("publish/nuget"), 2, "snupkg");
        write_numbered_files(root.path().join("publish/standalone"), 20, "asset");

        let error = validate_release_artifact_counts(root.path(), "1.2.3")
            .expect_err("missing npm package should fail validation");

        assert!(error.contains("expected 9 npm packages, found 8"));
    }

    fn write_numbered_files(directory: PathBuf, count: u32, extension: &str) {
        for index in 0..count {
            fs::write(directory.join(format!("{index}.{extension}")), "")
                .expect("artifact fixture should be written");
        }
    }

    /// Write six correctly-tagged wheel fixtures plus one matching sdist into
    /// `python_dir`, mirroring what a real `AssembleRelease` job would have
    /// pre-staged before `publish-stage --pack-only` runs.
    fn write_python_release_fixtures(python_dir: &Path, version: &str) {
        fs::create_dir_all(python_dir).expect("publish/python should be created");
        for platform in PLATFORMS {
            fs::write(
                python_dir.join(format!(
                    "{PYTHON_DISTRIBUTION_NAME}-{version}-{}.whl",
                    expected_python_wheel_tag(platform)
                )),
                "wheel",
            )
            .expect("wheel fixture should be written");
        }
        fs::write(
            python_dir.join(format!("{PYTHON_DISTRIBUTION_NAME}-{version}.tar.gz")),
            "sdist",
        )
        .expect("sdist fixture should be written");
    }

    #[test]
    fn test_copy_files_with_extension() {
        let src = tempfile::TempDir::new().unwrap();
        let dest = tempfile::TempDir::new().unwrap();
        fs::write(src.path().join("pkg.crate"), "data").unwrap();
        fs::write(src.path().join("other.txt"), "nope").unwrap();

        copy_files_with_extension(src.path(), dest.path(), "crate").unwrap();

        assert!(dest.path().join("pkg.crate").exists());
        assert!(!dest.path().join("other.txt").exists());
    }

    #[test]
    fn test_copy_directory_contents_preserves_subdirectories() {
        let src = tempfile::TempDir::new().unwrap();
        let dest = tempfile::TempDir::new().unwrap();
        fs::create_dir_all(src.path().join("handler")).unwrap();
        fs::write(
            src.path().join("handler").join("webui_wasm_handler.js"),
            "js",
        )
        .unwrap();
        fs::write(
            src.path()
                .join("handler")
                .join("webui_wasm_handler_bg.wasm"),
            "wasm",
        )
        .unwrap();

        let copied = copy_directory_contents(src.path(), dest.path()).unwrap();

        assert_eq!(copied, 2);
        assert!(dest
            .path()
            .join("handler")
            .join("webui_wasm_handler.js")
            .exists());
        assert!(dest
            .path()
            .join("handler")
            .join("webui_wasm_handler_bg.wasm")
            .exists());
    }

    #[test]
    fn export_native_target_preserves_pipeline_layout() {
        let root = tempfile::TempDir::new().expect("root should be created");
        let output = tempfile::TempDir::new().expect("output should be created");
        fs::create_dir_all(root.path().join("publish/native"))
            .expect("native directory should be created");
        fs::create_dir_all(root.path().join("packages/webui-linux-x64"))
            .expect("package directory should be created");
        fs::create_dir_all(root.path().join("dotnet/runtimes/linux-x64/native"))
            .expect("runtime directory should be created");
        fs::write(root.path().join("publish/native/webui-linux-x64"), "cli")
            .expect("native fixture should be written");
        fs::write(
            root.path().join("packages/webui-linux-x64/package.json"),
            "{}",
        )
        .expect("package fixture should be written");
        fs::write(
            root.path()
                .join("dotnet/runtimes/linux-x64/native/libwebui_ffi.so"),
            "ffi",
        )
        .expect("runtime fixture should be written");

        export_target(
            root.path(),
            output.path(),
            "x86_64-unknown-linux-gnu",
            BuildMode::NativeOnly,
        )
        .expect("native artifacts should be exported");

        assert!(output
            .path()
            .join("publish/native/webui-linux-x64")
            .is_file());
        assert!(output
            .path()
            .join("packages/webui-linux-x64/package.json")
            .is_file());
        assert!(output
            .path()
            .join("dotnet/runtimes/linux-x64/native/libwebui_ffi.so")
            .is_file());
    }

    #[test]
    fn export_native_target_rejects_workspace_root() {
        let root = tempfile::TempDir::new().expect("root should be created");
        let error = export_target(
            root.path(),
            root.path(),
            "x86_64-unknown-linux-gnu",
            BuildMode::NativeOnly,
        )
        .expect_err("workspace root should be rejected");

        assert!(error.contains("unsafe export output directory"));
        assert!(root.path().exists());
    }

    #[test]
    fn export_native_target_rejects_filesystem_root() {
        let root = tempfile::TempDir::new().expect("root should be created");
        let filesystem_root = root
            .path()
            .ancestors()
            .last()
            .expect("filesystem root should exist");
        let error = export_target(
            root.path(),
            filesystem_root,
            "x86_64-unknown-linux-gnu",
            BuildMode::NativeOnly,
        )
        .expect_err("filesystem root should be rejected");

        assert!(error.contains("unsafe export output directory"));
        assert!(root.path().exists());
    }

    #[test]
    fn export_native_target_rejects_relative_workspace_escape() {
        let parent = tempfile::TempDir::new().expect("parent should be created");
        let root = parent.path().join("workspace");
        let sibling = parent.path().join("sibling");
        fs::create_dir_all(&root).expect("workspace should be created");
        fs::create_dir_all(&sibling).expect("sibling should be created");
        fs::write(sibling.join("keep.txt"), "keep").expect("fixture should be written");

        let error = export_target(
            &root,
            Path::new("../sibling"),
            "x86_64-unknown-linux-gnu",
            BuildMode::NativeOnly,
        )
        .expect_err("relative workspace escape should be rejected");

        assert!(error.contains("must remain within the workspace"));
        assert!(sibling.join("keep.txt").is_file());
    }

    #[cfg(unix)]
    #[test]
    fn export_native_target_rejects_symlinked_ancestor() {
        use std::os::unix::fs::symlink;

        let parent = tempfile::TempDir::new().expect("parent should be created");
        let root = parent.path().join("workspace");
        let external = parent.path().join("external");
        fs::create_dir_all(&root).expect("workspace should be created");
        fs::create_dir_all(&external).expect("external directory should be created");
        fs::write(external.join("keep.txt"), "keep").expect("fixture should be written");
        symlink(&external, root.join("artifacts")).expect("symlink should be created");

        let error = export_target(
            &root,
            Path::new("artifacts/stage"),
            "x86_64-unknown-linux-gnu",
            BuildMode::NativeOnly,
        )
        .expect_err("symlinked ancestor should be rejected");

        assert!(error.contains("symlinked path component"));
        assert!(external.join("keep.txt").is_file());
    }

    #[test]
    fn test_stage_built_wasm_artifacts_populates_npm_and_publish_outputs() {
        let root = tempfile::TempDir::new().unwrap();
        let source = tempfile::TempDir::new().unwrap();
        let package_wasm = root.path().join("packages/webui/wasm");
        fs::create_dir_all(package_wasm.join("handler")).unwrap();
        fs::write(package_wasm.join(".gitkeep"), "").unwrap();
        fs::write(package_wasm.join("tracked.txt"), "tracked").unwrap();
        fs::write(package_wasm.join("handler/stale.js"), "stale").unwrap();
        let handler = source.path().join("handler");
        fs::create_dir_all(&handler).unwrap();
        fs::write(handler.join("webui_wasm_handler.js"), "js").unwrap();
        fs::write(handler.join("webui_wasm_handler_bg.wasm"), "wasm").unwrap();

        let copied = stage_built_wasm_artifacts(root.path(), source.path()).unwrap();

        assert_eq!(copied, 2);
        assert!(package_wasm.join(".gitkeep").exists());
        assert!(package_wasm.join("tracked.txt").exists());
        assert!(!package_wasm.join("handler/stale.js").exists());
        for output in [package_wasm, root.path().join("publish/wasm")] {
            assert!(output.join("handler/webui_wasm_handler.js").exists());
            assert!(output.join("handler/webui_wasm_handler_bg.wasm").exists());
        }
    }

    #[test]
    fn test_stage_standalone_release_assets_copies_expected_files() {
        let root = tempfile::TempDir::new().unwrap();
        let publish = root.path().join("publish");
        for (source, _) in STANDALONE_RELEASE_FILES {
            let source = publish.join(source);
            fs::create_dir_all(source.parent().unwrap()).unwrap();
            fs::write(source, "asset").unwrap();
        }

        let copied = stage_standalone_release_assets(root.path()).unwrap();

        assert_eq!(copied, 20);
        let output = publish.join("standalone");
        for (_, destination) in STANDALONE_RELEASE_FILES {
            assert!(output.join(destination).is_file());
        }
        assert_eq!(fs::read_dir(output).unwrap().count(), 20);
    }

    #[test]
    fn test_detect_host_triple_format() {
        let triple = detect_host_triple();
        assert!(
            triple.contains('-'),
            "host triple should contain a dash: {triple}"
        );
    }

    #[test]
    fn test_is_private_package_true() {
        let dir = tempfile::TempDir::new().unwrap();
        fs::write(
            dir.path().join("package.json"),
            r#"{"name": "@microsoft/webui-test-support", "private": true}"#,
        )
        .unwrap();
        assert!(is_private_package(dir.path()));
    }

    #[test]
    fn test_is_private_package_false_when_absent() {
        let dir = tempfile::TempDir::new().unwrap();
        fs::write(
            dir.path().join("package.json"),
            r#"{"name": "@microsoft/webui"}"#,
        )
        .unwrap();
        assert!(!is_private_package(dir.path()));
    }

    #[test]
    fn test_is_private_package_false_when_explicit_false() {
        let dir = tempfile::TempDir::new().unwrap();
        fs::write(
            dir.path().join("package.json"),
            r#"{"name": "@microsoft/webui", "private": false}"#,
        )
        .unwrap();
        assert!(!is_private_package(dir.path()));
    }

    #[test]
    fn test_is_private_package_no_package_json() {
        let dir = tempfile::TempDir::new().unwrap();
        assert!(!is_private_package(dir.path()));
    }
}

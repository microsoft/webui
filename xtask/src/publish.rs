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

use crate::util::{build_command, run_command, run_command_quiet};
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
    },
    PlatformEntry {
        triple: "aarch64-unknown-linux-gnu",
        npm_package: "webui-linux-arm64",
        nuget_rid: "linux-arm64",
        ffi_lib: "libwebui_ffi.so",
        node_addon: "libwebui_node.so",
        cli_binary: "webui",
        platform_suffix: "linux-arm64",
    },
    PlatformEntry {
        triple: "x86_64-pc-windows-msvc",
        npm_package: "webui-win32-x64",
        nuget_rid: "win-x64",
        ffi_lib: "webui_ffi.dll",
        node_addon: "webui_node.dll",
        cli_binary: "webui.exe",
        platform_suffix: "win32-x64",
    },
    PlatformEntry {
        triple: "aarch64-pc-windows-msvc",
        npm_package: "webui-win32-arm64",
        nuget_rid: "win-arm64",
        ffi_lib: "webui_ffi.dll",
        node_addon: "webui_node.dll",
        cli_binary: "webui.exe",
        platform_suffix: "win32-arm64",
    },
    PlatformEntry {
        triple: "x86_64-apple-darwin",
        npm_package: "webui-darwin-x64",
        nuget_rid: "osx-x64",
        ffi_lib: "libwebui_ffi.dylib",
        node_addon: "libwebui_node.dylib",
        cli_binary: "webui",
        platform_suffix: "darwin-x64",
    },
    PlatformEntry {
        triple: "aarch64-apple-darwin",
        npm_package: "webui-darwin-arm64",
        nuget_rid: "osx-arm64",
        ffi_lib: "libwebui_ffi.dylib",
        node_addon: "libwebui_node.dylib",
        cli_binary: "webui",
        platform_suffix: "darwin-arm64",
    },
];

/// Subdirectories created inside `publish/`.
const PUBLISH_SUBDIRS: &[&str] = &["native", "npm", "nuget", "crates", "wasm", "standalone"];
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

#[derive(Debug, Eq, PartialEq)]
struct BuildOptions {
    target_triples: Vec<String>,
    profile: String,
    output_root: Option<PathBuf>,
}

// ── Public entry point ──────────────────────────────────────────────────

/// Build and stage native release artifacts for one or more target triples.
///
/// Usage: `cargo xtask publish-build --target <triple> [--target <triple>] [--profile release|debug]`
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

    for triple in &options.target_triples {
        eprintln!(
            "\n{} Building native release artifacts for {}",
            console::style("▸").cyan().bold(),
            console::style(triple).bold(),
        );
        if let Err(error) = build_native_target(&root, triple, &options.profile) {
            eprintln!(
                "  {} Failed to build {triple}: {error}",
                console::style("✘").red().bold(),
            );
            return ExitCode::FAILURE;
        }
    }

    if let Err(error) = stage_native_targets(
        &root,
        options.target_triples.iter().map(String::as_str),
        &options.profile,
    ) {
        eprintln!(
            "  {} Failed to stage native release artifacts: {error}",
            console::style("✘").red().bold(),
        );
        return ExitCode::FAILURE;
    }

    if let Some(output_root) = &options.output_root {
        if let Err(error) = export_native_targets(&root, output_root, &options.target_triples) {
            eprintln!(
                "  {} Failed to export native release artifacts: {error}",
                console::style("✘").red().bold(),
            );
            return ExitCode::FAILURE;
        }
    }

    eprintln!(
        "\n{} Native release artifacts built and staged\n",
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

    if let Err(e) = validate_release_artifact_counts(&root) {
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

/// Create the `publish/` directory tree, cleaning it first if it exists.
fn prepare_publish_dirs(root: &Path, mode: StageMode) -> Result<(), String> {
    let publish_dir = root.join("publish");

    match mode {
        StageMode::Full | StageMode::NativeOnly => {
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
            fs::create_dir_all(publish_dir.join("native"))
                .map_err(|e| format!("failed to create publish/native: {e}"))?;

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

fn parse_build_options(args: &[String]) -> Result<BuildOptions, String> {
    let mut target_triples = Vec::new();
    let mut profile = String::from("release");
    let mut output_root = None;
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
                if !target_triples.contains(triple) {
                    target_triples.push(triple.clone());
                }
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
            argument => return Err(format!("unknown publish-build argument: {argument}")),
        }
        i += 1;
    }

    if target_triples.is_empty() {
        return Err("publish-build requires at least one --target".to_string());
    }

    Ok(BuildOptions {
        target_triples,
        profile,
        output_root,
    })
}

fn build_native_target(root: &Path, triple: &str, profile: &str) -> Result<(), String> {
    let args = native_build_args(triple, profile)?;
    run_command("cargo", &args, Some(root))
}

fn native_build_args<'a>(triple: &'a str, profile: &str) -> Result<Vec<&'a str>, String> {
    let mut args = Vec::with_capacity(13);
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

fn export_native_targets(
    root: &Path,
    output_root: &Path,
    target_triples: &[String],
) -> Result<(), String> {
    let safe_output_root = validate_export_output_root(root, output_root)?;
    if safe_output_root.exists() {
        fs::remove_dir_all(&safe_output_root)
            .map_err(|error| format!("failed to clean {}: {error}", safe_output_root.display()))?;
    }

    copy_directory_contents(
        &root.join("publish").join("native"),
        &safe_output_root.join("publish").join("native"),
    )?;
    for triple in target_triples {
        let platform = PLATFORMS
            .iter()
            .find(|platform| platform.triple == triple)
            .ok_or_else(|| format!("unknown target triple: {triple}"))?;
        copy_directory_contents(
            &root.join("packages").join(platform.npm_package),
            &safe_output_root.join("packages").join(platform.npm_package),
        )?;
        copy_directory_contents(
            &root
                .join("dotnet")
                .join("runtimes")
                .join(platform.nuget_rid),
            &safe_output_root
                .join("dotnet")
                .join("runtimes")
                .join(platform.nuget_rid),
        )?;
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

    if normalized_output
        .symlink_metadata()
        .is_ok_and(|metadata| metadata.file_type().is_symlink())
    {
        return Err(format!(
            "refusing to clean symlinked export output directory: {}",
            normalized_output.display()
        ));
    }
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
    if normalized_output.is_file() {
        return Err(format!(
            "export output path is a file: {}",
            normalized_output.display()
        ));
    }

    Ok(normalized_output)
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

fn validate_release_artifact_counts(root: &Path) -> Result<(), String> {
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
    )
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
    fn parse_build_options_supports_multiple_targets() {
        let options = parse_build(&[
            "--target",
            "x86_64-unknown-linux-gnu",
            "--target",
            "aarch64-unknown-linux-gnu",
            "--profile",
            "debug",
        ])
        .expect("publish build options should parse");

        assert_eq!(
            options.target_triples,
            ["x86_64-unknown-linux-gnu", "aarch64-unknown-linux-gnu"]
        );
        assert_eq!(options.profile, "debug");
        assert_eq!(options.output_root, None);
    }

    #[test]
    fn parse_build_options_rejects_missing_target() {
        let error = parse_build(&[]).expect_err("a target should be required");

        assert!(error.contains("requires at least one --target"));
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
        let args = native_build_args("x86_64-unknown-linux-gnu", "debug")
            .expect("debug profile should be supported");

        assert_eq!(args[0], "build");
        assert!(!args.contains(&"--profile"));
        assert!(!args.contains(&"--release"));
    }

    #[test]
    fn native_build_args_map_release_to_release_flag() {
        let args = native_build_args("x86_64-unknown-linux-gnu", "release")
            .expect("release profile should be supported");

        assert!(args.contains(&"--release"));
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
    fn validate_release_artifact_counts_accepts_expected_layout() {
        let root = tempfile::TempDir::new().expect("root should be created");
        for directory in ["npm", "crates", "nuget", "standalone"] {
            fs::create_dir_all(root.path().join("publish").join(directory))
                .expect("publish directory should be created");
        }
        write_numbered_files(root.path().join("publish/npm"), 9, "tgz");
        write_numbered_files(root.path().join("publish/crates"), 15, "crate");
        write_numbered_files(root.path().join("publish/nuget"), 8, "nupkg");
        write_numbered_files(root.path().join("publish/nuget"), 2, "snupkg");
        write_numbered_files(root.path().join("publish/standalone"), 20, "asset");

        assert!(validate_release_artifact_counts(root.path()).is_ok());
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

        let error = validate_release_artifact_counts(root.path())
            .expect_err("missing npm package should fail validation");

        assert!(error.contains("expected 9 npm packages, found 8"));
    }

    fn write_numbered_files(directory: PathBuf, count: u32, extension: &str) {
        for index in 0..count {
            fs::write(directory.join(format!("{index}.{extension}")), "")
                .expect("artifact fixture should be written");
        }
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
    fn export_native_targets_preserves_pipeline_layout() {
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

        export_native_targets(
            root.path(),
            output.path(),
            &["x86_64-unknown-linux-gnu".to_string()],
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
    fn export_native_targets_rejects_workspace_root() {
        let root = tempfile::TempDir::new().expect("root should be created");
        let error = export_native_targets(root.path(), root.path(), &[])
            .expect_err("workspace root should be rejected");

        assert!(error.contains("unsafe export output directory"));
        assert!(root.path().exists());
    }

    #[test]
    fn export_native_targets_rejects_filesystem_root() {
        let root = tempfile::TempDir::new().expect("root should be created");
        let filesystem_root = root
            .path()
            .ancestors()
            .last()
            .expect("filesystem root should exist");
        let error = export_native_targets(root.path(), filesystem_root, &[])
            .expect_err("filesystem root should be rejected");

        assert!(error.contains("unsafe export output directory"));
        assert!(root.path().exists());
    }

    #[test]
    fn export_native_targets_rejects_relative_workspace_escape() {
        let parent = tempfile::TempDir::new().expect("parent should be created");
        let root = parent.path().join("workspace");
        let sibling = parent.path().join("sibling");
        fs::create_dir_all(&root).expect("workspace should be created");
        fs::create_dir_all(&sibling).expect("sibling should be created");
        fs::write(sibling.join("keep.txt"), "keep").expect("fixture should be written");

        let error = export_native_targets(&root, Path::new("../sibling"), &[])
            .expect_err("relative workspace escape should be rejected");

        assert!(error.contains("must remain within the workspace"));
        assert!(sibling.join("keep.txt").is_file());
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

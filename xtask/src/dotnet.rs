//! `cargo xtask publish-stage` — copy native binaries into npm and NuGet package directories.

use std::path::{Path, PathBuf};
use std::process::ExitCode;

/// Mapping from Rust target triple to platform identifiers and binary filenames.
struct PlatformEntry {
    triple: &'static str,
    npm_package: &'static str,
    nuget_rid: &'static str,
    ffi_lib: &'static str,
    node_addon: &'static str,
    cli_binary: &'static str,
}

const PLATFORMS: &[PlatformEntry] = &[
    PlatformEntry {
        triple: "x86_64-unknown-linux-gnu",
        npm_package: "webui-linux-x64",
        nuget_rid: "linux-x64",
        ffi_lib: "libwebui_ffi.so",
        node_addon: "libwebui_node.so",
        cli_binary: "webui",
    },
    PlatformEntry {
        triple: "aarch64-unknown-linux-gnu",
        npm_package: "webui-linux-arm64",
        nuget_rid: "linux-arm64",
        ffi_lib: "libwebui_ffi.so",
        node_addon: "libwebui_node.so",
        cli_binary: "webui",
    },
    PlatformEntry {
        triple: "x86_64-pc-windows-msvc",
        npm_package: "webui-win32-x64",
        nuget_rid: "win-x64",
        ffi_lib: "webui_ffi.dll",
        node_addon: "webui_node.dll",
        cli_binary: "webui.exe",
    },
    PlatformEntry {
        triple: "aarch64-pc-windows-msvc",
        npm_package: "webui-win32-arm64",
        nuget_rid: "win-arm64",
        ffi_lib: "webui_ffi.dll",
        node_addon: "webui_node.dll",
        cli_binary: "webui.exe",
    },
    PlatformEntry {
        triple: "x86_64-apple-darwin",
        npm_package: "webui-darwin-x64",
        nuget_rid: "osx-x64",
        ffi_lib: "libwebui_ffi.dylib",
        node_addon: "libwebui_node.dylib",
        cli_binary: "webui",
    },
    PlatformEntry {
        triple: "aarch64-apple-darwin",
        npm_package: "webui-darwin-arm64",
        nuget_rid: "osx-arm64",
        ffi_lib: "libwebui_ffi.dylib",
        node_addon: "libwebui_node.dylib",
        cli_binary: "webui",
    },
];

/// Stage native binaries from cargo build output into npm and NuGet package directories.
///
/// Usage: `cargo xtask publish-stage [--target <triple|all>] [--profile release]`
///
/// Pass `--target all` to stage every platform whose build artifacts exist.
/// If `--target` is omitted, detects the current host platform.
///
/// Copies:
///   npm:   packages/webui-{platform}/bin/webui + webui.node
///   NuGet: dotnet/runtimes/{rid}/native/libwebui_ffi.*
pub fn run_stage(args: &[String]) -> ExitCode {
    let root = std::env::current_dir().unwrap_or_default();

    let mut target_triple: Option<&str> = None;
    let mut profile = "release";
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--target" => {
                i += 1;
                if i < args.len() {
                    target_triple = Some(args[i].as_str());
                }
            }
            "--profile" => {
                i += 1;
                if i < args.len() {
                    profile = args[i].as_str();
                }
            }
            _ => {}
        }
        i += 1;
    }

    match target_triple {
        Some("all") => run_stage_all(&root, profile),
        Some(triple) => run_stage_one(&root, triple, profile),
        None => {
            let host = detect_host_triple();
            eprintln!(
                "  {} No --target specified, using host: {}",
                console::style("▸").cyan().bold(),
                console::style(&host).bold(),
            );
            run_stage_one(&root, &host, profile)
        }
    }
}

/// Stage all platforms whose build artifacts exist under target/.
fn run_stage_all(root: &Path, profile: &str) -> ExitCode {
    eprintln!(
        "\n{} Staging all available platforms ({})",
        console::style("▸").cyan().bold(),
        console::style(profile).dim(),
    );

    let host = detect_host_triple();
    let mut staged = 0u32;
    let mut skipped = 0u32;
    let mut failed = 0u32;

    for platform in PLATFORMS {
        // For non-host triples, only check the cross-compiled directory.
        // The host triple can also use target/{profile}/ directly.
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
            console::style("cargo build --release -p webui-ffi -p webui-node -p webui-cli").dim(),
        );
        return ExitCode::FAILURE;
    }

    eprintln!();
    ExitCode::SUCCESS
}

/// Stage a single platform by triple name.
fn run_stage_one(root: &Path, triple: &str, profile: &str) -> ExitCode {
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
        "\n{} Staging native binaries for {} ({})",
        console::style("▸").cyan().bold(),
        console::style(triple).bold(),
        console::style(profile).dim(),
    );

    if stage_platform(root, platform, &build_dir) {
        eprintln!(
            "\n{} All binaries staged for {}\n",
            console::style("✔").green(),
            console::style(platform.triple).bold(),
        );
        ExitCode::SUCCESS
    } else {
        eprintln!(
            "\n{} Some binaries could not be staged (see errors above)\n",
            console::style("⚠").yellow(),
        );
        ExitCode::FAILURE
    }
}

/// Copy all artifacts for a single platform. Returns true if all found files staged.
fn stage_platform(root: &Path, platform: &PlatformEntry, build_dir: &Path) -> bool {
    let mut ok = true;

    // NuGet: FFI library
    ok &= stage_file(&CopySpec {
        src: &build_dir.join(platform.ffi_lib),
        dest_dir: &root
            .join("dotnet/runtimes")
            .join(platform.nuget_rid)
            .join("native"),
        dest_name: platform.ffi_lib,
        label: "nuget",
    });

    // npm: CLI binary
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

    ok
}

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

    if let Err(e) = std::fs::create_dir_all(spec.dest_dir) {
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
    if let Err(e) = std::fs::copy(spec.src, &dest) {
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

    // Strip the root prefix for cleaner output
    let rel = dest
        .strip_prefix(std::env::current_dir().unwrap_or_default())
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
    // Cross-compiled: target/{triple}/{profile}/
    let cross = root.join("target").join(triple).join(profile);
    if cross.exists() {
        return cross;
    }
    // Host build: target/{profile}/
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

//! WASM playground build task.
//!
//! Builds the `webui-wasm` crate to WebAssembly via `wasm-pack` and patches
//! the generated JS glue for browser compatibility.

use crate::util::{run_command, which_exists, Printer};
use std::path::Path;
use std::process::Command;
use std::{env, fs};

/// Build the webui-wasm package for the playground.
pub fn run() -> Result<(), String> {
    let p = Printer::new();
    let crate_dir = Path::new("crates/webui-wasm");
    let out_dir = Path::new("docs/public/wasm");

    // 1. Check wasm-pack is installed
    if !which_exists("wasm-pack") {
        return Err(missing_tool_error(
            &p,
            "wasm-pack",
            &[("All platforms", "cargo install wasm-pack")],
        ));
    }

    // 2. Check wasm32-unknown-unknown target
    let rustup_out = Command::new("rustup")
        .args(["target", "list", "--installed"])
        .output()
        .map_err(|e| format!("Failed to run rustup: {}", e))?;
    let targets = String::from_utf8_lossy(&rustup_out.stdout);
    if !targets.contains("wasm32-unknown-unknown") {
        eprintln!(
            "  {} Adding wasm32-unknown-unknown target...",
            p.dim.apply_to("•")
        );
        run_command("rustup", &["target", "add", "wasm32-unknown-unknown"], None)?;
    }

    // 3. Detect C compiler for wasm32
    let cc = find_wasm_cc().ok_or_else(|| {
        missing_tool_error(
            &p,
            "LLVM clang (with wasm32 support)",
            &[
                ("macOS", "brew install llvm"),
                ("Ubuntu/Debian", "sudo apt install clang"),
                ("Fedora", "sudo dnf install clang"),
                (
                    "Windows",
                    "Download from https://releases.llvm.org/download.html",
                ),
            ],
        )
    })?;
    let ar = find_wasm_ar().unwrap_or_default();

    // 4. Detect WASM include path (wasi-libc headers for tree-sitter C code)
    let wasi_include = find_wasi_include().ok_or_else(|| {
        missing_tool_error(
            &p,
            "WASM C stdlib headers",
            &[
                ("macOS", "brew install wasi-libc"),
                ("Linux", "Install wasi-sdk or set WASI_INCLUDE env var"),
                ("Windows", "Set WASI_INCLUDE env var to wasi-sdk include path"),
                ("All", "Run 'cargo fetch' first — headers may auto-resolve from tree-sitter-language crate"),
            ],
        )
    })?;

    eprintln!("  {} CC:   {}", p.dim.apply_to("•"), p.bold.apply_to(&cc));
    if !ar.is_empty() {
        eprintln!("  {} AR:   {}", p.dim.apply_to("•"), p.bold.apply_to(&ar));
    }
    eprintln!(
        "  {} WASI: {}",
        p.dim.apply_to("•"),
        p.bold.apply_to(&wasi_include)
    );

    // 5. Clean output directory
    if out_dir.exists() {
        fs::remove_dir_all(out_dir)
            .map_err(|e| format!("Failed to clean {}: {}", out_dir.display(), e))?;
    }

    // 6. Run wasm-pack build with env vars
    let cflags = format!("-I{} -D_WASI_EMULATED_SIGNAL", wasi_include);
    let mut cmd = Command::new("wasm-pack");
    cmd.args([
        "build",
        &crate_dir.display().to_string(),
        "--target",
        "web",
        "--out-dir",
        &format!("../../{}", out_dir.display()),
    ]);
    cmd.env("CC_wasm32_unknown_unknown", &cc);
    cmd.env("CFLAGS_wasm32_unknown_unknown", &cflags);
    if !ar.is_empty() {
        cmd.env("AR_wasm32_unknown_unknown", &ar);
    }
    cmd.env("RUSTFLAGS", "-C link-arg=--allow-multiple-definition");

    let status = cmd
        .status()
        .map_err(|e| format!("wasm-pack failed to start: {}", e))?;
    if !status.success() {
        return Err(format!("wasm-pack exited with {}", status));
    }

    // 7. Patch JS glue: replace `import 'env'` with inline C stdlib stubs
    let js_path = out_dir.join("webui_wasm.js");
    patch_wasm_js_glue(&p, &js_path)?;

    // 8. Remove wasm-pack generated .gitignore
    let gitignore = out_dir.join(".gitignore");
    if gitignore.exists() {
        let _ = fs::remove_file(&gitignore);
    }

    // 9. Report
    let wasm_path = out_dir.join("webui_wasm_bg.wasm");
    if let Ok(meta) = fs::metadata(&wasm_path) {
        let size_kb = meta.len() / 1024;
        eprintln!(
            "  {} Output: {}",
            p.green.apply_to("✔"),
            p.bold.apply_to(out_dir.display())
        );
        eprintln!(
            "  {} Size:   {}",
            p.green.apply_to("✔"),
            p.bold.apply_to(format!("{} KB", size_kb))
        );
    }

    Ok(())
}

/// Patch the wasm-bindgen JS glue to replace `import * as __wbg_star0 from 'env'`
/// with inline JavaScript stubs for the C stdlib functions tree-sitter needs.
fn patch_wasm_js_glue(p: &Printer, js_path: &Path) -> Result<(), String> {
    let content = fs::read_to_string(js_path)
        .map_err(|e| format!("Failed to read {}: {}", js_path.display(), e))?;

    let env_import = "import * as __wbg_star0 from 'env';";
    if !content.contains(env_import) {
        return Ok(()); // Nothing to patch
    }

    eprintln!(
        "  {} Patching JS glue: replacing {} import with inline stubs",
        p.dim.apply_to("•"),
        p.cyan.apply_to("'env'")
    );

    let stub = "const __wbg_star0 = {\n    \
        towupper: (c) => (c >= 97 && c <= 122) ? c - 32 : c,\n    \
        iswspace: (c) => (c === 32 || (c >= 9 && c <= 13)) ? 1 : 0,\n    \
        iswalnum: (c) => ((c >= 48 && c <= 57) || (c >= 65 && c <= 90) || (c >= 97 && c <= 122)) ? 1 : 0,\n    \
        iswalpha: (c) => ((c >= 65 && c <= 90) || (c >= 97 && c <= 122)) ? 1 : 0,\n    \
        iswdigit: (c) => (c >= 48 && c <= 57) ? 1 : 0,\n    \
        iswlower: (c) => (c >= 97 && c <= 122) ? 1 : 0,\n    \
        iswupper: (c) => (c >= 65 && c <= 90) ? 1 : 0,\n    \
        memchr: () => 0,\n    \
        strlen: () => 0,\n\
    };";

    let patched = content.replace(env_import, stub);
    fs::write(js_path, patched)
        .map_err(|e| format!("Failed to write {}: {}", js_path.display(), e))?;

    Ok(())
}

// ── Toolchain detection ─────────────────────────────────────────────────

/// Find a C compiler that supports wasm32 targets.
fn find_wasm_cc() -> Option<String> {
    if let Ok(cc) = env::var("CC_wasm32_unknown_unknown") {
        if !cc.is_empty() {
            return Some(cc);
        }
    }

    if cfg!(target_os = "macos") {
        for path in &[
            "/opt/homebrew/opt/llvm/bin/clang",
            "/usr/local/opt/llvm/bin/clang",
        ] {
            if Path::new(path).exists() {
                return Some(path.to_string());
            }
        }
    }

    if cfg!(target_os = "windows") {
        for path in &[
            r"C:\Program Files\LLVM\bin\clang.exe",
            r"C:\Program Files (x86)\LLVM\bin\clang.exe",
        ] {
            if Path::new(path).exists() {
                return Some(path.to_string());
            }
        }
    }

    if which_exists("clang") {
        return Some("clang".to_string());
    }

    None
}

/// Find llvm-ar for wasm32 archiving.
fn find_wasm_ar() -> Option<String> {
    if let Ok(ar) = env::var("AR_wasm32_unknown_unknown") {
        if !ar.is_empty() {
            return Some(ar);
        }
    }

    if cfg!(target_os = "macos") {
        for path in &[
            "/opt/homebrew/opt/llvm/bin/llvm-ar",
            "/usr/local/opt/llvm/bin/llvm-ar",
        ] {
            if Path::new(path).exists() {
                return Some(path.to_string());
            }
        }
    }

    if cfg!(target_os = "windows") {
        for path in &[
            r"C:\Program Files\LLVM\bin\llvm-ar.exe",
            r"C:\Program Files (x86)\LLVM\bin\llvm-ar.exe",
        ] {
            if Path::new(path).exists() {
                return Some(path.to_string());
            }
        }
    }

    if which_exists("llvm-ar") {
        return Some("llvm-ar".to_string());
    }

    None
}

/// Find wasi-libc include directory with C stdlib headers for WASM.
/// Falls back to tree-sitter-language's bundled WASM headers in the cargo registry.
fn find_wasi_include() -> Option<String> {
    if let Ok(inc) = env::var("WASI_INCLUDE") {
        if Path::new(&inc).is_dir() {
            return Some(inc);
        }
    }

    if cfg!(target_os = "macos") {
        for prefix in &["/opt/homebrew", "/usr/local"] {
            let path = format!(
                "{}/opt/wasi-libc/share/wasi-sysroot/include/wasm32-wasi",
                prefix
            );
            if Path::new(&path).is_dir() {
                return Some(path);
            }
        }
    }

    // Linux: check common wasi-libc paths (apt package installs to /usr/include/)
    for path in &[
        "/usr/include/wasm32-wasi",
        "/usr/share/wasi-sysroot/include/wasm32-wasi",
        "/opt/wasi-sdk/share/wasi-sysroot/include/wasm32-wasi",
    ] {
        if Path::new(path).is_dir() {
            return Some(path.to_string());
        }
    }

    if cfg!(target_os = "windows") {
        if let Ok(home) = env::var("USERPROFILE") {
            let path = format!(r"{}\wasi-sdk\share\wasi-sysroot\include\wasm32-wasi", home);
            if Path::new(&path).is_dir() {
                return Some(path);
            }
        }
    }

    // Fallback: tree-sitter-language ships minimal WASM headers in cargo registry
    find_tree_sitter_wasm_headers()
}

/// Search the cargo registry for tree-sitter-language's bundled WASM include dir.
fn find_tree_sitter_wasm_headers() -> Option<String> {
    let cargo_home = env::var("CARGO_HOME").unwrap_or_else(|_| {
        let home = env::var("HOME")
            .or_else(|_| env::var("USERPROFILE"))
            .unwrap_or_default();
        format!("{}/.cargo", home)
    });

    let registry_src = Path::new(&cargo_home).join("registry").join("src");
    if !registry_src.is_dir() {
        return None;
    }

    // Walk index directories looking for tree-sitter-language-*/wasm/include
    if let Ok(entries) = fs::read_dir(&registry_src) {
        for index_entry in entries.flatten() {
            let index_dir = index_entry.path();
            if !index_dir.is_dir() {
                continue;
            }
            if let Ok(crates) = fs::read_dir(&index_dir) {
                for crate_entry in crates.flatten() {
                    let name = crate_entry.file_name();
                    let name_str = name.to_string_lossy();
                    if name_str.starts_with("tree-sitter-language-") {
                        let inc = crate_entry.path().join("wasm").join("include");
                        if inc.is_dir() {
                            return Some(inc.to_string_lossy().to_string());
                        }
                    }
                }
            }
        }
    }

    None
}

/// Build a friendly, colorful error message for a missing tool.
fn missing_tool_error(p: &Printer, tool: &str, install_hints: &[(&str, &str)]) -> String {
    let mut msg = format!(
        "\n  {} {} not found\n\n  {} Install it using one of:\n",
        p.red.apply_to("✘"),
        p.bold.apply_to(tool),
        p.yellow.apply_to("hint:"),
    );
    for (platform, command) in install_hints {
        msg.push_str(&format!(
            "\n    {} {:<16} {}",
            p.dim.apply_to("▸"),
            p.dim.apply_to(platform),
            p.cyan.apply_to(command),
        ));
    }
    msg.push('\n');
    msg
}

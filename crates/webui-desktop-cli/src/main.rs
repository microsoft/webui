// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
#[cfg(not(target_os = "macos"))]
use std::sync::Arc;
use std::sync::OnceLock;
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result};
use clap::{Args, Parser, Subcommand, ValueEnum};
use expand_tilde::expand_tilde;
use webui::DEFAULT_CSS_FILE_NAME_TEMPLATE;
use webui_desktop::{
    build_desktop_bundle, package_desktop_bundle, DesktopBundleOptions, DesktopPackageOptions,
    DesktopPackageTarget, DesktopShellConfig, WindowOptions,
};
#[cfg(not(target_os = "macos"))]
use webui_desktop::{DesktopRuntime, DesktopSourceConfig};

#[derive(Parser)]
#[command(name = "webui-desktop", about = "WebUI desktop runner and packager")]
struct Cli {
    /// Output format: `human` or `json`
    #[arg(long, value_enum, default_value_t = OutputFormat::Human, global = true)]
    format: OutputFormat,

    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, ValueEnum)]
enum OutputFormat {
    #[default]
    Human,
    Json,
}

static FORMAT: OnceLock<OutputFormat> = OnceLock::new();

fn set_format(format: OutputFormat) {
    let _ = FORMAT.set(format);
}

fn is_json() -> bool {
    matches!(
        FORMAT.get().copied().unwrap_or_default(),
        OutputFormat::Json
    )
}

#[derive(Subcommand)]
enum Commands {
    /// Run a WebUI app in a native desktop window
    Run(RunArgs),
    /// Build an immutable desktop bundle
    Build(BuildArgs),
    /// Package a desktop bundle for a native target
    Package(PackageArgs),
}

#[derive(Args)]
struct RunArgs {
    #[command(flatten)]
    app: AppArgs,

    /// Path to the JSON state file used for startup render
    #[arg(long)]
    state: Option<PathBuf>,

    /// Optional directory for static desktop assets
    #[arg(long)]
    servedir: Option<PathBuf>,

    /// Design token theme: a path to a JSON file or an npm package name
    #[arg(long)]
    theme: Option<String>,

    /// Rebuild and reload on source changes
    #[arg(long)]
    watch: bool,

    #[command(flatten)]
    window: WindowArgs,
}

#[derive(Args)]
struct BuildArgs {
    #[command(flatten)]
    app: AppArgs,

    /// Output desktop bundle directory
    #[arg(long)]
    out: PathBuf,

    /// Path to the JSON state file used for startup render
    #[arg(long)]
    state: Option<PathBuf>,

    /// Optional directory for static desktop assets
    #[arg(long)]
    servedir: Option<PathBuf>,

    /// Design token theme: a path to a JSON file or an npm package name
    #[arg(long)]
    theme: Option<String>,

    /// Reverse-DNS application identifier
    #[arg(long, default_value = "com.microsoft.webui.app")]
    app_id: String,

    /// Human-readable application name
    #[arg(long, default_value = "WebUI App")]
    app_name: String,

    /// Application version
    #[arg(long, default_value = "0.0.0")]
    app_version: String,

    /// Publisher name
    #[arg(long, default_value = "Microsoft")]
    publisher: String,

    /// Optional app icon file copied into the desktop bundle
    #[arg(long)]
    icon: Option<PathBuf>,

    #[command(flatten)]
    window: WindowArgs,
}

#[derive(Args)]
struct PackageArgs {
    /// Desktop bundle directory, or a WebUI app root with webuiDesktop config
    bundle: PathBuf,

    /// Package target or `all`
    #[arg(long, default_value = "macos-app")]
    target: String,

    /// Output directory for package artifacts
    #[arg(long)]
    out: PathBuf,

    /// Runner executable to package; use an app-specific Rust host for route/IPC apps
    #[arg(long)]
    runner: Option<PathBuf>,

    /// Design token theme override for app-root packaging
    #[arg(long)]
    theme: Option<String>,

    /// Optional app icon override for app-root packaging
    #[arg(long)]
    icon: Option<PathBuf>,

    /// Cargo package name for the app-specific Rust desktop runner
    #[arg(long)]
    runner_crate: Option<String>,

    /// Build the runner in release mode before packaging
    #[arg(long)]
    release: bool,

    /// Optional desktop bundle output directory to keep; defaults to a temporary bundle
    #[arg(long)]
    bundle_out: Option<PathBuf>,

    /// Skip configured web build scripts before building the desktop bundle
    #[arg(long)]
    no_web_build: bool,
}

#[derive(Args, Clone)]
struct AppArgs {
    /// Path to the app folder
    #[arg(default_value = ".")]
    app: PathBuf,

    /// Entry HTML file name
    #[arg(long, default_value = "index.html")]
    entry: String,

    /// CSS delivery strategy
    #[arg(long, value_enum, default_value_t = webui::CssStrategy::Link)]
    css: webui::CssStrategy,

    /// DOM strategy
    #[arg(long, value_enum, default_value_t = webui::DomStrategy::Shadow)]
    dom: webui::DomStrategy,

    /// Framework plugin to load
    #[arg(long, value_enum)]
    plugin: Option<webui::Plugin>,

    /// Additional component sources
    #[arg(long, value_name = "SOURCE")]
    components: Vec<String>,

    /// Link-mode CSS filename template using [name], [hash], [ext]
    #[arg(long, default_value = DEFAULT_CSS_FILE_NAME_TEMPLATE)]
    css_file_name_template: String,

    /// Optional base URL/path prefix for Link-mode CSS hrefs
    #[arg(long)]
    css_public_base: Option<String>,

    /// Legal comment handling
    #[arg(long, value_enum, default_value_t = webui::LegalComments::Inline)]
    legal_comments: webui::LegalComments,
}

impl AppArgs {
    fn build_options(&self, app_dir: PathBuf) -> webui::BuildOptions {
        webui::BuildOptions {
            app_dir,
            entry: self.entry.clone(),
            css: self.css,
            dom: self.dom,
            plugin: self.plugin,
            components: self.components.clone(),
            component_asset_roots: Vec::new(),
            css_file_name_template: self.css_file_name_template.clone(),
            css_public_base: self.css_public_base.clone(),
            legal_comments: self.legal_comments,
        }
    }
}

#[derive(Args)]
struct WindowArgs {
    /// Window title
    #[arg(long, default_value = "WebUI")]
    title: String,

    /// Initial window width
    #[arg(long, default_value_t = 1200)]
    width: u32,

    /// Initial window height
    #[arg(long, default_value_t = 800)]
    height: u32,

    /// Enable web inspector/devtools for the desktop webview
    #[arg(long)]
    devtools: bool,
}

#[derive(Default)]
struct DesktopAppPackageConfig {
    entry: Option<String>,
    source: Option<PathBuf>,
    state: Option<PathBuf>,
    assets: Option<PathBuf>,
    icon: Option<PathBuf>,
    theme: Option<String>,
    runner_crate: Option<String>,
    package_manager: Option<String>,
    build_scripts: Option<Vec<String>>,
    app_id: Option<String>,
    app_name: Option<String>,
    app_version: Option<String>,
    publisher: Option<String>,
    title: Option<String>,
    width: Option<u32>,
    height: Option<u32>,
    devtools: Option<bool>,
    plugin: Option<webui::Plugin>,
}

struct AppPackagePlan {
    app_root: PathBuf,
    source_dir: PathBuf,
    entry: String,
    state_file: Option<PathBuf>,
    staged_assets: Option<PathBuf>,
    icon_file: Option<PathBuf>,
    bundle_dir: PathBuf,
    runner_exe: PathBuf,
    token_css: Option<std::collections::HashMap<String, String>>,
    app_id: String,
    app_name: String,
    app_version: String,
    publisher: String,
    window: WindowOptions,
    plugin: Option<webui::Plugin>,
}

struct TempWorkDir {
    path: PathBuf,
    keep: bool,
}

impl TempWorkDir {
    fn new(prefix: &str) -> Result<Self> {
        let path = std::env::temp_dir().join(format!(
            "{prefix}-{}-{}",
            std::process::id(),
            monotonic_millis()
        ));
        fs::create_dir_all(&path)
            .with_context(|| format!("Failed to create temporary directory {}", path.display()))?;
        Ok(Self { path, keep: false })
    }
}

impl Drop for TempWorkDir {
    fn drop(&mut self) {
        if !self.keep {
            let _ = fs::remove_dir_all(&self.path);
        }
    }
}

fn monotonic_millis() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis())
        .unwrap_or(0)
}

fn main() {
    let cli = Cli::parse();
    set_format(cli.format);
    if let Err(err) = run(cli) {
        print_error(&err);
        std::process::exit(1);
    }
}

fn run(cli: Cli) -> Result<()> {
    match cli.command {
        Some(Commands::Run(args)) => run_desktop(args),
        Some(Commands::Build(args)) => build_bundle(args),
        Some(Commands::Package(args)) => package_bundle(args),
        None => webui_desktop_runner::run_packaged_app(),
    }
}

fn run_desktop(args: RunArgs) -> Result<()> {
    if args.watch {
        return Err(anyhow::anyhow!(
            "desktop --watch is not wired yet; the native reload worker is implemented in the HMR phase"
        ));
    }

    let app_dir = canonicalize_existing_dir(&args.app.app, "app")?;
    let state_file = optional_existing_file(args.state.as_ref(), "state")?;
    let asset_root = optional_existing_dir(args.servedir.as_ref(), "serve directory")?;

    print_header("WebUI Desktop");
    print_field("App", &app_dir.display());
    print_field("Entry", &args.app.entry);
    if let Some(state) = &state_file {
        print_field("State", &state.display());
    }
    if let Some(assets) = &asset_root {
        print_field("ServeDir", &assets.display());
    }
    print_field(
        "Window",
        &format!("{}x{}", args.window.width, args.window.height),
    );

    #[cfg(target_os = "macos")]
    {
        run_macos_from_source(args, app_dir, state_file, asset_root)
    }

    #[cfg(any(target_os = "linux", target_os = "windows"))]
    let mut config = DesktopSourceConfig::new(args.app.build_options(app_dir));
    #[cfg(any(target_os = "linux", target_os = "windows"))]
    {
        config.state_file = state_file;
        config.asset_root = asset_root;
        let runtime =
            DesktopRuntime::from_source(config).with_context(|| "Desktop build failed")?;
        run_webview(Arc::new(runtime), &args.window)
    }

    #[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
    {
        let _ = (args, state_file, asset_root);
        Err(anyhow::anyhow!(
            "desktop webview backend is not implemented on this platform yet"
        ))
    }
}

#[cfg(target_os = "macos")]
fn run_macos_from_source(
    args: RunArgs,
    app_dir: PathBuf,
    state_file: Option<PathBuf>,
    asset_root: Option<PathBuf>,
) -> Result<()> {
    let temp_root = std::env::temp_dir().join(format!(
        "webui-desktop-run-{}-{}",
        std::process::id(),
        monotonic_millis()
    ));
    let bundle_dir = temp_root.join("bundle");
    let package_dir = temp_root.join("package");
    let runner_exe =
        std::env::current_exe().with_context(|| "Failed to locate webui-desktop executable")?;

    let token_css = resolve_theme_css(args.theme.as_deref(), &args.app, &app_dir)?;

    build_desktop_bundle(DesktopBundleOptions {
        build_options: args.app.build_options(app_dir),
        out_dir: bundle_dir.clone(),
        state_file,
        asset_root,
        token_css,
        app_id: "com.microsoft.webui.desktop.run".to_string(),
        app_name: args.window.title.clone(),
        version: "0.0.0".to_string(),
        publisher: "Microsoft".to_string(),
        window: WindowOptions {
            title: args.window.title,
            width: args.window.width,
            height: args.window.height,
            maximized: false,
            devtools: args.window.devtools,
        },
        icon_file: None,
        shell: DesktopShellConfig::default(),
        package_targets: Vec::new(),
    })
    .with_context(|| "Desktop run build failed")?;

    let package = package_desktop_bundle(DesktopPackageOptions {
        bundle_dir,
        out_dir: package_dir,
        target: DesktopPackageTarget::MacosApp,
        runner_exe,
    })
    .with_context(|| "Desktop run package failed")?;

    let executable = package.output_path.join("Contents/MacOS/webui-desktop");
    let status = Command::new(&executable)
        .status()
        .with_context(|| format!("Failed to launch {}", executable.display()))?;
    let _ = std::fs::remove_dir_all(&temp_root);
    if status.success() {
        Ok(())
    } else {
        Err(anyhow::anyhow!("desktop app exited with {status}"))
    }
}

fn build_bundle(args: BuildArgs) -> Result<()> {
    let app_dir = canonicalize_existing_dir(&args.app.app, "app")?;
    let state_file = optional_existing_file(args.state.as_ref(), "state")?;
    let asset_root = optional_existing_dir(args.servedir.as_ref(), "serve directory")?;
    let out_dir = expand_path(&args.out, "output")?;
    let token_css = resolve_theme_css(args.theme.as_deref(), &args.app, &app_dir)?;

    print_header("WebUI Desktop Build");
    print_field("App", &app_dir.display());
    print_field("Entry", &args.app.entry);
    print_field("Output", &out_dir.display());

    let manifest = build_desktop_bundle(DesktopBundleOptions {
        build_options: args.app.build_options(app_dir),
        out_dir,
        state_file,
        asset_root,
        token_css,
        app_id: args.app_id,
        app_name: args.app_name,
        version: args.app_version,
        publisher: args.publisher,
        window: WindowOptions {
            title: args.window.title,
            width: args.window.width,
            height: args.window.height,
            maximized: false,
            devtools: args.window.devtools,
        },
        icon_file: optional_existing_file(args.icon.as_ref(), "icon")?,
        shell: DesktopShellConfig::default(),
        package_targets: Vec::new(),
    })
    .with_context(|| "Desktop bundle build failed")?;

    print_field("Protocol", &manifest.protocol_path.display());
    print_field("Assets", &manifest.integrity.assets.len());
    print_finish("Desktop bundle complete");
    Ok(())
}

fn resolve_theme_css(
    theme: Option<&str>,
    app_args: &AppArgs,
    app_dir: &Path,
) -> Result<Option<std::collections::HashMap<String, String>>> {
    let Some(theme) = theme else {
        return Ok(None);
    };
    let token_file = load_theme(theme, app_dir)?;
    let probe = webui::build(app_args.build_options(app_dir.to_path_buf()))
        .with_context(|| "Desktop theme probe build failed")?;
    let resolved = webui_tokens::resolve_tokens(&probe.protocol.tokens, &token_file)
        .with_context(|| "Desktop theme token resolution failed")?;
    for warning in &resolved.warnings {
        print_warning(warning);
    }
    Ok(Some(resolved.css))
}

fn load_theme(theme: &str, app_dir: &Path) -> Result<webui_tokens::TokenFile> {
    let resolved = webui_tokens::resolve_theme_path(theme, app_dir)
        .with_context(|| format!("Failed to resolve desktop theme: {theme}"))?;
    webui_tokens::load_token_file(&resolved)
        .with_context(|| format!("Failed to load desktop theme: {}", resolved.display()))
}

fn package_bundle(args: PackageArgs) -> Result<()> {
    let input = expand_path(&args.bundle, "bundle or app")?;
    if input.join("manifest.webui-desktop.json").is_file() {
        let bundle_dir = canonicalize_existing_dir(&args.bundle, "bundle")?;
        package_existing_bundle(args, bundle_dir)
    } else {
        let app_root = canonicalize_existing_dir(&args.bundle, "app")?;
        package_app_root(args, app_root)
    }
}

fn package_existing_bundle(args: PackageArgs, bundle_dir: PathBuf) -> Result<()> {
    let out_dir = expand_path(&args.out, "output")?;
    let targets = parse_package_targets(&args.target)?;
    let runner_exe = match args.runner.as_ref() {
        Some(runner) => optional_existing_file(Some(runner), "runner")?
            .ok_or_else(|| anyhow::anyhow!("runner path is required"))?,
        None => {
            std::env::current_exe().with_context(|| "Failed to locate webui-desktop executable")?
        }
    };

    print_header("WebUI Desktop Package");
    print_field("Bundle", &bundle_dir.display());
    print_field("Target", &args.target);
    print_field("Output", &out_dir.display());
    print_field("Runner", &runner_exe.display());

    for target in targets {
        let result = package_desktop_bundle(DesktopPackageOptions {
            bundle_dir: bundle_dir.clone(),
            out_dir: out_dir.clone(),
            target,
            runner_exe: runner_exe.clone(),
        })
        .with_context(|| "Desktop packaging failed")?;
        print_field("Package", &result.output_path.display());
    }
    print_finish("Desktop package complete");
    Ok(())
}

fn package_app_root(args: PackageArgs, app_root: PathBuf) -> Result<()> {
    let config = read_desktop_app_config(&app_root)?;
    if !args.no_web_build {
        run_web_build_scripts(&app_root, &config)?;
    }

    let temp = TempWorkDir::new("webui-desktop-package")?;
    let plan = create_app_package_plan(&args, app_root, config, &temp.path)?;
    let targets = parse_package_targets(&args.target)?;

    print_header("WebUI Desktop App Package");
    print_field("App", &plan.app_root.display());
    print_field("Source", &plan.source_dir.display());
    print_field("Bundle", &plan.bundle_dir.display());
    print_field("Runner", &plan.runner_exe.display());

    let manifest = build_desktop_bundle(DesktopBundleOptions {
        build_options: webui::BuildOptions {
            app_dir: plan.source_dir,
            entry: plan.entry,
            css: webui::CssStrategy::Link,
            dom: webui::DomStrategy::Shadow,
            plugin: plan.plugin,
            components: Vec::new(),
            component_asset_roots: Vec::new(),
            css_file_name_template: DEFAULT_CSS_FILE_NAME_TEMPLATE.to_string(),
            css_public_base: None,
            legal_comments: webui::LegalComments::Inline,
        },
        out_dir: plan.bundle_dir.clone(),
        state_file: plan.state_file,
        asset_root: plan.staged_assets,
        token_css: plan.token_css,
        app_id: plan.app_id,
        app_name: plan.app_name,
        version: plan.app_version,
        publisher: plan.publisher,
        window: plan.window,
        icon_file: plan.icon_file,
        shell: DesktopShellConfig::default(),
        package_targets: Vec::new(),
    })
    .with_context(|| "Desktop app bundle build failed")?;
    print_field("Protocol", &manifest.protocol_path.display());
    print_field("Assets", &manifest.integrity.assets.len());

    let out_dir = expand_path(&args.out, "output")?;
    for target in targets {
        let result = package_desktop_bundle(DesktopPackageOptions {
            bundle_dir: plan.bundle_dir.clone(),
            out_dir: out_dir.clone(),
            target,
            runner_exe: plan.runner_exe.clone(),
        })
        .with_context(|| "Desktop app packaging failed")?;
        print_field("Package", &result.output_path.display());
    }
    print_finish("Desktop app package complete");
    Ok(())
}

fn create_app_package_plan(
    args: &PackageArgs,
    app_root: PathBuf,
    config: DesktopAppPackageConfig,
    temp_root: &Path,
) -> Result<AppPackagePlan> {
    let source_dir = config_existing_dir(&app_root, config.source.as_ref(), "src", "app source")?;
    let state_file =
        config_optional_file(&app_root, config.state.as_ref(), "data/state.json", "state")?;
    let assets = config_optional_dir(&app_root, config.assets.as_ref(), "dist", "assets")?;
    let icon_file = config_optional_file(
        &app_root,
        args.icon.as_ref().or(config.icon.as_ref()),
        "desktop/icon.png",
        "icon",
    )?;
    let runner_exe = resolve_package_runner(args, &app_root, config.runner_crate.as_deref())?;
    let bundle_dir = match args.bundle_out.as_ref() {
        Some(path) => expand_path(path, "bundle output")?,
        None => temp_root.join("bundle"),
    };
    let entry = config.entry.unwrap_or_else(|| "index.html".to_string());
    let plugin = config.plugin;
    let generated_css = generated_css_names(&source_dir, &entry, plugin)?;
    let staged_assets = stage_app_assets(
        assets.as_ref(),
        &temp_root.join("assets"),
        generated_css.as_slice(),
    )?;
    let theme = args.theme.as_deref().or(config.theme.as_deref());
    let token_css = match theme {
        Some(theme) => Some(resolve_theme_for_source(
            theme,
            &source_dir,
            &app_root,
            &entry,
            plugin,
        )?),
        None => None,
    };
    let app_name = config
        .app_name
        .clone()
        .unwrap_or_else(|| title_from_path(&app_root));
    let app_id = config
        .app_id
        .clone()
        .unwrap_or_else(|| default_app_id(&app_name));

    Ok(AppPackagePlan {
        app_root,
        source_dir,
        entry,
        state_file,
        staged_assets,
        icon_file,
        bundle_dir,
        runner_exe,
        token_css,
        app_id,
        app_name: app_name.clone(),
        app_version: config.app_version.unwrap_or_else(|| "0.0.0".to_string()),
        publisher: config.publisher.unwrap_or_else(|| "Microsoft".to_string()),
        window: WindowOptions {
            title: config.title.unwrap_or(app_name),
            width: config.width.unwrap_or(1200),
            height: config.height.unwrap_or(800),
            maximized: false,
            devtools: config.devtools.unwrap_or(false),
        },
        plugin,
    })
}

fn read_desktop_app_config(app_root: &Path) -> Result<DesktopAppPackageConfig> {
    let package_path = app_root.join("package.json");
    if !package_path.is_file() {
        return Ok(DesktopAppPackageConfig::default());
    }
    let text = fs::read_to_string(&package_path)
        .with_context(|| format!("Failed to read {}", package_path.display()))?;
    let package: serde_json::Value = serde_json::from_str(&text)
        .with_context(|| format!("Failed to parse {}", package_path.display()))?;
    let mut config = DesktopAppPackageConfig {
        app_name: string_field(&package, "name").map(title_from_package_name),
        app_version: string_field(&package, "version"),
        ..DesktopAppPackageConfig::default()
    };

    let Some(desktop) = package
        .get("webuiDesktop")
        .and_then(serde_json::Value::as_object)
    else {
        config.build_scripts = default_build_scripts(&package);
        return Ok(config);
    };

    config.entry = string_field_in(desktop, "entry");
    config.source = path_field(desktop, "app").or_else(|| path_field(desktop, "source"));
    config.state = path_field(desktop, "state");
    config.assets = path_field(desktop, "assets");
    config.icon = path_field(desktop, "icon");
    config.theme = string_field_in(desktop, "theme");
    config.runner_crate = string_field_in(desktop, "runnerCrate");
    config.package_manager = string_field_in(desktop, "packageManager");
    config.build_scripts =
        string_array_field(desktop, "buildScripts").or_else(|| default_build_scripts(&package));
    config.app_id = string_field_in(desktop, "appId");
    config.app_name = string_field_in(desktop, "appName").or(config.app_name);
    config.app_version = string_field_in(desktop, "appVersion").or(config.app_version);
    config.publisher = string_field_in(desktop, "publisher");
    config.title = string_field_in(desktop, "title");
    config.width = u32_field(desktop, "width")?;
    config.height = u32_field(desktop, "height")?;
    config.devtools = desktop.get("devtools").and_then(serde_json::Value::as_bool);
    config.plugin = string_field_in(desktop, "plugin")
        .map(|raw| parse_plugin_value(&raw))
        .transpose()?;
    Ok(config)
}

fn run_web_build_scripts(app_root: &Path, config: &DesktopAppPackageConfig) -> Result<()> {
    let Some(scripts) = &config.build_scripts else {
        return Ok(());
    };
    let package_manager = config.package_manager.as_deref().unwrap_or("pnpm");
    for script in scripts {
        print_field("Script", script);
        let mut command = Command::new(package_manager);
        match package_manager {
            "npm" => {
                command.arg("--prefix").arg(app_root).arg("run").arg(script);
            }
            "yarn" | "bun" => {
                command.arg("--cwd").arg(app_root).arg("run").arg(script);
            }
            _ => {
                command.arg("--dir").arg(app_root).arg("run").arg(script);
            }
        }
        run_command(
            &mut command,
            &format!("desktop web build script '{script}'"),
        )?;
    }
    Ok(())
}

fn resolve_package_runner(
    args: &PackageArgs,
    app_root: &Path,
    configured_crate: Option<&str>,
) -> Result<PathBuf> {
    if let Some(runner) = args.runner.as_ref() {
        return optional_existing_file(Some(runner), "runner")?
            .ok_or_else(|| anyhow::anyhow!("runner path is required"));
    }
    let manifest = app_runner_manifest(app_root);
    let runner_crate = match args.runner_crate.as_deref().or(configured_crate) {
        Some(name) => name.to_string(),
        None => infer_runner_crate(manifest.as_deref().ok_or_else(|| {
            anyhow::anyhow!("failed to infer runner crate; pass --runner-crate")
        })?)?,
    };
    build_runner_crate(&runner_crate, args.release, manifest.as_deref())?;
    runner_executable_path(&runner_crate, args.release, manifest.as_deref())
}

fn app_runner_manifest(app_root: &Path) -> Option<PathBuf> {
    let manifest = app_root.join("desktop").join("Cargo.toml");
    manifest.is_file().then_some(manifest)
}

fn build_runner_crate(runner_crate: &str, release: bool, manifest: Option<&Path>) -> Result<()> {
    let mut command = Command::new("cargo");
    command.arg("build");
    if let Some(manifest) = manifest {
        command.arg("--manifest-path").arg(manifest);
    } else {
        command.arg("-p").arg(runner_crate);
    }
    if release {
        command.arg("--release");
    }
    run_command(&mut command, &format!("cargo build -p {runner_crate}"))
}

fn runner_executable_path(
    runner_crate: &str,
    release: bool,
    manifest: Option<&Path>,
) -> Result<PathBuf> {
    let mut path = cargo_target_directory(manifest)?;
    path.push(if release { "release" } else { "debug" });
    path.push(format!("{runner_crate}{}", std::env::consts::EXE_SUFFIX));
    if !path.is_file() {
        return Err(anyhow::anyhow!(
            "runner executable was not built: {}",
            path.display()
        ));
    }
    Ok(path)
}

fn cargo_target_directory(manifest: Option<&Path>) -> Result<PathBuf> {
    let mut command = Command::new("cargo");
    command.args(["metadata", "--no-deps", "--format-version", "1"]);
    if let Some(manifest) = manifest {
        command.arg("--manifest-path").arg(manifest);
    }
    let output = command
        .output()
        .with_context(|| "Failed to run cargo metadata")?;
    if !output.status.success() {
        return Err(anyhow::anyhow!(
            "cargo metadata exited with {}",
            output.status
        ));
    }
    let metadata: serde_json::Value = serde_json::from_slice(&output.stdout)
        .with_context(|| "Failed to parse cargo metadata output")?;
    metadata
        .get("target_directory")
        .and_then(serde_json::Value::as_str)
        .map(PathBuf::from)
        .ok_or_else(|| anyhow::anyhow!("cargo metadata did not include target_directory"))
}

fn infer_runner_crate(cargo_toml: &Path) -> Result<String> {
    let text = fs::read_to_string(cargo_toml)
        .with_context(|| format!("Failed to read {}", cargo_toml.display()))?;
    parse_cargo_package_name(&text).ok_or_else(|| {
        anyhow::anyhow!(
            "failed to infer runner crate from {}; pass --runner-crate",
            cargo_toml.display()
        )
    })
}

fn parse_cargo_package_name(text: &str) -> Option<String> {
    let mut in_package = false;
    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed == "[package]" {
            in_package = true;
            continue;
        }
        if in_package && trimmed.starts_with('[') {
            return None;
        }
        if in_package {
            let Some((key, value)) = trimmed.split_once('=') else {
                continue;
            };
            if key.trim() == "name" {
                return trim_toml_string(value.trim());
            }
        }
    }
    None
}

fn trim_toml_string(value: &str) -> Option<String> {
    let trimmed = value.trim();
    let quoted = trimmed.strip_prefix('"')?.strip_suffix('"')?;
    Some(quoted.to_string())
}

fn run_command(command: &mut Command, description: &str) -> Result<()> {
    let status = command
        .status()
        .with_context(|| format!("Failed to run {description}"))?;
    if status.success() {
        Ok(())
    } else {
        Err(anyhow::anyhow!("{description} exited with {status}"))
    }
}

fn config_existing_dir(
    app_root: &Path,
    configured: Option<&PathBuf>,
    default: &str,
    label: &str,
) -> Result<PathBuf> {
    let path = resolve_config_path(app_root, configured, default);
    canonicalize_existing_dir(&path, label)
}

fn config_optional_file(
    app_root: &Path,
    configured: Option<&PathBuf>,
    default: &str,
    label: &str,
) -> Result<Option<PathBuf>> {
    let path = resolve_config_path(app_root, configured, default);
    if configured.is_none() && !path.exists() {
        return Ok(None);
    }
    optional_existing_file(Some(&path), label)
}

fn config_optional_dir(
    app_root: &Path,
    configured: Option<&PathBuf>,
    default: &str,
    label: &str,
) -> Result<Option<PathBuf>> {
    let path = resolve_config_path(app_root, configured, default);
    if configured.is_none() && !path.exists() {
        return Ok(None);
    }
    optional_existing_dir(Some(&path), label)
}

fn resolve_config_path(app_root: &Path, configured: Option<&PathBuf>, default: &str) -> PathBuf {
    match configured {
        Some(path) if path.is_absolute() => path.clone(),
        Some(path) => app_root.join(path),
        None => app_root.join(default),
    }
}

fn generated_css_names(
    source_dir: &Path,
    entry: &str,
    plugin: Option<webui::Plugin>,
) -> Result<Vec<String>> {
    let result = webui::build(webui::BuildOptions {
        app_dir: source_dir.to_path_buf(),
        entry: entry.to_string(),
        css: webui::CssStrategy::Link,
        dom: webui::DomStrategy::Shadow,
        plugin,
        components: Vec::new(),
        component_asset_roots: Vec::new(),
        css_file_name_template: DEFAULT_CSS_FILE_NAME_TEMPLATE.to_string(),
        css_public_base: None,
        legal_comments: webui::LegalComments::Inline,
    })
    .with_context(|| "Desktop generated CSS discovery build failed")?;
    Ok(result.css_files.into_iter().map(|(name, _)| name).collect())
}

fn stage_app_assets(
    asset_root: Option<&PathBuf>,
    dest_root: &Path,
    generated_css: &[String],
) -> Result<Option<PathBuf>> {
    let Some(asset_root) = asset_root else {
        return Ok(None);
    };
    let mut copied = false;
    let mut stack = vec![asset_root.clone()];
    while let Some(dir) = stack.pop() {
        for entry in fs::read_dir(&dir)
            .with_context(|| format!("Failed to read asset directory {}", dir.display()))?
        {
            let entry = entry
                .with_context(|| format!("Failed to read asset entry in {}", dir.display()))?;
            let path = entry.path();
            let ty = entry
                .file_type()
                .with_context(|| format!("Failed to read asset type {}", path.display()))?;
            if ty.is_dir() {
                stack.push(path);
            } else if ty.is_file() {
                let relative = path.strip_prefix(asset_root).with_context(|| {
                    format!(
                        "Failed to compute asset path {} relative to {}",
                        path.display(),
                        asset_root.display()
                    )
                })?;
                if should_stage_asset(relative, generated_css) {
                    let dest = dest_root.join(relative);
                    if let Some(parent) = dest.parent() {
                        fs::create_dir_all(parent).with_context(|| {
                            format!(
                                "Failed to create staged asset directory {}",
                                parent.display()
                            )
                        })?;
                    }
                    fs::copy(&path, &dest).with_context(|| {
                        format!(
                            "Failed to stage desktop asset {} to {}",
                            path.display(),
                            dest.display()
                        )
                    })?;
                    copied = true;
                }
            }
        }
    }
    Ok(copied.then(|| dest_root.to_path_buf()))
}

fn should_stage_asset(relative: &Path, generated_css: &[String]) -> bool {
    let Some(name) = relative.file_name().and_then(|name| name.to_str()) else {
        return false;
    };
    if matches!(
        name,
        "protocol.bin"
            | "manifest.webui-desktop.json"
            | "state.json"
            | "index.html"
            | "webui-desktop-ipc.js"
    ) {
        return false;
    }
    let is_top_level = relative
        .parent()
        .is_none_or(|parent| parent.as_os_str().is_empty());
    !(is_top_level && generated_css.iter().any(|generated| generated == name))
}

fn resolve_theme_for_source(
    theme: &str,
    source_dir: &Path,
    app_root: &Path,
    entry: &str,
    plugin: Option<webui::Plugin>,
) -> Result<std::collections::HashMap<String, String>> {
    let token_file = load_theme(theme, app_root)?;
    let probe_options = webui::BuildOptions {
        app_dir: source_dir.to_path_buf(),
        entry: entry.to_string(),
        css: webui::CssStrategy::Link,
        dom: webui::DomStrategy::Shadow,
        plugin,
        components: Vec::new(),
        component_asset_roots: Vec::new(),
        css_file_name_template: DEFAULT_CSS_FILE_NAME_TEMPLATE.to_string(),
        css_public_base: None,
        legal_comments: webui::LegalComments::Inline,
    };
    let probe = webui::build(probe_options).with_context(|| "Desktop theme probe build failed")?;
    let resolved = webui_tokens::resolve_tokens(&probe.protocol.tokens, &token_file)
        .with_context(|| "Desktop theme token resolution failed")?;
    for warning in &resolved.warnings {
        print_warning(warning);
    }
    Ok(resolved.css)
}

fn parse_plugin_value(raw: &str) -> Result<webui::Plugin> {
    match raw {
        "fast" | "fast-v2" => Ok(webui::Plugin::FastV2),
        "fast-v3" => Ok(webui::Plugin::FastV3),
        "webui" => Ok(webui::Plugin::WebUI),
        other => Err(anyhow::anyhow!(
            "unknown desktop plugin '{other}'; expected fast, fast-v2, fast-v3, or webui"
        )),
    }
}

fn string_field(value: &serde_json::Value, key: &str) -> Option<String> {
    value
        .get(key)
        .and_then(serde_json::Value::as_str)
        .map(str::to_string)
}

fn string_field_in(map: &serde_json::Map<String, serde_json::Value>, key: &str) -> Option<String> {
    map.get(key)
        .and_then(serde_json::Value::as_str)
        .map(str::to_string)
}

fn path_field(map: &serde_json::Map<String, serde_json::Value>, key: &str) -> Option<PathBuf> {
    string_field_in(map, key).map(PathBuf::from)
}

fn string_array_field(
    map: &serde_json::Map<String, serde_json::Value>,
    key: &str,
) -> Option<Vec<String>> {
    map.get(key)
        .and_then(serde_json::Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(serde_json::Value::as_str)
                .map(str::to_string)
                .collect()
        })
}

fn u32_field(map: &serde_json::Map<String, serde_json::Value>, key: &str) -> Result<Option<u32>> {
    let Some(value) = map.get(key).and_then(serde_json::Value::as_u64) else {
        return Ok(None);
    };
    u32::try_from(value)
        .map(Some)
        .with_context(|| format!("desktop config field '{key}' is too large for u32"))
}

fn default_build_scripts(package: &serde_json::Value) -> Option<Vec<String>> {
    let scripts = package
        .get("scripts")
        .and_then(serde_json::Value::as_object)?;
    let mut out = Vec::new();
    for name in ["build:deps", "build:client"] {
        if scripts.contains_key(name) {
            out.push(name.to_string());
        }
    }
    (!out.is_empty()).then_some(out)
}

fn title_from_package_name(name: String) -> String {
    let mut title = String::with_capacity(name.len());
    let mut uppercase_next = true;
    for ch in name.chars() {
        if ch.is_ascii_alphanumeric() {
            if !title.is_empty() && uppercase_next {
                title.push(' ');
            }
            if uppercase_next {
                title.push(ch.to_ascii_uppercase());
                uppercase_next = false;
            } else {
                title.push(ch);
            }
        } else {
            uppercase_next = true;
        }
    }
    if title.is_empty() {
        "WebUI App".to_string()
    } else {
        title
    }
}

fn title_from_path(path: &Path) -> String {
    path.file_name()
        .and_then(|name| name.to_str())
        .map(|name| title_from_package_name(name.to_string()))
        .unwrap_or_else(|| "WebUI App".to_string())
}

fn default_app_id(app_name: &str) -> String {
    let mut suffix = String::with_capacity(app_name.len());
    for ch in app_name.chars() {
        if ch.is_ascii_alphanumeric() {
            suffix.push(ch.to_ascii_lowercase());
        }
    }
    if suffix.is_empty() {
        "com.microsoft.webui.app".to_string()
    } else {
        let mut out = String::with_capacity("com.microsoft.webui.".len() + suffix.len());
        out.push_str("com.microsoft.webui.");
        out.push_str(&suffix);
        out
    }
}

fn parse_package_targets(raw: &str) -> Result<Vec<DesktopPackageTarget>> {
    match raw {
        "all" => Ok(vec![
            DesktopPackageTarget::MacosApp,
            DesktopPackageTarget::WindowsPortable,
            DesktopPackageTarget::WindowsMsi,
            DesktopPackageTarget::WindowsMsix,
            DesktopPackageTarget::LinuxPortable,
            DesktopPackageTarget::LinuxAppImage,
            DesktopPackageTarget::LinuxDeb,
            DesktopPackageTarget::LinuxRpm,
        ]),
        "macos-app" => Ok(vec![DesktopPackageTarget::MacosApp]),
        "windows-portable" => Ok(vec![DesktopPackageTarget::WindowsPortable]),
        "windows-msi" => Ok(vec![DesktopPackageTarget::WindowsMsi]),
        "windows-msix" => Ok(vec![DesktopPackageTarget::WindowsMsix]),
        "linux-portable" => Ok(vec![DesktopPackageTarget::LinuxPortable]),
        "linux-appimage" => Ok(vec![DesktopPackageTarget::LinuxAppImage]),
        "linux-deb" => Ok(vec![DesktopPackageTarget::LinuxDeb]),
        "linux-rpm" => Ok(vec![DesktopPackageTarget::LinuxRpm]),
        other => Err(anyhow::anyhow!(
            "unknown desktop package target '{other}'; expected macos-app, windows-portable, windows-msi, windows-msix, linux-portable, linux-appimage, linux-deb, linux-rpm, or all"
        )),
    }
}

#[cfg(target_os = "linux")]
fn run_webview(runtime: Arc<DesktopRuntime>, window: &WindowArgs) -> Result<()> {
    webui_desktop_runner::run_runtime(runtime, window_options(window))
}

#[cfg(target_os = "windows")]
fn run_webview(runtime: Arc<DesktopRuntime>, window: &WindowArgs) -> Result<()> {
    webui_desktop_runner::run_runtime(runtime, window_options(window))
}

#[cfg(any(target_os = "linux", target_os = "windows"))]
fn window_options(window: &WindowArgs) -> WindowOptions {
    WindowOptions {
        title: window.title.clone(),
        width: window.width,
        height: window.height,
        maximized: false,
        devtools: window.devtools,
    }
}

fn canonicalize_existing_dir(path: &Path, label: &str) -> Result<PathBuf> {
    let expanded = expand_path(path, label)?;
    let canonical = expanded
        .canonicalize()
        .with_context(|| format!("Failed to resolve {label} path: {}", path.display()))?;
    if !canonical.is_dir() {
        return Err(anyhow::anyhow!(
            "{label} path must be a directory: {}",
            canonical.display()
        ));
    }
    Ok(canonical)
}

fn expand_path(path: &Path, label: &str) -> Result<PathBuf> {
    let expanded = expand_tilde(path)
        .with_context(|| format!("Failed to expand {label} path: {}", path.display()))?
        .into_owned();
    Ok(expanded)
}

fn optional_existing_dir(path: Option<&PathBuf>, label: &str) -> Result<Option<PathBuf>> {
    path.map(|path| canonicalize_existing_dir(path, label))
        .transpose()
}

fn optional_existing_file(path: Option<&PathBuf>, label: &str) -> Result<Option<PathBuf>> {
    let Some(path) = path else {
        return Ok(None);
    };
    let expanded = expand_tilde(path)
        .with_context(|| format!("Failed to expand {label} path: {}", path.display()))?
        .into_owned();
    let canonical = expanded
        .canonicalize()
        .with_context(|| format!("Failed to resolve {label} path: {}", path.display()))?;
    if !canonical.is_file() {
        return Err(anyhow::anyhow!(
            "{label} path must be a file: {}",
            canonical.display()
        ));
    }
    Ok(Some(canonical))
}

fn print_header(title: &str) {
    if is_json() {
        return;
    }
    eprintln!(
        "\n  {} {}\n",
        console::style("▸").cyan().bold(),
        console::style(title).cyan().bold()
    );
}

fn print_field(label: &str, value: &dyn std::fmt::Display) {
    if is_json() {
        return;
    }
    eprintln!(
        "  {} {}",
        console::style(format!("{label:<10}")).dim(),
        console::style(value).bold()
    );
}

fn print_warning(message: &dyn std::fmt::Display) {
    if is_json() {
        return;
    }
    eprintln!(
        "  {} {}",
        console::style("⚠").yellow(),
        console::style(message).dim()
    );
}

fn print_finish(message: &str) {
    if is_json() {
        return;
    }
    eprintln!("\n  {}", console::style(message).green());
}

fn print_error(err: &anyhow::Error) {
    if is_json() {
        println!("{}", error_json(err));
        return;
    }
    eprintln!(
        "\n  {} {}",
        console::style("✘").red().bold(),
        console::style(err).red().bold()
    );
    for cause in err.chain().skip(1) {
        eprintln!("  {} {cause}", console::style("caused by:").dim());
    }
}

fn error_json(err: &anyhow::Error) -> serde_json::Value {
    use serde_json::Value;

    let mut map = serde_json::Map::new();
    let chain: Vec<Value> = err
        .chain()
        .map(|cause| Value::from(cause.to_string()))
        .collect();
    map.insert("severity".to_string(), Value::from("error"));
    map.insert("code".to_string(), Value::from("desktop-error"));
    map.insert("message".to_string(), Value::from(err.to_string()));
    map.insert("file".to_string(), Value::Null);
    map.insert("line".to_string(), Value::Null);
    map.insert("column".to_string(), Value::Null);
    map.insert("snippet".to_string(), Value::Null);
    map.insert("help".to_string(), Value::Null);
    map.insert("chain".to_string(), Value::Array(chain));
    Value::Object(map)
}

#[cfg(test)]
#[allow(clippy::disallowed_methods)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn write_file(root: &Path, path: &str, content: &str) {
        let full = root.join(path);
        if let Some(parent) = full.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(full, content).unwrap();
    }

    #[test]
    fn reads_webui_desktop_package_config() {
        let dir = TempDir::new().unwrap();
        write_file(
            dir.path(),
            "package.json",
            r#"{
              "name": "contact-book-manager",
              "version": "1.0.0",
              "scripts": { "build:deps": "true", "build:client": "true" },
              "webuiDesktop": {
                "app": "src",
                "state": "data/state.json",
                "assets": "dist",
                "theme": "@microsoft/webui-examples-theme",
                "plugin": "webui",
                "runnerCrate": "contact-book-desktop",
                "buildScripts": ["build:deps", "build:client"],
                "appId": "com.microsoft.webui.contactbook",
                "appName": "Contact Book Manager",
                "appVersion": "1.0.0",
                "title": "Contact Book Manager",
                "width": 1200,
                "height": 800,
                "devtools": true
              }
            }"#,
        );

        let config = read_desktop_app_config(dir.path()).unwrap();

        assert_eq!(config.runner_crate.as_deref(), Some("contact-book-desktop"));
        assert_eq!(
            config.theme.as_deref(),
            Some("@microsoft/webui-examples-theme")
        );
        assert_eq!(
            config.app_id.as_deref(),
            Some("com.microsoft.webui.contactbook")
        );
        assert_eq!(config.width, Some(1200));
        assert_eq!(config.height, Some(800));
        assert_eq!(config.devtools, Some(true));
        assert_eq!(
            config.build_scripts.as_deref(),
            Some(["build:deps".to_string(), "build:client".to_string()].as_slice())
        );
        assert!(matches!(config.plugin, Some(webui::Plugin::WebUI)));
    }

    #[test]
    fn infers_runner_crate_from_desktop_cargo_toml() {
        let dir = TempDir::new().unwrap();
        write_file(
            dir.path(),
            "desktop/Cargo.toml",
            r#"[package]
name = "contact-book-desktop"
version = "0.1.0"
"#,
        );

        let name = infer_runner_crate(&dir.path().join("desktop/Cargo.toml")).unwrap();

        assert_eq!(name, "contact-book-desktop");
    }

    #[test]
    fn stages_static_css_but_skips_generated_top_level_css() {
        let dir = TempDir::new().unwrap();
        let assets = dir.path().join("dist");
        let staged = dir.path().join("staged");
        write_file(&assets, "my-card.css", "generated");
        write_file(&assets, "global.css", "static");
        write_file(&assets, "nested/my-card.css", "static nested");
        write_file(&assets, "app.js", "console.log('ok');");

        let root = stage_app_assets(Some(&assets), &staged, &["my-card.css".to_string()])
            .unwrap()
            .unwrap();

        assert_eq!(root, staged);
        assert!(!root.join("my-card.css").exists());
        assert!(root.join("global.css").is_file());
        assert!(root.join("nested/my-card.css").is_file());
        assert!(root.join("app.js").is_file());
    }
}

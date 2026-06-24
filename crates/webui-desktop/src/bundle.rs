// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

use std::collections::HashSet;
use std::ffi::OsString;
use std::fs;
use std::io::Read;
use std::path::{Component, Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use webui::RenderOptions;
use webui_handler::plugin::fast_v2::FastV2HydrationPlugin;
use webui_handler::plugin::fast_v3::FastV3HydrationPlugin;
use webui_handler::plugin::webui::WebUIHydrationPlugin;
use webui_handler::ResponseWriter;

use crate::error::{DesktopError, Result};

/// Desktop window defaults stored in a bundle manifest.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct WindowOptions {
    /// Window title.
    pub title: String,
    /// Initial window width in physical-independent pixels.
    pub width: u32,
    /// Initial window height in physical-independent pixels.
    pub height: u32,
    /// Whether to start maximized.
    pub maximized: bool,
    /// Whether to enable web inspector/devtools for development builds.
    pub devtools: bool,
}

impl Default for WindowOptions {
    fn default() -> Self {
        Self {
            title: "WebUI".to_string(),
            width: 1200,
            height: 800,
            maximized: false,
            devtools: false,
        }
    }
}

/// Runtime-neutral shell extension points for native desktop hosts.
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct DesktopShellConfig {
    /// Optional app icon path relative to the bundle root.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub icon_path: Option<PathBuf>,
    /// Native menu declarations. Empty means platform default menu.
    #[serde(default)]
    pub menus: Vec<DesktopMenu>,
    /// Windows jump-list declarations. Ignored on platforms that do not support them.
    #[serde(default)]
    pub jump_list: Vec<DesktopJumpListItem>,
    /// Whether popup/popover child windows are allowed.
    #[serde(default)]
    pub popovers: DesktopPopoverPolicy,
    /// File download policy for webview downloads.
    #[serde(default)]
    pub downloads: DesktopDownloadPolicy,
}

/// Native menu descriptor.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct DesktopMenu {
    /// Stable menu identifier.
    pub id: String,
    /// Visible menu label.
    pub label: String,
    /// Child menu items.
    #[serde(default)]
    pub items: Vec<DesktopMenuItem>,
}

/// Native menu item descriptor.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct DesktopMenuItem {
    /// Stable item identifier.
    pub id: String,
    /// Visible item label.
    pub label: String,
    /// Optional desktop IPC command invoked by this menu item.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub command: Option<String>,
    /// Optional keyboard accelerator in platform-neutral text form.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub accelerator: Option<String>,
}

/// Windows jump-list item descriptor.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct DesktopJumpListItem {
    /// Stable item identifier.
    pub id: String,
    /// Visible item label.
    pub label: String,
    /// App route or external URL.
    pub target: String,
}

/// Popup/popover child window policy.
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct DesktopPopoverPolicy {
    /// Whether popover child windows are enabled.
    pub enabled: bool,
    /// Maximum simultaneously open popovers.
    #[serde(default)]
    pub max_open: u8,
}

/// Webview download handling policy.
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct DesktopDownloadPolicy {
    /// Whether downloads are enabled.
    pub enabled: bool,
    /// Optional IPC command that receives download requests.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub command: Option<String>,
}

/// Native package target supported by `webui desktop package`.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum DesktopPackageTarget {
    /// macOS `.app` bundle.
    MacosApp,
    /// Windows portable directory or zip.
    WindowsPortable,
    /// Windows MSI installer.
    WindowsMsi,
    /// Windows MSIX package.
    WindowsMsix,
    /// Linux portable directory or tar.gz.
    LinuxPortable,
    /// Linux AppImage.
    LinuxAppImage,
    /// Linux Debian package.
    LinuxDeb,
    /// Linux RPM package.
    LinuxRpm,
}

/// One asset included in a desktop bundle.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct BundleAsset {
    /// Path relative to the bundle resource root.
    pub path: String,
    /// SHA-256 digest encoded as lowercase hexadecimal.
    pub sha256: String,
    /// Asset size in bytes.
    pub size_bytes: u64,
}

/// Integrity metadata for immutable bundle contents.
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct BundleIntegrity {
    /// SHA-256 digest of `protocol.bin` encoded as lowercase hexadecimal.
    pub protocol_sha256: String,
    /// Packaged asset hashes.
    pub assets: Vec<BundleAsset>,
}

/// Manifest generated by `webui desktop build` and consumed by packaging.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct DesktopBundleManifest {
    /// Manifest schema version.
    pub manifest_version: u32,
    /// Reverse-DNS application identifier.
    pub app_id: String,
    /// Human-readable app name.
    pub app_name: String,
    /// Application version.
    pub version: String,
    /// Publisher name.
    pub publisher: String,
    /// Entry fragment name used for startup render.
    pub entry: String,
    /// Handler plugin used by the desktop host.
    #[serde(default)]
    pub plugin: Option<String>,
    /// Protocol file path relative to the bundle root.
    pub protocol_path: PathBuf,
    /// Optional startup state path relative to the bundle root.
    pub state_path: Option<PathBuf>,
    /// Asset directory path relative to the bundle root.
    pub assets_dir: PathBuf,
    /// Optional IPC schema path relative to the bundle root.
    pub ipc_schema: Option<PathBuf>,
    /// Window defaults.
    pub window: WindowOptions,
    /// Native shell extension configuration.
    #[serde(default)]
    pub shell: DesktopShellConfig,
    /// Package targets requested for this bundle.
    pub package_targets: Vec<DesktopPackageTarget>,
    /// Integrity hashes for immutable bundle contents.
    pub integrity: BundleIntegrity,
}

impl DesktopBundleManifest {
    /// Current desktop bundle manifest version.
    pub const VERSION: u32 = 1;

    /// Load a desktop bundle manifest from disk.
    ///
    /// # Errors
    ///
    /// Returns [`DesktopError`] when the file cannot be read or parsed.
    pub fn load(path: &Path) -> Result<Self> {
        let bytes = fs::read(path).map_err(|source| DesktopError::Io {
            context: format!("reading desktop manifest {}", path.display()),
            source,
        })?;
        serde_json::from_slice(&bytes).map_err(|source| DesktopError::ManifestDeserialization {
            path: path.to_path_buf(),
            source,
        })
    }
}

/// Inputs for generating an immutable desktop bundle.
pub struct DesktopBundleOptions {
    /// WebUI build options.
    pub build_options: webui::BuildOptions,
    /// Output bundle directory.
    pub out_dir: PathBuf,
    /// Optional startup state JSON file.
    pub state_file: Option<PathBuf>,
    /// Optional static asset root copied into the bundle.
    pub asset_root: Option<PathBuf>,
    /// Optional resolved token CSS keyed by theme name.
    pub token_css: Option<std::collections::HashMap<String, String>>,
    /// Reverse-DNS application identifier.
    pub app_id: String,
    /// Human-readable app name.
    pub app_name: String,
    /// Application version.
    pub version: String,
    /// Publisher name.
    pub publisher: String,
    /// Window defaults.
    pub window: WindowOptions,
    /// Optional app icon file copied into the bundle.
    pub icon_file: Option<PathBuf>,
    /// Native shell extension configuration.
    pub shell: DesktopShellConfig,
    /// Package targets requested for this bundle.
    pub package_targets: Vec<DesktopPackageTarget>,
}

/// Build an immutable desktop bundle on disk.
///
/// # Errors
///
/// Returns [`DesktopError`] if WebUI build fails, bundle files cannot be
/// written, static assets escape the asset root, or manifest serialization
/// fails.
pub fn build_desktop_bundle(options: DesktopBundleOptions) -> Result<DesktopBundleManifest> {
    validate_bundle_output(&options)?;
    prepare_out_dir(&options.out_dir)?;

    let build_options = options.build_options.clone();
    let build_result = webui::build(build_options.clone())?;
    let protocol_path = PathBuf::from("protocol.bin");
    let protocol_dest = options.out_dir.join(&protocol_path);
    fs::write(&protocol_dest, &build_result.protocol_bytes).map_err(|source| DesktopError::Io {
        context: format!("writing desktop protocol {}", protocol_dest.display()),
        source,
    })?;

    let assets_dir = PathBuf::from("assets");
    let assets_dest = options.out_dir.join(&assets_dir);
    fs::create_dir_all(&assets_dest).map_err(|source| DesktopError::Io {
        context: format!(
            "creating desktop assets directory {}",
            assets_dest.display()
        ),
        source,
    })?;

    let mut claimed_assets = HashSet::new();
    let mut assets =
        write_generated_css(&assets_dest, &mut claimed_assets, &build_result.css_files)?;
    write_ipc_client(&assets_dest, &mut claimed_assets, &mut assets)?;
    write_startup_html(StartupHtmlInput {
        assets_dest: &assets_dest,
        claimed_assets: &mut claimed_assets,
        assets: &mut assets,
        protocol: &build_result.protocol,
        plugin: build_options.plugin,
        entry: &build_options.entry,
        state_file: options.state_file.as_ref(),
        token_css: options.token_css.as_ref(),
    })?;
    if let Some(asset_root) = &options.asset_root {
        copy_static_assets(asset_root, &assets_dest, &mut claimed_assets, &mut assets)?;
    }
    let mut shell = options.shell;
    if let Some(icon_file) = &options.icon_file {
        shell.icon_path = Some(copy_app_icon(
            icon_file,
            &assets_dest,
            &mut claimed_assets,
            &mut assets,
        )?);
    }

    let state_path = match &options.state_file {
        Some(state_file) => {
            let state_dest_name = PathBuf::from("state.json");
            let state_dest = options.out_dir.join(&state_dest_name);
            write_bundle_state(state_file, options.token_css.as_ref(), &state_dest)?;
            Some(state_dest_name)
        }
        None => match options.token_css.as_ref() {
            Some(token_css) => {
                let state_dest_name = PathBuf::from("state.json");
                let state_dest = options.out_dir.join(&state_dest_name);
                let mut state = Value::Object(serde_json::Map::new());
                webui_tokens::inject_token_css(&mut state, token_css);
                write_state_value(&state, &state_dest)?;
                Some(state_dest_name)
            }
            None => None,
        },
    };

    let manifest = DesktopBundleManifest {
        manifest_version: DesktopBundleManifest::VERSION,
        app_id: options.app_id,
        app_name: options.app_name,
        version: options.version,
        publisher: options.publisher,
        entry: build_options.entry,
        plugin: build_options.plugin.map(plugin_name).map(str::to_string),
        protocol_path,
        state_path,
        assets_dir,
        ipc_schema: None,
        window: options.window,
        shell,
        package_targets: options.package_targets,
        integrity: BundleIntegrity {
            protocol_sha256: sha256_file(&protocol_dest)?,
            assets,
        },
    };

    let manifest_path = options.out_dir.join("manifest.webui-desktop.json");
    let manifest_bytes =
        serde_json::to_vec_pretty(&manifest).map_err(DesktopError::ManifestSerialization)?;
    fs::write(&manifest_path, manifest_bytes).map_err(|source| DesktopError::Io {
        context: format!("writing desktop manifest {}", manifest_path.display()),
        source,
    })?;

    Ok(manifest)
}

fn validate_bundle_output(options: &DesktopBundleOptions) -> Result<()> {
    let output = normalized_absolute_path(&options.out_dir)?;
    let lexical_output = lexical_absolute_path(&options.out_dir)?;
    let app_dir = normalized_absolute_path(&options.build_options.app_dir)?;
    let lexical_app_dir = lexical_absolute_path(&options.build_options.app_dir)?;
    reject_path_overlap(&output, &app_dir, "app")?;
    reject_path_overlap(&lexical_output, &app_dir, "app")?;
    reject_path_overlap(&output, &lexical_app_dir, "app")?;
    reject_path_overlap(&lexical_output, &lexical_app_dir, "app")?;
    if let Some(state_file) = &options.state_file {
        let state = normalized_absolute_path(state_file)?;
        let lexical_state = lexical_absolute_path(state_file)?;
        reject_path_overlap(&output, &state, "state")?;
        reject_path_overlap(&lexical_output, &state, "state")?;
        reject_path_overlap(&output, &lexical_state, "state")?;
        reject_path_overlap(&lexical_output, &lexical_state, "state")?;
    }
    if let Some(asset_root) = &options.asset_root {
        let assets = normalized_absolute_path(asset_root)?;
        let lexical_assets = lexical_absolute_path(asset_root)?;
        reject_path_overlap(&output, &assets, "asset")?;
        reject_path_overlap(&lexical_output, &assets, "asset")?;
        reject_path_overlap(&output, &lexical_assets, "asset")?;
        reject_path_overlap(&lexical_output, &lexical_assets, "asset")?;
    }
    Ok(())
}

fn plugin_name(plugin: webui::Plugin) -> &'static str {
    match plugin {
        webui::Plugin::Fast | webui::Plugin::FastV2 => "fast",
        webui::Plugin::FastV3 => "fast-v3",
        webui::Plugin::WebUI => "webui",
    }
}

fn prepare_out_dir(out_dir: &Path) -> Result<()> {
    if out_dir.exists() {
        fs::remove_dir_all(out_dir).map_err(|source| DesktopError::Io {
            context: format!("cleaning desktop bundle directory {}", out_dir.display()),
            source,
        })?;
    }
    fs::create_dir_all(out_dir).map_err(|source| DesktopError::Io {
        context: format!("creating desktop bundle directory {}", out_dir.display()),
        source,
    })
}

pub(crate) fn normalized_absolute_path(path: &Path) -> Result<PathBuf> {
    let absolute = lexical_absolute_path(path)?;
    let base = resolve_existing_ancestor(&absolute)?;
    Ok(normalize_components(&base))
}

pub(crate) fn lexical_absolute_path(path: &Path) -> Result<PathBuf> {
    let absolute = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()
            .map_err(|source| DesktopError::Io {
                context: "resolving current directory for desktop output validation".to_string(),
                source,
            })?
            .join(path)
    };
    Ok(normalize_components(&absolute))
}

fn normalize_components(path: &Path) -> PathBuf {
    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                normalized.pop();
            }
            Component::Prefix(_) | Component::RootDir | Component::Normal(_) => {
                normalized.push(component.as_os_str());
            }
        }
    }
    normalized
}

fn resolve_existing_ancestor(path: &Path) -> Result<PathBuf> {
    if path.exists() {
        return path.canonicalize().map_err(|source| DesktopError::Io {
            context: format!("resolving desktop path {}", path.display()),
            source,
        });
    }

    let mut missing = Vec::<OsString>::new();
    let mut cursor = path;
    while !cursor.exists() {
        if let Some(name) = cursor.file_name() {
            missing.push(name.to_os_string());
        }
        let Some(parent) = cursor.parent() else {
            break;
        };
        cursor = parent;
    }

    let mut resolved = cursor.canonicalize().map_err(|source| DesktopError::Io {
        context: format!("resolving desktop path ancestor {}", cursor.display()),
        source,
    })?;
    for component in missing.iter().rev() {
        resolved.push(component);
    }
    Ok(resolved)
}

pub(crate) fn reject_path_overlap(
    output: &Path,
    input: &Path,
    input_label: &'static str,
) -> Result<()> {
    if output == input || output.starts_with(input) || input.starts_with(output) {
        return Err(DesktopError::OutputPathOverlap {
            output: output.to_path_buf(),
            input: input.to_path_buf(),
            input_label,
        });
    }
    Ok(())
}

fn write_generated_css(
    assets_dest: &Path,
    claimed_assets: &mut HashSet<String>,
    css_files: &[(String, String)],
) -> Result<Vec<BundleAsset>> {
    let mut assets = Vec::with_capacity(css_files.len());
    for (name, content) in css_files {
        claim_asset(claimed_assets, name)?;
        let path = assets_dest.join(name);
        fs::write(&path, content).map_err(|source| DesktopError::Io {
            context: format!("writing generated desktop CSS {}", path.display()),
            source,
        })?;
        assets.push(asset_record(name, &path)?);
    }
    Ok(assets)
}

fn write_ipc_client(
    assets_dest: &Path,
    claimed_assets: &mut HashSet<String>,
    assets: &mut Vec<BundleAsset>,
) -> Result<()> {
    const IPC_CLIENT_NAME: &str = "webui-desktop-ipc.js";
    claim_asset(claimed_assets, IPC_CLIENT_NAME)?;
    let path = assets_dest.join(IPC_CLIENT_NAME);
    fs::write(&path, IPC_CLIENT_JS).map_err(|source| DesktopError::Io {
        context: format!("writing desktop IPC client {}", path.display()),
        source,
    })?;
    assets.push(asset_record(IPC_CLIENT_NAME, &path)?);
    Ok(())
}

fn copy_app_icon(
    icon_file: &Path,
    assets_dest: &Path,
    claimed_assets: &mut HashSet<String>,
    assets: &mut Vec<BundleAsset>,
) -> Result<PathBuf> {
    let extension = icon_file
        .extension()
        .and_then(|value| value.to_str())
        .filter(|value| !value.is_empty())
        .unwrap_or("icon");
    let mut relative = String::with_capacity("app-icon.".len() + extension.len());
    relative.push_str("app-icon.");
    relative.push_str(extension);
    claim_asset(claimed_assets, &relative)?;
    let dest = assets_dest.join(&relative);
    copy_file_to(icon_file, &dest)?;
    assets.push(asset_record(&relative, &dest)?);
    Ok(PathBuf::from("assets").join(relative))
}

struct StartupHtmlInput<'a> {
    assets_dest: &'a Path,
    claimed_assets: &'a mut HashSet<String>,
    assets: &'a mut Vec<BundleAsset>,
    protocol: &'a webui_protocol::WebUIProtocol,
    plugin: Option<webui::Plugin>,
    entry: &'a str,
    state_file: Option<&'a PathBuf>,
    token_css: Option<&'a std::collections::HashMap<String, String>>,
}

fn write_startup_html(input: StartupHtmlInput<'_>) -> Result<()> {
    const STARTUP_HTML_NAME: &str = "index.html";
    claim_asset(input.claimed_assets, STARTUP_HTML_NAME)?;
    let mut state = read_bundle_state(input.state_file)?;
    if let Some(token_css) = input.token_css {
        webui_tokens::inject_token_css(&mut state, token_css);
    }
    if let Value::Object(map) = &mut state {
        map.insert("basePath".to_string(), Value::String("/".to_string()));
    }
    let handler = create_handler(input.plugin);
    let mut writer = MemoryWriter::with_capacity(4096);
    handler.handle(
        input.protocol,
        &state,
        &RenderOptions::new(input.entry, "/"),
        &mut writer,
    )?;
    let path = input.assets_dest.join(STARTUP_HTML_NAME);
    fs::write(&path, writer.buf).map_err(|source| DesktopError::Io {
        context: format!("writing desktop startup HTML {}", path.display()),
        source,
    })?;
    input.assets.push(asset_record(STARTUP_HTML_NAME, &path)?);
    Ok(())
}

fn read_bundle_state(path: Option<&PathBuf>) -> Result<Value> {
    let Some(path) = path else {
        return Ok(Value::Object(serde_json::Map::new()));
    };
    let json = fs::read_to_string(path).map_err(|source| DesktopError::Io {
        context: format!("reading desktop state {}", path.display()),
        source,
    })?;
    serde_json::from_str(&json).map_err(|source| DesktopError::StateJson {
        path: path.clone(),
        source,
    })
}

fn write_bundle_state(
    state_file: &Path,
    token_css: Option<&std::collections::HashMap<String, String>>,
    state_dest: &Path,
) -> Result<()> {
    if token_css.is_none() {
        return copy_file(state_file, state_dest);
    }
    let mut state = read_bundle_state(Some(&state_file.to_path_buf()))?;
    if let Some(token_css) = token_css {
        webui_tokens::inject_token_css(&mut state, token_css);
    }
    write_state_value(&state, state_dest)
}

fn write_state_value(state: &Value, state_dest: &Path) -> Result<()> {
    let bytes = serde_json::to_vec_pretty(state).map_err(|source| DesktopError::Serialization {
        context: "serializing desktop bundle state".to_string(),
        source,
    })?;
    fs::write(state_dest, bytes).map_err(|source| DesktopError::Io {
        context: format!("writing desktop bundle state {}", state_dest.display()),
        source,
    })
}

fn create_handler(plugin: Option<webui::Plugin>) -> webui::WebUIHandler {
    match plugin {
        Some(webui::Plugin::Fast | webui::Plugin::FastV2) => {
            webui::WebUIHandler::with_plugin(|| Box::new(FastV2HydrationPlugin::new()))
        }
        Some(webui::Plugin::FastV3) => {
            webui::WebUIHandler::with_plugin(|| Box::new(FastV3HydrationPlugin::new()))
        }
        Some(webui::Plugin::WebUI) => {
            webui::WebUIHandler::with_plugin(|| Box::new(WebUIHydrationPlugin::new()))
        }
        None => webui::WebUIHandler::new(),
    }
}

struct MemoryWriter {
    buf: String,
}

impl MemoryWriter {
    fn with_capacity(capacity: usize) -> Self {
        Self {
            buf: String::with_capacity(capacity),
        }
    }
}

impl ResponseWriter for MemoryWriter {
    fn write(&mut self, content: &str) -> webui_handler::Result<()> {
        self.buf.push_str(content);
        Ok(())
    }

    fn end(&mut self) -> webui_handler::Result<()> {
        Ok(())
    }
}

fn copy_static_assets(
    asset_root: &Path,
    assets_dest: &Path,
    claimed_assets: &mut HashSet<String>,
    assets: &mut Vec<BundleAsset>,
) -> Result<()> {
    let canonical_root = asset_root
        .canonicalize()
        .map_err(|source| DesktopError::Io {
            context: format!("resolving desktop static assets {}", asset_root.display()),
            source,
        })?;

    let mut stack = vec![canonical_root.clone()];
    while let Some(dir) = stack.pop() {
        for entry in fs::read_dir(&dir).map_err(|source| DesktopError::Io {
            context: format!("reading desktop static assets {}", dir.display()),
            source,
        })? {
            let entry = entry.map_err(|source| DesktopError::Io {
                context: format!("reading desktop static asset entry in {}", dir.display()),
                source,
            })?;
            let path = entry.path();
            let file_type = entry.file_type().map_err(|source| DesktopError::Io {
                context: format!("reading desktop static asset type {}", path.display()),
                source,
            })?;
            if file_type.is_dir() {
                stack.push(path);
                continue;
            }
            if !file_type.is_file() {
                continue;
            }

            let canonical = path.canonicalize().map_err(|source| DesktopError::Io {
                context: format!("resolving desktop static asset {}", path.display()),
                source,
            })?;
            if !canonical.starts_with(&canonical_root) {
                return Err(DesktopError::InvalidAssetPath {
                    path: canonical.display().to_string(),
                });
            }
            let relative = canonical.strip_prefix(&canonical_root).map_err(|_| {
                DesktopError::InvalidAssetPath {
                    path: canonical.display().to_string(),
                }
            })?;
            let relative_string = relative.to_string_lossy().replace('\\', "/");
            claim_asset(claimed_assets, &relative_string)?;
            let dest = assets_dest.join(relative);
            copy_file(&canonical, &dest)?;
            assets.push(asset_record(&relative_string, &dest)?);
        }
    }
    Ok(())
}

fn claim_asset(claimed_assets: &mut HashSet<String>, path: &str) -> Result<()> {
    if claimed_assets.insert(path.to_string()) {
        return Ok(());
    }
    Err(DesktopError::BundleAssetCollision {
        path: path.to_string(),
    })
}

fn copy_file(source: &Path, dest: &Path) -> Result<()> {
    copy_file_to(source, dest)
}

pub(crate) fn copy_file_to(source: &Path, dest: &Path) -> Result<()> {
    if let Some(parent) = dest.parent() {
        fs::create_dir_all(parent).map_err(|source| DesktopError::Io {
            context: format!("creating desktop bundle directory {}", parent.display()),
            source,
        })?;
    }
    fs::copy(source, dest).map_err(|source| DesktopError::Io {
        context: format!("copying desktop bundle file to {}", dest.display()),
        source,
    })?;
    Ok(())
}

fn asset_record(relative: &str, path: &Path) -> Result<BundleAsset> {
    let size_bytes = fs::metadata(path)
        .map_err(|source| DesktopError::Io {
            context: format!("reading desktop bundle asset metadata {}", path.display()),
            source,
        })?
        .len();
    let mut bundle_path = String::with_capacity("assets/".len() + relative.len());
    bundle_path.push_str("assets/");
    bundle_path.push_str(relative);
    Ok(BundleAsset {
        path: bundle_path,
        sha256: sha256_file(path)?,
        size_bytes,
    })
}

fn sha256_file(path: &Path) -> Result<String> {
    let mut file = fs::File::open(path).map_err(|source| DesktopError::Io {
        context: format!("hashing desktop bundle file {}", path.display()),
        source,
    })?;
    let mut hasher = Sha256::new();
    let mut buffer = [0_u8; 16 * 1024];
    loop {
        let read = file.read(&mut buffer).map_err(|source| DesktopError::Io {
            context: format!("hashing desktop bundle file {}", path.display()),
            source,
        })?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    let digest = hasher.finalize();
    Ok(hex_lower(&digest))
}

fn hex_lower(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push(char::from(HEX[usize::from(byte >> 4)]));
        out.push(char::from(HEX[usize::from(byte & 0x0f)]));
    }
    out
}

const IPC_CLIENT_JS: &str = r#"// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

const IPC_ENDPOINT = "/_webui/ipc";
const IPC_VERSION = 1;
const textEncoder = new TextEncoder();
const textDecoder = new TextDecoder();

function varint(value) {
  let n = BigInt(value);
  const out = [];
  while (n >= 0x80n) {
    out.push(Number((n & 0x7fn) | 0x80n));
    n >>= 7n;
  }
  out.push(Number(n));
  return out;
}

function bytesField(tag, bytes) {
  return [tag, ...varint(bytes.length), ...bytes];
}

function concat(parts) {
  let size = 0;
  for (let i = 0; i < parts.length; i++) {
    size += parts[i].length;
  }
  const out = new Uint8Array(size);
  let offset = 0;
  for (let i = 0; i < parts.length; i++) {
    out.set(parts[i], offset);
    offset += parts[i].length;
  }
  return out;
}

function readVarint(bytes, cursor) {
  let shift = 0n;
  let value = 0n;
  while (cursor.offset < bytes.length) {
    const byte = bytes[cursor.offset++];
    value |= BigInt(byte & 0x7f) << shift;
    if ((byte & 0x80) === 0) {
      return value;
    }
    shift += 7n;
  }
  throw new Error("invalid desktop IPC varint");
}

function readBytes(bytes, cursor) {
  const len = Number(readVarint(bytes, cursor));
  const end = cursor.offset + len;
  if (end > bytes.length) {
    throw new Error("invalid desktop IPC length");
  }
  const value = bytes.subarray(cursor.offset, end);
  cursor.offset = end;
  return value;
}

function skipField(bytes, cursor, wireType) {
  if (wireType === 0) {
    readVarint(bytes, cursor);
    return;
  }
  if (wireType === 2) {
    readBytes(bytes, cursor);
    return;
  }
  throw new Error(`unsupported desktop IPC wire type ${wireType}`);
}

function decodeError(bytes) {
  const cursor = { offset: 0 };
  const error = { code: "", message: "", help: "" };
  while (cursor.offset < bytes.length) {
    const tag = Number(readVarint(bytes, cursor));
    const field = tag >> 3;
    const wireType = tag & 7;
    if (wireType === 2 && field === 1) {
      error.code = textDecoder.decode(readBytes(bytes, cursor));
    } else if (wireType === 2 && field === 2) {
      error.message = textDecoder.decode(readBytes(bytes, cursor));
    } else if (wireType === 2 && field === 3) {
      error.help = textDecoder.decode(readBytes(bytes, cursor));
    } else {
      skipField(bytes, cursor, wireType);
    }
  }
  return error;
}

function decodeResponse(bytes) {
  const cursor = { offset: 0 };
  const response = { version: 0, requestId: 0, payload: null, error: null };
  while (cursor.offset < bytes.length) {
    const tag = Number(readVarint(bytes, cursor));
    const field = tag >> 3;
    const wireType = tag & 7;
    if (wireType === 0 && field === 1) {
      response.version = Number(readVarint(bytes, cursor));
    } else if (wireType === 0 && field === 2) {
      response.requestId = Number(readVarint(bytes, cursor));
    } else if (wireType === 2 && field === 3) {
      response.payload = readBytes(bytes, cursor);
    } else if (wireType === 2 && field === 4) {
      response.error = decodeError(readBytes(bytes, cursor));
    } else {
      skipField(bytes, cursor, wireType);
    }
  }
  return response;
}

function encodeRequest(requestId, method, payload) {
  const methodBytes = textEncoder.encode(method);
  const payloadBytes = payload instanceof Uint8Array ? payload : new Uint8Array(payload);
  return concat([
    Uint8Array.from([8, ...varint(IPC_VERSION)]),
    Uint8Array.from([16, ...varint(requestId)]),
    Uint8Array.from(bytesField(26, methodBytes)),
    Uint8Array.from(bytesField(34, payloadBytes)),
  ]);
}

let nextRequestId = 1;

export async function invokeDesktop(method, payload = new Uint8Array()) {
  const requestId = nextRequestId++;
  const frame = encodeRequest(requestId, method, payload);
  const response = await fetch(IPC_ENDPOINT, {
    method: "POST",
    headers: { "Content-Type": "application/x-protobuf" },
    body: frame,
  });
  const bytes = new Uint8Array(await response.arrayBuffer());
  const decoded = decodeResponse(bytes);
  if (decoded.error) {
    const err = new Error(decoded.error.message);
    err.code = decoded.error.code;
    err.help = decoded.error.help;
    throw err;
  }
  return decoded.payload || new Uint8Array();
}
"#;

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

    fn build_options(app_dir: PathBuf) -> webui::BuildOptions {
        webui::BuildOptions {
            app_dir,
            entry: "index.html".to_string(),
            ..webui::BuildOptions::default()
        }
    }

    #[test]
    fn builds_manifest_with_protocol_state_and_assets() {
        let app = TempDir::new().unwrap();
        let out = TempDir::new().unwrap();
        write_file(
            app.path(),
            "index.html",
            "<my-card></my-card><main>Hello {{name}}</main>",
        );
        write_file(app.path(), "my-card.html", "<p class=\"card\">Card</p>");
        write_file(app.path(), "my-card.css", ".card { color: red; }");
        write_file(app.path(), "state.json", r#"{"name":"Bundle"}"#);
        write_file(app.path(), "public/app.js", "console.log('bundle');");

        let manifest = build_desktop_bundle(DesktopBundleOptions {
            build_options: build_options(app.path().to_path_buf()),
            out_dir: out.path().join("desktop"),
            state_file: Some(app.path().join("state.json")),
            asset_root: Some(app.path().join("public")),
            token_css: None,
            app_id: "com.microsoft.webui.test".to_string(),
            app_name: "WebUI Test".to_string(),
            version: "0.0.0".to_string(),
            publisher: "Microsoft".to_string(),
            window: WindowOptions::default(),
            icon_file: None,
            shell: DesktopShellConfig::default(),
            package_targets: vec![DesktopPackageTarget::MacosApp],
        })
        .unwrap();

        let bundle = out.path().join("desktop");
        assert!(bundle.join("protocol.bin").is_file());
        assert!(bundle.join("state.json").is_file());
        assert!(bundle.join("assets/app.js").is_file());
        assert!(bundle.join("assets/my-card.css").is_file());
        assert!(bundle.join("assets/index.html").is_file());
        assert!(bundle.join("assets/webui-desktop-ipc.js").is_file());
        assert!(fs::read_to_string(bundle.join("assets/index.html"))
            .unwrap()
            .contains(r#"<link rel="stylesheet" href="my-card.css">"#));
        assert!(bundle.join("manifest.webui-desktop.json").is_file());
        assert_eq!(manifest.state_path, Some(PathBuf::from("state.json")));
        assert!(manifest
            .integrity
            .assets
            .iter()
            .any(|asset| asset.path == "assets/app.js"));
        assert!(manifest
            .integrity
            .assets
            .iter()
            .any(|asset| asset.path == "assets/webui-desktop-ipc.js"));
    }

    #[test]
    fn rejects_asset_collision_with_reserved_ipc_asset() {
        let app = TempDir::new().unwrap();
        let out = TempDir::new().unwrap();
        write_file(app.path(), "index.html", "<main>Hello</main>");
        write_file(app.path(), "public/webui-desktop-ipc.js", "collision");

        let err = build_desktop_bundle(DesktopBundleOptions {
            build_options: webui::BuildOptions {
                app_dir: app.path().to_path_buf(),
                entry: "index.html".to_string(),
                ..webui::BuildOptions::default()
            },
            out_dir: out.path().join("desktop"),
            state_file: None,
            asset_root: Some(app.path().join("public")),
            token_css: None,
            app_id: "com.microsoft.webui.test".to_string(),
            app_name: "WebUI Test".to_string(),
            version: "0.0.0".to_string(),
            publisher: "Microsoft".to_string(),
            window: WindowOptions::default(),
            icon_file: None,
            shell: DesktopShellConfig::default(),
            package_targets: Vec::new(),
        })
        .unwrap_err();

        assert!(matches!(err, DesktopError::BundleAssetCollision { .. }));
    }

    #[test]
    fn copies_app_icon_into_bundle_shell_metadata() {
        let app = TempDir::new().unwrap();
        let out = TempDir::new().unwrap();
        write_file(app.path(), "index.html", "<main>Hello</main>");
        write_file(app.path(), "app.icns", "icon");

        let manifest = build_desktop_bundle(DesktopBundleOptions {
            build_options: build_options(app.path().to_path_buf()),
            out_dir: out.path().join("desktop"),
            state_file: None,
            asset_root: None,
            token_css: None,
            app_id: "com.microsoft.webui.test".to_string(),
            app_name: "WebUI Test".to_string(),
            version: "0.0.0".to_string(),
            publisher: "Microsoft".to_string(),
            window: WindowOptions::default(),
            icon_file: Some(app.path().join("app.icns")),
            shell: DesktopShellConfig::default(),
            package_targets: Vec::new(),
        })
        .unwrap();

        let bundle = out.path().join("desktop");
        assert_eq!(
            manifest.shell.icon_path,
            Some(PathBuf::from("assets/app-icon.icns"))
        );
        assert!(bundle.join("assets/app-icon.icns").is_file());
        assert!(manifest
            .integrity
            .assets
            .iter()
            .any(|asset| asset.path == "assets/app-icon.icns"));
    }

    #[test]
    fn rejects_output_inside_app_directory() {
        let app = TempDir::new().unwrap();
        write_file(app.path(), "index.html", "<main>Hello</main>");

        let err = build_desktop_bundle(DesktopBundleOptions {
            build_options: build_options(app.path().to_path_buf()),
            out_dir: app.path().join("desktop"),
            state_file: None,
            asset_root: None,
            token_css: None,
            app_id: "com.microsoft.webui.test".to_string(),
            app_name: "WebUI Test".to_string(),
            version: "0.0.0".to_string(),
            publisher: "Microsoft".to_string(),
            window: WindowOptions::default(),
            icon_file: None,
            shell: DesktopShellConfig::default(),
            package_targets: Vec::new(),
        })
        .unwrap_err();

        assert!(matches!(err, DesktopError::OutputPathOverlap { .. }));
    }

    #[cfg(unix)]
    #[test]
    fn rejects_symlinked_output_inside_app_directory() {
        let root = TempDir::new().unwrap();
        let app = root.path().join("app");
        let linked_app = root.path().join("linked-app");
        write_file(&app, "index.html", "<main>Hello</main>");
        std::os::unix::fs::symlink(&app, &linked_app).unwrap();

        let err = build_desktop_bundle(DesktopBundleOptions {
            build_options: build_options(app),
            out_dir: linked_app.join("desktop"),
            state_file: None,
            asset_root: None,
            token_css: None,
            app_id: "com.microsoft.webui.test".to_string(),
            app_name: "WebUI Test".to_string(),
            version: "0.0.0".to_string(),
            publisher: "Microsoft".to_string(),
            window: WindowOptions::default(),
            icon_file: None,
            shell: DesktopShellConfig::default(),
            package_targets: Vec::new(),
        })
        .unwrap_err();

        assert!(matches!(err, DesktopError::OutputPathOverlap { .. }));
    }

    #[cfg(unix)]
    #[test]
    fn rejects_symlink_output_leaf_inside_app_directory() {
        let root = TempDir::new().unwrap();
        let app = root.path().join("app");
        let outside = root.path().join("outside");
        let linked_output = app.join("desktop");
        write_file(&app, "index.html", "<main>Hello</main>");
        fs::create_dir_all(&outside).unwrap();
        std::os::unix::fs::symlink(&outside, &linked_output).unwrap();

        let err = build_desktop_bundle(DesktopBundleOptions {
            build_options: build_options(app),
            out_dir: linked_output,
            state_file: None,
            asset_root: None,
            token_css: None,
            app_id: "com.microsoft.webui.test".to_string(),
            app_name: "WebUI Test".to_string(),
            version: "0.0.0".to_string(),
            publisher: "Microsoft".to_string(),
            window: WindowOptions::default(),
            icon_file: None,
            shell: DesktopShellConfig::default(),
            package_targets: Vec::new(),
        })
        .unwrap_err();

        assert!(matches!(err, DesktopError::OutputPathOverlap { .. }));
    }
}

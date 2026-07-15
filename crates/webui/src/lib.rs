// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

//! Programmatic API for the WebUI build-time rendering framework.
//!
//! This crate provides the core build, render, and inspection APIs
//! that power the `webui` CLI, Node.js bindings, and WASM module.
//!
//! # Example
//!
//! ```rust,no_run
//! use webui::{build, BuildOptions};
//! use std::path::PathBuf;
//!
//! let result = build(BuildOptions {
//!     app_dir: PathBuf::from("./src"),
//!     ..BuildOptions::default()
//! }).unwrap();
//!
//! println!("Built {} fragments in {:?}", result.stats.fragment_count, result.stats.duration);
//! ```

mod component_assets;
mod error;
mod projection;
pub mod server;
pub mod streaming;

pub use component_assets::{render_component_assets, ComponentAssetFile};
pub use error::WebUIError;

// Re-export core types from downstream crates
pub use webui_handler::route_handler::{
    encode_inventory, get_needed_components, get_needed_components_for_request, parse_inventory,
    ProtocolIndex,
};
pub use webui_handler::route_matcher::CompiledRouteCache;
pub use webui_handler::Result as HandlerResult;
pub use webui_handler::{
    plugin::HandlerPlugin, HandlerError, PreparedProtocol, RenderOptions, ResponseWriter,
    WebUIHandler,
};
pub use webui_parser::plugin::{ComponentTemplateArtifact, StateSurface};
pub use webui_parser::CssStrategy;
pub use webui_parser::DomStrategy;
pub use webui_parser::LegalComments;
pub use webui_parser::ParserError;
pub use webui_parser::ParserOptions;
pub use webui_parser::Plugin;
pub use webui_parser::DEFAULT_CSS_FILE_NAME_TEMPLATE;
pub use webui_parser::{
    AssetFileNameTemplate, AssetFileNameTemplateError, DEFAULT_ASSET_FILE_NAME_TEMPLATE,
};
pub use webui_parser::{CssFallbackChain, CssTokenAnalysis};
pub use webui_parser::{Diagnostic, Severity};
pub use webui_protocol::WebUIProtocol;
pub use webui_tokens::{load_token_file, resolve_theme_path, TokenFile};

use std::collections::HashSet;
use std::ffi::{OsStr, OsString};
use std::fs;
use std::path::Path;
use std::time::Instant;
use webui_parser::plugin::fast_v2::FastV2ParserPlugin;
use webui_parser::plugin::fast_v3::FastV3ParserPlugin;
use webui_parser::plugin::webui::WebUIParserPlugin;
use webui_parser::plugin::ParserPluginArtifacts;
use webui_parser::HtmlParser;

/// Projection metadata validated once for reuse across many protocol builds.
#[derive(Debug, Clone)]
pub struct PreparedProjectionManifests {
    components: std::sync::Arc<std::collections::BTreeMap<String, projection::ComponentEntry>>,
}

#[derive(Debug)]
enum ProjectionBarrierState {
    Waiting,
    Ready(PreparedProjectionManifests),
    Failed(String),
}

/// A reusable projection source that waits for an orchestrated client build.
#[doc(hidden)]
#[derive(Debug, Clone)]
pub struct PendingProjectionManifests {
    state: std::sync::Arc<(std::sync::Mutex<ProjectionBarrierState>, std::sync::Condvar)>,
}

/// Completes a pending projection source exactly once.
#[doc(hidden)]
pub struct ProjectionManifestCompleter {
    state: std::sync::Arc<(std::sync::Mutex<ProjectionBarrierState>, std::sync::Condvar)>,
    completed: bool,
}

/// One projection manifest supplied to a build.
#[derive(Debug, Clone)]
pub enum ProjectionManifestSource {
    /// Read and validate a manifest from disk.
    Path(std::path::PathBuf),
    /// Validate an already parsed/transported manifest JSON object.
    ///
    /// `manifest_path` is its logical disk location and anchors the manifest's
    /// relative `root` plus stale input/output checks.
    Inline {
        /// Logical path the manifest would occupy on disk.
        manifest_path: std::path::PathBuf,
        /// Canonical manifest JSON.
        json: String,
    },
    /// Reuse metadata that has already passed schema, hash, merge, and stale
    /// validation.
    Prepared(PreparedProjectionManifests),
    /// Wait for an orchestrator to validate the completed client bundle.
    #[doc(hidden)]
    Pending(PendingProjectionManifests),
}

impl From<std::path::PathBuf> for ProjectionManifestSource {
    fn from(path: std::path::PathBuf) -> Self {
        Self::Path(path)
    }
}

/// Validate and merge projection sources once for repeated builds.
///
/// This is intended for orchestrators such as `webui-press` that build many
/// page protocols against one completed client bundle.
pub fn prepare_projection_manifests(
    sources: &[ProjectionManifestSource],
) -> Result<PreparedProjectionManifests, WebUIError> {
    let components = projection::load_and_merge(sources)?.unwrap_or_default();
    Ok(PreparedProjectionManifests {
        components: std::sync::Arc::new(components),
    })
}

/// Create a pending projection source and its one-shot completer.
#[doc(hidden)]
#[must_use]
pub fn projection_manifest_barrier() -> (ProjectionManifestSource, ProjectionManifestCompleter) {
    let state = std::sync::Arc::new((
        std::sync::Mutex::new(ProjectionBarrierState::Waiting),
        std::sync::Condvar::new(),
    ));
    (
        ProjectionManifestSource::Pending(PendingProjectionManifests {
            state: std::sync::Arc::clone(&state),
        }),
        ProjectionManifestCompleter {
            state,
            completed: false,
        },
    )
}

impl PendingProjectionManifests {
    fn wait(&self) -> Result<PreparedProjectionManifests, WebUIError> {
        let (lock, ready) = self.state.as_ref();
        let mut state = lock.lock().map_err(|_| {
            WebUIError::InvalidBuildOptions("projection synchronization lock poisoned".to_string())
        })?;
        loop {
            match &*state {
                ProjectionBarrierState::Waiting => {
                    state = ready.wait(state).map_err(|_| {
                        WebUIError::InvalidBuildOptions(
                            "projection synchronization lock poisoned".to_string(),
                        )
                    })?;
                }
                ProjectionBarrierState::Ready(projection) => {
                    return Ok(projection.clone());
                }
                ProjectionBarrierState::Failed(message) => {
                    return Err(WebUIError::InvalidBuildOptions(message.clone()));
                }
            }
        }
    }
}

impl ProjectionManifestCompleter {
    /// Wake waiting builds with validated metadata or a producer failure.
    pub fn complete(mut self, result: std::result::Result<PreparedProjectionManifests, String>) {
        let (lock, ready) = self.state.as_ref();
        if let Ok(mut state) = lock.lock() {
            *state = match result {
                Ok(projection) => ProjectionBarrierState::Ready(projection),
                Err(message) => ProjectionBarrierState::Failed(message),
            };
            self.completed = true;
            ready.notify_all();
        }
    }
}

impl Drop for ProjectionManifestCompleter {
    fn drop(&mut self) {
        if self.completed {
            return;
        }
        let (lock, ready) = self.state.as_ref();
        if let Ok(mut state) = lock.lock() {
            *state = ProjectionBarrierState::Failed(
                "projection producer terminated before completing metadata".to_string(),
            );
            ready.notify_all();
        }
    }
}

/// Options for building a WebUI application.
#[derive(Debug, Clone)]
pub struct BuildOptions {
    /// Path to the application folder containing templates.
    pub app_dir: std::path::PathBuf,
    /// Entry HTML file name (e.g., `"index.html"`).
    pub entry: String,
    /// CSS delivery strategy for component stylesheets.
    pub css: CssStrategy,
    /// DOM strategy for component rendering (shadow or light).
    pub dom: DomStrategy,
    /// Framework plugin to load.
    pub plugin: Option<Plugin>,
    /// Additional component sources (npm packages or local paths).
    pub components: Vec<String>,
    /// Additional root components to compile for static component asset emission.
    ///
    /// These roots are parsed into the protocol so their templates, styles, and
    /// dependency closures can be emitted later, but they are not connected to
    /// the entry fragment and therefore are not rendered during initial SSR.
    pub component_asset_roots: Vec<String>,
    /// Emitted asset filename template using `[name]`, `[hash]`, and `[ext]`.
    ///
    /// Applies to Link-mode CSS files and static component assets.
    pub css_file_name_template: String,
    /// Optional URL/base-path prefix for Link-mode `css_href` values.
    /// When set, emitted protocol hrefs become `<base>/<filename>`.
    pub css_public_base: Option<String>,
    /// Legal comment preservation strategy.
    pub legal_comments: LegalComments,
    /// Optional loaded design-token theme used to validate discovered CSS tokens.
    ///
    /// When present, every discovered CSS token that remains unresolved after
    /// local and ancestor CSS definitions are removed must exist in every theme,
    /// unless the `var()` usage supplies a literal CSS fallback (e.g.
    /// `var(--x, 16px)`), which provides its own value. Missing required tokens
    /// fail the build as parser diagnostics.
    pub theme: Option<TokenFile>,
    /// Bundler-neutral state projection manifest fragments supplied as disk
    /// paths or inline JSON with a logical manifest path.
    ///
    /// Empty (the default) means projection is disabled: the build emits
    /// [`webui_protocol::InitialStateStrategy::Full`] and every scripted
    /// component keeps a full-state (`All`) partial-navigation surface. No
    /// JavaScript/TypeScript analysis ever runs in Rust.
    ///
    /// When non-empty, every manifest is loaded, hash-validated against the
    /// files it declares, merged by component tag, and used to narrow each
    /// scripted component that is actually compiled into the protocol down to
    /// an exact hydration/navigation key surface. Every such scripted
    /// component must have exactly one manifest entry, or the build fails
    /// with [`WebUIError::Projection`] (`PROJ-B001`). Supplying manifests with
    /// a `plugin` other than [`Plugin::WebUI`] fails the build immediately
    /// (`PROJ-B002`): only the WebUI plugin produces protocol fields
    /// compatible with per-component key encoding.
    ///
    /// Manifests are read only at build time. The runtime handler never
    /// opens a manifest file; the compiled protocol is fully self-contained.
    pub projection_manifests: Vec<ProjectionManifestSource>,
}

impl Default for BuildOptions {
    fn default() -> Self {
        Self {
            app_dir: std::path::PathBuf::from("."),
            entry: "index.html".to_string(),
            css: CssStrategy::Link,
            dom: DomStrategy::Shadow,
            plugin: None,
            components: Vec::new(),
            component_asset_roots: Vec::new(),
            css_file_name_template: DEFAULT_CSS_FILE_NAME_TEMPLATE.to_string(),
            css_public_base: None,
            legal_comments: LegalComments::default(),
            theme: None,
            projection_manifests: Vec::new(),
        }
    }
}

/// Statistics about a completed build.
#[derive(Debug, Clone)]
pub struct BuildStats {
    /// Total wall-clock build time.
    pub duration: std::time::Duration,
    /// Total number of protocol fragments.
    pub fragment_count: usize,
    /// Number of registered components.
    pub component_count: usize,
    /// Number of CSS files produced.
    pub css_file_count: usize,
    /// Size of the serialized protocol in bytes.
    pub protocol_size_bytes: usize,
    /// Number of unique CSS tokens discovered.
    pub token_count: usize,
}

/// Result of a successful build.
#[derive(Debug)]
pub struct BuildResult {
    /// The compiled WebUI protocol.
    pub protocol: WebUIProtocol,
    /// Serialized protocol bytes (protobuf binary).
    pub protocol_bytes: Vec<u8>,
    /// Component CSS files: `(filename, content)` — only components referenced in the protocol.
    pub css_files: Vec<(String, String)>,
    /// Static component asset files: `(filename, ESM module content)`.
    ///
    /// Populated when [`BuildOptions::component_asset_roots`] is non-empty.
    pub component_asset_files: Vec<ComponentAssetFile>,
    /// Component client template payloads.
    /// Includes templates for all components encountered during parsing,
    /// including route-referenced components.
    pub component_templates: Vec<ComponentTemplateArtifact>,
    /// Non-fatal build advisories as warning-severity [`Diagnostic`]s.
    ///
    /// Currently surfaces CSS tokens that are referenced only with a literal
    /// `var()` fallback and defined in no theme — often typos. Empty when no
    /// theme is supplied. Carries the same structured location/snippet/`help:`
    /// data as errors; the entry point decides how to present (and color) them.
    pub warnings: Vec<Diagnostic>,
    /// Build statistics.
    pub stats: BuildStats,
}

/// Build a WebUI application from an app directory.
///
/// Parses templates, discovers components, and produces a compiled protocol
/// with build statistics.
///
/// # Errors
///
/// Returns [`WebUIError`] if the app directory is invalid, templates fail
/// to parse, theme tokens are incomplete, or the protocol cannot be serialized.
#[must_use = "BuildResult contains the compiled protocol and statistics"]
pub fn build(options: BuildOptions) -> Result<BuildResult, WebUIError> {
    let started = Instant::now();

    let raw = build_protocol_inner(&options)?;

    let protocol_bytes = raw.protocol.to_protobuf()?;

    let stats = BuildStats {
        duration: started.elapsed(),
        fragment_count: raw.fragment_count,
        component_count: raw.component_count,
        css_file_count: raw.css_files.len(),
        protocol_size_bytes: protocol_bytes.len(),
        token_count: raw.token_count,
    };

    Ok(BuildResult {
        protocol: raw.protocol,
        protocol_bytes,
        css_files: raw.css_files,
        component_asset_files: raw.component_asset_files,
        component_templates: raw.component_templates,
        warnings: raw.warnings,
        stats,
    })
}

/// Build a WebUI application and write output files to disk.
///
/// Writes `protocol.bin`, any external CSS files, and static component assets
/// to `out_dir`.
/// Creates `out_dir` if it does not exist.
///
/// # Errors
///
/// Returns [`WebUIError`] on build failure or if output files cannot be written.
pub fn build_to_disk(options: BuildOptions, out_dir: &Path) -> Result<BuildStats, WebUIError> {
    let result = build(options)?;
    validate_output_file_names(OsStr::new("protocol.bin"), &result)?;

    fs::create_dir_all(out_dir).map_err(|source| WebUIError::Io {
        context: format!("Failed to create {}", out_dir.display()),
        source,
    })?;

    fs::write(out_dir.join("protocol.bin"), &result.protocol_bytes).map_err(|source| {
        WebUIError::Io {
            context: format!("Failed to write protocol.bin to {}", out_dir.display()),
            source,
        }
    })?;

    for (name, content) in &result.css_files {
        fs::write(out_dir.join(name), content).map_err(|source| WebUIError::Io {
            context: format!("Failed to write {name} to {}", out_dir.display()),
            source,
        })?;
    }
    for file in &result.component_asset_files {
        fs::write(out_dir.join(&file.name), &file.content).map_err(|source| WebUIError::Io {
            context: format!(
                "Failed to write component asset {} to {}",
                file.name,
                out_dir.display()
            ),
            source,
        })?;
    }

    Ok(result.stats)
}

/// Inspect a compiled WebUI protocol file and return its JSON representation.
pub fn inspect(protocol_path: &Path) -> Result<String, WebUIError> {
    let bytes = fs::read(protocol_path).map_err(|source| WebUIError::Io {
        context: format!("Failed to read {}", protocol_path.display()),
        source,
    })?;
    inspect_bytes(&bytes)
}

/// Inspect raw protocol bytes and return their JSON representation.
pub fn inspect_bytes(protocol_bytes: &[u8]) -> Result<String, WebUIError> {
    let protocol = WebUIProtocol::from_protobuf(protocol_bytes)?;
    protocol
        .to_json_pretty()
        .map_err(|e| WebUIError::Serialization(e.to_string()))
}

/// Internal intermediate build output before stats are computed.
struct RawBuildOutput {
    protocol: WebUIProtocol,
    css_files: Vec<(String, String)>,
    component_asset_files: Vec<ComponentAssetFile>,
    component_templates: Vec<ComponentTemplateArtifact>,
    warnings: Vec<Diagnostic>,
    fragment_count: usize,
    component_count: usize,
    token_count: usize,
}

/// Internal build logic shared by `build()` and `build_to_disk()`.
fn build_protocol_inner(options: &BuildOptions) -> Result<RawBuildOutput, WebUIError> {
    let projection_enabled = !options.projection_manifests.is_empty();
    if projection_enabled && options.plugin != Some(Plugin::WebUI) {
        return Err(projection::incompatible_plugin_error());
    }
    let parser_options = ParserOptions::try_new(
        options.css,
        options.dom,
        &options.css_file_name_template,
        options.css_public_base.as_deref(),
        options.legal_comments,
    )
    .map_err(|e| WebUIError::InvalidBuildOptions(e.to_string()))?;
    let css_link_options = parser_options.css_link_options.clone();

    let mut parser = match options.plugin {
        Some(Plugin::Fast | Plugin::FastV2) => {
            HtmlParser::with_plugin_options(Box::new(FastV2ParserPlugin::new()), parser_options)
        }
        Some(Plugin::FastV3) => {
            HtmlParser::with_plugin_options(Box::new(FastV3ParserPlugin::new()), parser_options)
        }
        Some(Plugin::WebUI) => {
            HtmlParser::with_plugin_options(Box::new(WebUIParserPlugin::new()), parser_options)
        }
        None => HtmlParser::with_options(parser_options),
    };

    // Register app directory components
    parser
        .component_registry_mut()
        .register_from_paths(&[&options.app_dir])
        .map_err(|e| {
            WebUIError::ComponentRegistration(format!(
                "Failed to register components from {}: {e}",
                options.app_dir.display()
            ))
        })?;

    // Discover and register external component sources
    for source in &options.components {
        let result = webui_discovery::discover_source(source, &options.app_dir).map_err(|e| {
            WebUIError::ComponentDiscovery(format!(
                "Failed to discover components from {source}: {e}"
            ))
        })?;
        for comp in &result.components {
            // Script presence marks client ownership. Rust never analyzes the
            // source; exact state surfaces come from projection manifests.
            parser
                .component_registry_mut()
                .register_component(webui_parser::ComponentRegistration {
                    tag_name: &comp.tag_name,
                    html_content: &comp.html_content,
                    css_content: comp.css_content.as_deref(),
                    is_client_owned: comp.is_client_owned,
                })
                .map_err(|e| {
                    WebUIError::ComponentRegistration(format!(
                        "Failed to register component '{}' from {}: {e}",
                        comp.tag_name, comp.source
                    ))
                })?;
        }
    }

    let component_count = parser.component_registry().len();

    // Parse entry HTML
    let entry_path = options.app_dir.join(&options.entry);
    let html_content = fs::read_to_string(&entry_path).map_err(|source| WebUIError::Io {
        context: format!("Failed to read {}", entry_path.display()),
        source,
    })?;
    parser
        .parse(&options.entry, &html_content)
        .map_err(|source| WebUIError::Parse {
            context: format!("Failed to parse {}", options.entry),
            source,
        })?;

    let synthetic_asset_fragments =
        parse_component_asset_roots(&mut parser, &options.component_asset_roots)?;

    let css_snapshot: Vec<(String, String)> = parser
        .component_registry()
        .get_all()
        .filter(|component| parser.has_fragment(&component.tag_name))
        .filter_map(|component| {
            component
                .css_content
                .as_ref()
                .map(|css| (component.tag_name.clone(), css.clone()))
        })
        .collect();

    // Collect CSS token analysis before consuming the parser.
    let token_analysis = parser.token_analysis();
    let mut warnings: Vec<Diagnostic> = Vec::new();
    if let Some(theme) = options.theme.as_ref() {
        token_analysis
            .validate_theme_tokens(theme)
            .map_err(|source| WebUIError::Parse {
                context: "Failed to validate theme tokens".to_string(),
                source,
            })?;
        // Advisory: a token used only as `var(--x, <literal>)` and absent from
        // every theme is often a misspelling. The literal keeps the build green,
        // so this is a warning (with a "did you mean …?" suggestion), not an
        // error.
        warnings = token_analysis.theme_token_warnings(theme);
    }
    let token_count = token_analysis.protocol_tokens.len();

    let component_templates =
        match parser
            .take_plugin_artifacts()
            .map_err(|source| WebUIError::Parse {
                context: format!(
                    "Failed to compile component templates for {}",
                    options.entry
                ),
                source,
            })? {
            ParserPluginArtifacts::None => Vec::new(),
            ParserPluginArtifacts::ComponentTemplates(templates) => templates,
        };
    // Build protocol (consumes parser)
    let mut fragment_records = parser.into_fragment_records();
    for fragment_id in synthetic_asset_fragments {
        fragment_records.remove(&fragment_id);
    }
    let fragment_count: usize = fragment_records.values().map(|v| v.fragments.len()).sum();

    // Resolve projection only after template compilation. Ordinary path/inline
    // builds pay the same validation cost, while orchestrators can overlap
    // parser work with an in-flight client bundle through a pending source.
    let merged_manifest = projection::load_and_merge(&options.projection_manifests)?;
    let mut protocol = WebUIProtocol::with_tokens(fragment_records, token_analysis.protocol_tokens);
    protocol.initial_state_strategy = if merged_manifest.is_some() {
        webui_protocol::InitialStateStrategy::Components as i32
    } else {
        webui_protocol::InitialStateStrategy::Full as i32
    };

    // Strict coverage applies only to scripted components that actually made
    // it into the compiled protocol/route closure (i.e. their fragment was
    // emitted). Scripted components discovered but never compiled into a
    // route/fragment do not require manifest coverage.
    if let Some(merged) = &merged_manifest {
        let compiled_scripted_tags: Vec<&str> = component_templates
            .iter()
            .filter(|artifact| {
                artifact.is_scripted && protocol.fragments.contains_key(&artifact.tag_name)
            })
            .map(|artifact| artifact.tag_name.as_str())
            .collect();
        projection::validate_coverage(merged, &compiled_scripted_tags)?;
    }

    // Record build-wide strategies so the handler can decide rendering behavior.
    protocol.set_css_strategy(match options.css {
        CssStrategy::Link => webui_protocol::CssStrategy::Link,
        CssStrategy::Style => webui_protocol::CssStrategy::Style,
        CssStrategy::Module => webui_protocol::CssStrategy::Module,
    });
    protocol.set_dom_strategy(match options.dom {
        DomStrategy::Shadow => webui_protocol::DomStrategy::Shadow,
        DomStrategy::Light => webui_protocol::DomStrategy::Light,
    });

    // Process component CSS in a single pass: store Module CSS content,
    // set Link-strategy css_href, and collect external CSS files.
    let is_module = options.css == CssStrategy::Module;
    let is_link = options.css == CssStrategy::Link;
    let mut css_files: Vec<(String, String)> = Vec::new();
    let mut emitted_names: HashSet<String> = HashSet::new();
    for (tag, css) in css_snapshot {
        if !protocol.fragments.contains_key(&tag) {
            continue;
        }
        if is_module {
            protocol.components.entry(tag).or_default().css = css.trim().to_string();
        } else if is_link {
            let resolved = css_link_options.resolve(&tag, &css);
            if !emitted_names.insert(resolved.filename.clone()) {
                return Err(WebUIError::InvalidBuildOptions(format!(
                    "CSS filename collision for Link strategy: '{}'. Adjust the asset filename template to include [name] or another unique component-specific segment.",
                    resolved.filename
                )));
            }
            protocol.components.entry(tag).or_default().css_href = resolved.href;
            css_files.push((resolved.filename, css));
        }
        // Style strategy: CSS is already baked into raw fragments by the
        // parser — nothing to store in the protocol or emit as files.
    }

    // Store compiled client templates in the protocol so any host server can
    // query them. Scriptless templates retain navigation metadata but contribute
    // no initial hydration state.
    for artifact in &component_templates {
        let component = protocol
            .components
            .entry(artifact.tag_name.clone())
            .or_default();
        component.template = artifact.template.clone();
        component.template_json = artifact.template_json.clone();
        component.template_functions = artifact.template_functions.clone();
        // Manifest keys are the initial hydration surface. Navigation is the
        // union of manifest client keys and Rust-compiled template roots.
        // Scriptless components are never governed by a manifest: they keep
        // their Rust-derived surface regardless of projection.
        let manifest_entry = if artifact.is_scripted {
            merged_manifest
                .as_ref()
                .and_then(|merged| merged.get(&artifact.tag_name))
        } else {
            None
        };
        let (hydration_mode, hydration_keys) = match manifest_entry {
            Some(entry) => encode_state_surface(&StateSurface::Keys(entry.hydration_keys.clone())),
            None => encode_state_surface(&artifact.hydration),
        };
        component.hydration_mode = hydration_mode;
        component.hydration_keys = hydration_keys;
        let (navigation_mode, navigation_keys) = match manifest_entry {
            Some(entry) => {
                let union =
                    projection::union_keys(&entry.navigation_keys, &artifact.template_roots);
                encode_state_surface(&StateSurface::Keys(union))
            }
            None => encode_state_surface(&artifact.navigation),
        };
        component.navigation_mode = navigation_mode;
        component.navigation_keys = navigation_keys;
    }

    let component_asset_files = component_assets::render_component_assets(
        &protocol,
        &options.component_asset_roots,
        &options.css_file_name_template,
    )?;
    validate_generated_file_names(&css_files, &component_asset_files)?;

    Ok(RawBuildOutput {
        protocol,
        css_files,
        component_asset_files,
        component_templates,
        warnings,
        fragment_count,
        component_count,
        token_count,
    })
}

fn encode_state_surface(surface: &StateSurface) -> (i32, Vec<String>) {
    match surface {
        StateSurface::None => (webui_protocol::StateProjectionMode::None as i32, Vec::new()),
        StateSurface::Keys(keys) => (
            webui_protocol::StateProjectionMode::Keys as i32,
            keys.clone(),
        ),
        StateSurface::All => (webui_protocol::StateProjectionMode::All as i32, Vec::new()),
    }
}

fn parse_component_asset_roots(
    parser: &mut HtmlParser,
    roots: &[String],
) -> Result<Vec<String>, WebUIError> {
    let mut synthetic_fragments = Vec::with_capacity(roots.len());
    for (index, root) in roots.iter().enumerate() {
        let root = root.trim();
        if parser.has_fragment(root) {
            continue;
        }
        // Compile the requested lazy root without connecting it to the entry
        // graph, so it can be emitted as a static asset but stays out of SSR.
        let mut synthetic = String::with_capacity(root.len() * 2 + 5);
        synthetic.push('<');
        synthetic.push_str(root);
        synthetic.push_str("></");
        synthetic.push_str(root);
        synthetic.push('>');

        let fragment_id = format!("__webui_asset_root_{index}");
        parser
            .parse(&fragment_id, &synthetic)
            .map_err(|source| WebUIError::Parse {
                context: format!("Failed to parse component asset root <{root}>"),
                source,
            })?;
        synthetic_fragments.push(fragment_id);
    }
    Ok(synthetic_fragments)
}

fn validate_generated_file_names(
    css_files: &[(String, String)],
    component_asset_files: &[ComponentAssetFile],
) -> Result<(), WebUIError> {
    let mut names = HashSet::with_capacity(css_files.len() + component_asset_files.len());
    for (name, _) in css_files {
        names.insert(name.as_str());
    }
    for file in component_asset_files {
        if !names.insert(file.name.as_str()) {
            return Err(WebUIError::InvalidBuildOptions(format!(
                "output filename collision for '{}'. Adjust the asset filename template to include [ext] or another unique asset-type segment.",
                file.name
            )));
        }
    }
    Ok(())
}

fn validate_output_file_names(
    protocol_name: &OsStr,
    result: &BuildResult,
) -> Result<(), WebUIError> {
    let mut names =
        HashSet::with_capacity(1 + result.css_files.len() + result.component_asset_files.len());
    names.insert(protocol_name.to_os_string());
    for (name, _) in &result.css_files {
        let file_name = OsString::from(name);
        if !names.insert(file_name.clone()) {
            return Err(output_file_collision_error(&file_name));
        }
    }
    for file in &result.component_asset_files {
        let file_name = OsString::from(&file.name);
        if !names.insert(file_name.clone()) {
            return Err(output_file_collision_error(&file_name));
        }
    }
    Ok(())
}

fn output_file_collision_error(name: &OsStr) -> WebUIError {
    WebUIError::InvalidBuildOptions(format!(
        "output filename collision for '{}'. Adjust the asset filename template to include [ext] or another unique asset-type segment.",
        name.to_string_lossy()
    ))
}

#[cfg(test)]
#[allow(clippy::disallowed_methods)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use std::path::PathBuf;
    use tempfile::TempDir;
    use webui_handler::plugin::webui::WebUIHydrationPlugin;
    use webui_protocol::web_ui_fragment::Fragment;

    struct StringWriter {
        buf: String,
    }

    impl ResponseWriter for StringWriter {
        fn write(&mut self, content: &str) -> webui_handler::Result<()> {
            self.buf.push_str(content);
            Ok(())
        }

        fn end(&mut self) -> webui_handler::Result<()> {
            Ok(())
        }
    }

    fn create_app_dir(files: &[(&str, &str)]) -> TempDir {
        let dir = TempDir::new().unwrap();
        for (name, content) in files {
            let path = dir.path().join(name);
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent).unwrap();
            }
            fs::write(&path, content).unwrap();
        }
        dir
    }

    fn default_options(app_dir: &Path) -> BuildOptions {
        BuildOptions {
            app_dir: app_dir.to_path_buf(),
            ..BuildOptions::default()
        }
    }

    #[test]
    fn test_build_simple_html() {
        let app = create_app_dir(&[("index.html", "<h1>Hello</h1>")]);
        let result = build(default_options(app.path())).unwrap();

        assert!(result.protocol.fragments.contains_key("index.html"));
        assert!(result.stats.fragment_count > 0);
        assert!(result.stats.protocol_size_bytes > 0);
        assert!(!result.stats.duration.is_zero());
        assert_eq!(
            result.protocol.initial_state_strategy,
            webui_protocol::InitialStateStrategy::Full as i32
        );
    }

    #[test]
    fn pending_projection_unblocks_after_template_compilation() {
        let app = create_app_dir(&[("index.html", "<h1>Hello</h1>")]);
        let (source, completer) = projection_manifest_barrier();
        let options = BuildOptions {
            app_dir: app.path().to_path_buf(),
            plugin: Some(Plugin::WebUI),
            projection_manifests: vec![source],
            ..BuildOptions::default()
        };
        let handle = std::thread::spawn(move || build(options));
        let prepared = prepare_projection_manifests(&[]).unwrap();
        completer.complete(Ok(prepared));
        let result = handle.join().unwrap().unwrap();
        assert_eq!(
            result.protocol.initial_state_strategy,
            webui_protocol::InitialStateStrategy::Components as i32
        );
    }

    #[test]
    fn test_build_keeps_scriptless_component_dormant_until_client_use() {
        let app = create_app_dir(&[
            (
                "index.html",
                "<html><body><demo-card></demo-card></body></html>",
            ),
            ("demo-card.html", "<p>{{name}} {{serverTitle}}</p>"),
        ]);
        // An empty manifest fragment (no scripted components exist in this
        // app, so strict coverage trivially passes) still turns on
        // `InitialStateStrategy::Components`, letting scriptless components'
        // Rust-derived dormant surface (`None` hydration) take effect.
        let manifest_json =
            projection::test_support::build_valid_manifest_json(app.path(), &[], &[], &[]);
        let manifest = projection::test_support::write_manifest(
            app.path(),
            "webui-projection.json",
            &manifest_json,
        );
        let mut options = default_options(app.path());
        options.plugin = Some(Plugin::WebUI);
        options.projection_manifests = vec![manifest.into()];
        let result = build(options).unwrap();
        assert_eq!(
            result.protocol.initial_state_strategy,
            webui_protocol::InitialStateStrategy::Components as i32
        );

        assert!(result
            .component_templates
            .iter()
            .any(|template| template.tag_name == "demo-card"));
        let component = result
            .protocol
            .components
            .get("demo-card")
            .expect("scriptless component metadata should be retained");
        assert!(component.hydration_keys.is_empty());
        assert_eq!(component.navigation_keys, ["name", "serverTitle"]);
        assert!(component.template_json.contains(r#""th":1"#));

        let handler = WebUIHandler::with_plugin(|| Box::new(WebUIHydrationPlugin::new()));
        let state = serde_json::json!({ "name": "Server rendered" });
        let mut writer = StringWriter { buf: String::new() };
        handler
            .handle(
                &result.protocol,
                &state,
                &RenderOptions::new("index.html", "/"),
                &mut writer,
            )
            .unwrap();
        assert!(writer.buf.contains("Server rendered"));
        assert!(writer.buf.contains(r#""state":{}"#));
        assert!(writer.buf.contains(r#""templates":{"demo-card":"#));
    }

    #[test]
    fn test_build_treats_sibling_component_script_as_scripted() {
        let app = create_app_dir(&[
            ("index.html", "<demo-card></demo-card>"),
            (
                "demo-card.html",
                r#"<button @click="{onClick()}">{{name}}</button>"#,
            ),
            (
                "demo-card.ts",
                "import { WebUIElement } from '@microsoft/webui-framework';\n\
                 class DemoCard extends WebUIElement { onClick() {} }\n\
                 DemoCard.define('demo-card');",
            ),
        ]);
        let mut options = default_options(app.path());
        options.plugin = Some(Plugin::WebUI);
        let result = build(options).unwrap();

        let template = result
            .component_templates
            .iter()
            .find(|template| template.tag_name == "demo-card")
            .unwrap();
        assert!(template.template_json.contains(r#""eg":[["click""#));
    }

    #[test]
    fn test_build_populates_component_hydration_keys_from_script() {
        let app = create_app_dir(&[
            ("index.html", "<demo-card></demo-card>"),
            ("demo-card.html", "<p>{{name}}</p>"),
            (
                "demo-card.ts",
                "import { WebUIElement, attr, observable } from '@microsoft/webui-framework';\n\
                 class DemoCard extends WebUIElement {\n\
                 @observable name = '';\n\
                 @attr({ attribute: 'cta-href' }) ctaHref = '/x';\n\
                 @observable count = 0;\n\
                 }\n\
                 DemoCard.define('demo-card');",
            ),
        ]);
        // Rust performs no JavaScript/TypeScript analysis: the exact keys
        // come from a bundler-produced projection manifest, not from
        // scanning `demo-card.ts`.
        let manifest_json = projection::test_support::build_valid_manifest_json(
            app.path(),
            &[("demo-card.ts", "source")],
            &[("demo-card.js", "output")],
            &[(
                "demo-card",
                "demo-card.ts",
                &["demo-card.js"],
                &["count", "ctaHref", "name"],
                &["count", "ctaHref", "name"],
            )],
        );
        let manifest = projection::test_support::write_manifest(
            app.path(),
            "webui-projection.json",
            &manifest_json,
        );
        let mut options = default_options(app.path());
        options.plugin = Some(Plugin::WebUI);
        options.projection_manifests = vec![manifest.into()];
        let result = build(options).unwrap();

        assert_eq!(
            result.protocol.initial_state_strategy,
            webui_protocol::InitialStateStrategy::Components as i32
        );
        let component = result.protocol.components.get("demo-card").unwrap();
        assert_eq!(
            component.hydration_mode,
            webui_protocol::StateProjectionMode::Keys as i32
        );
        assert_eq!(component.hydration_keys, vec!["count", "ctaHref", "name"]);
        assert_eq!(
            component.navigation_mode,
            webui_protocol::StateProjectionMode::Keys as i32
        );
        assert_eq!(component.navigation_keys, vec!["count", "ctaHref", "name"]);
    }

    #[test]
    fn test_build_scripted_component_without_manifest_falls_back_to_full_state() {
        let app = create_app_dir(&[
            (
                "index.html",
                "<html><body><demo-card></demo-card></body></html>",
            ),
            ("demo-card.html", "<p>{{name}}</p>"),
            (
                "demo-card.ts",
                "import { WebUIElement, observable } from '@microsoft/webui-framework';\n\
                 class DemoCard extends WebUIElement {\n\
                 @observable name = '';\n\
                 }\n\
                 DemoCard.define('demo-card');",
            ),
        ]);
        let mut options = default_options(app.path());
        options.plugin = Some(Plugin::WebUI);
        // No `projection_manifests` supplied: without a manifest, Rust never
        // analyzes `demo-card.ts` and must assume the full, unproven surface.
        let result = build(options).unwrap();

        assert_eq!(
            result.protocol.initial_state_strategy,
            webui_protocol::InitialStateStrategy::Full as i32
        );
        let component = result.protocol.components.get("demo-card").unwrap();
        assert_eq!(
            component.hydration_mode,
            webui_protocol::StateProjectionMode::All as i32
        );
        assert!(component.hydration_keys.is_empty());
        assert_eq!(
            component.navigation_mode,
            webui_protocol::StateProjectionMode::All as i32
        );

        let handler = WebUIHandler::with_plugin(|| Box::new(WebUIHydrationPlugin::new()));
        let state = serde_json::json!({ "name": "Visible" });
        let mut writer = StringWriter { buf: String::new() };
        handler
            .handle(
                &result.protocol,
                &state,
                &RenderOptions::new("index.html", "/"),
                &mut writer,
            )
            .unwrap();
        assert!(writer.buf.contains("Visible"));
        // Full-state fallback serializes the entire supplied state object,
        // not a narrowed key subset.
        assert!(writer.buf.contains(r#""state":{"name":"Visible"}"#));
    }

    #[test]
    fn test_build_rejects_scriptless_event_component() {
        let app = create_app_dir(&[
            ("index.html", "<demo-card></demo-card>"),
            (
                "demo-card.html",
                r#"<button @click="{onClick()}">Click</button>"#,
            ),
        ]);
        let mut options = default_options(app.path());
        options.plugin = Some(Plugin::WebUI);
        let err = build(options).expect_err("scriptless events should fail");

        let message = err.chain_message();
        assert!(
            message.contains("scriptless-event-handler"),
            "msg: {message}"
        );
    }

    #[test]
    fn test_build_returns_actionable_error_for_invalid_w_ref() {
        let app = create_app_dir(&[
            ("index.html", "<my-card>Hello</my-card>"),
            ("my-card.html", r#"<div><input w-ref="myInput" /></div>"#),
        ]);
        let mut options = default_options(app.path());
        options.plugin = Some(Plugin::WebUI);

        // Authoring mistakes are returned (not panicked) so every consumer —
        // CLI, Node, FFI, WASM — can surface the message its own way.
        let Err(err) = build(options) else {
            panic!("non-braced w-ref must fail the build");
        };
        let msg = err.chain_message();
        assert!(msg.contains("invalid w-ref binding"), "msg: {msg}");
        assert!(
            msg.contains("component <my-card> · element <input>"),
            "msg: {msg}"
        );
        assert!(msg.contains("help:"), "msg: {msg}");
    }

    #[test]
    fn test_build_returns_actionable_error_for_invalid_event_handler() {
        let app = create_app_dir(&[
            ("index.html", "<my-btn>x</my-btn>"),
            (
                "my-btn.html",
                r#"<div><button @click="e.preventDefault()">x</button></div>"#,
            ),
        ]);
        let mut options = default_options(app.path());
        options.plugin = Some(Plugin::WebUI);

        let Err(err) = build(options) else {
            panic!("invalid @event handler must fail the build");
        };
        let msg = err.chain_message();
        assert!(msg.contains("invalid @click handler"), "msg: {msg}");
        assert!(
            msg.contains("component <my-btn> · element <button>"),
            "msg: {msg}"
        );
    }

    #[test]
    fn test_error_chain_message_has_no_duplicate_source() {
        let err = WebUIError::Parse {
            context: "Failed to parse index.html".to_string(),
            source: webui_parser::ParserError::Directive("Invalid for each: x".to_string()),
        };
        // Each layer's Display describes only its own level — the source is not
        // embedded — so anyhow's `{:#}` / chain walks never repeat it.
        assert_eq!(err.to_string(), "Failed to parse index.html");
        // The flat message includes the source exactly once (no repetition).
        assert_eq!(
            err.chain_message(),
            "Failed to parse index.html: Directive error: Invalid for each: x"
        );
    }

    #[test]
    fn test_build_with_directives() {
        let html = r#"<h1>Hello</h1>
<for each="item in items">
    <p>{{item.name}}</p>
</for>
<if condition="show">
    <p>Visible</p>
</if>"#;
        let app = create_app_dir(&[("index.html", html)]);
        let result = build(default_options(app.path())).unwrap();

        let index = &result.protocol.fragments["index.html"].fragments;
        assert!(index
            .iter()
            .any(|f| matches!(f.fragment.as_ref(), Some(Fragment::ForLoop(_)))));
        assert!(index
            .iter()
            .any(|f| matches!(f.fragment.as_ref(), Some(Fragment::IfCond(_)))));
    }

    #[test]
    fn test_build_with_component_css() {
        let app = create_app_dir(&[
            ("index.html", "<my-card>Hello</my-card>"),
            ("my-card.html", "<div><slot></slot></div>"),
            ("my-card.css", ".card { color: red; }"),
            ("my-card.ts", "export {};"),
        ]);
        let result = build(default_options(app.path())).unwrap();

        assert_eq!(result.css_files.len(), 1);
        assert_eq!(result.css_files[0].0, "my-card.css");
        assert!(result.css_files[0].1.contains("color: red"));
        assert_eq!(result.stats.css_file_count, 1);
    }

    #[test]
    fn test_build_strips_non_legal_css_comments_from_output_file() {
        let app = create_app_dir(&[
            ("index.html", "<my-card>Hello</my-card>"),
            ("my-card.html", "<div><slot></slot></div>"),
            (
                "my-card.css",
                "/* remove var(--ignored) */ .card { color: var(--textColor); }",
            ),
        ]);
        let result = build(default_options(app.path())).unwrap();

        assert_eq!(result.css_files[0].1, " .card { color: var(--textColor); }");
        assert_eq!(result.protocol.tokens, vec!["textColor"]);
    }

    #[test]
    fn test_build_preserves_legal_css_comments_by_default() {
        let app = create_app_dir(&[
            ("index.html", "<my-card>Hello</my-card>"),
            ("my-card.html", "<div><slot></slot></div>"),
            (
                "my-card.css",
                "/*! @license MIT */ .card { color: red; } /* remove */",
            ),
        ]);
        let result = build(default_options(app.path())).unwrap();

        assert_eq!(
            result.css_files[0].1,
            "/*! @license MIT */ .card { color: red; } "
        );
    }

    #[test]
    fn test_build_legal_comments_none_strips_legal_css_comments() {
        let app = create_app_dir(&[
            ("index.html", "<my-card>Hello</my-card>"),
            ("my-card.html", "<div><slot></slot></div>"),
            ("my-card.css", "/*! @license MIT */ .card { color: red; }"),
        ]);
        let mut options = default_options(app.path());
        options.legal_comments = LegalComments::None;
        let result = build(options).unwrap();

        assert_eq!(result.css_files[0].1, " .card { color: red; }");
    }

    #[test]
    fn test_build_with_css_hashed_filename_template() {
        let app = create_app_dir(&[
            ("index.html", "<my-card>Hello</my-card>"),
            ("my-card.html", "<div><slot></slot></div>"),
            ("my-card.css", ".card { color: red; }"),
        ]);
        let mut options = default_options(app.path());
        options.css_file_name_template = "[name]-[hash].[ext]".to_string();
        let result = build(options).unwrap();

        assert_eq!(result.css_files.len(), 1);
        let generated = &result.css_files[0].0;
        assert!(generated.starts_with("my-card-"));
        assert!(generated.ends_with(".css"));
        assert_eq!(generated.len(), "my-card-".len() + 8 + ".css".len());
        assert_eq!(
            result
                .protocol
                .components
                .get("my-card")
                .map(|c| c.css_href.as_str()),
            Some(generated.as_str())
        );
    }

    #[test]
    fn test_css_public_base_prefixes_css_href_only() {
        let app = create_app_dir(&[
            ("index.html", "<my-card>Hello</my-card>"),
            ("my-card.html", "<div><slot></slot></div>"),
            ("my-card.css", ".card { color: red; }"),
        ]);
        let mut options = default_options(app.path());
        options.css_file_name_template = "[name]-[hash].[ext]".to_string();
        options.css_public_base = Some("https://cdn.example.com/assets".to_string());
        let result = build(options).unwrap();

        let filename = &result.css_files[0].0;
        assert!(filename.starts_with("my-card-"));
        let expected_href = format!("https://cdn.example.com/assets/{filename}");
        assert_eq!(
            result
                .protocol
                .components
                .get("my-card")
                .map(|c| c.css_href.as_str()),
            Some(expected_href.as_str())
        );
    }

    #[test]
    fn test_css_public_base_emits_parser_and_handler_links() {
        let app = create_app_dir(&[
            (
                "index.html",
                "<html><head></head><body><my-card>Hello</my-card></body></html>",
            ),
            ("my-card.html", "<div><slot></slot></div>"),
            ("my-card.css", ".card { color: red; }"),
        ]);
        let mut options = default_options(app.path());
        options.css_file_name_template = "[name]-[hash].[ext]".to_string();
        options.css_public_base = Some("https://cdn.example.com/assets".to_string());
        let result = build(options).unwrap();

        let filename = &result.css_files[0].0;
        let expected_href = format!("https://cdn.example.com/assets/{filename}");
        let component_html = result.protocol.fragments["my-card"]
            .fragments
            .iter()
            .filter_map(|fragment| match fragment.fragment.as_ref() {
                Some(Fragment::Raw(raw)) => Some(raw.value.as_str()),
                _ => None,
            })
            .collect::<String>();
        assert!(
            component_html.contains(&format!(
                r#"<link rel="stylesheet" href="{expected_href}">"#
            )),
            "parser-generated component template should use CDN href: {component_html}"
        );

        let handler = WebUIHandler::new();
        let mut writer = StringWriter { buf: String::new() };
        handler
            .handle(
                &result.protocol,
                &serde_json::json!({}),
                &RenderOptions::new("index.html", "/"),
                &mut writer,
            )
            .unwrap();
        assert!(
            writer.buf.contains(&format!(
                r#"<link rel="preload" href="{expected_href}" as="style" data-webui-ssr-preload="style">"#
            )),
            "handler head preload should use CDN href: {}",
            writer.buf
        );
    }

    #[test]
    fn test_css_public_base_emits_webui_plugin_template_links() {
        let app = create_app_dir(&[
            ("index.html", "<my-card>Hello</my-card>"),
            ("my-card.html", "<div><slot></slot></div>"),
            ("my-card.css", ".card { color: red; }"),
            ("my-card.ts", "export {};"),
        ]);
        let mut options = default_options(app.path());
        options.plugin = Some(Plugin::WebUI);
        options.css_file_name_template = "[name]-[hash].[ext]".to_string();
        options.css_public_base = Some("https://cdn.example.com/assets".to_string());
        let result = build(options).unwrap();

        let filename = &result.css_files[0].0;
        let expected_href = format!("https://cdn.example.com/assets/{filename}");
        let template = &result.protocol.components["my-card"].template_json;
        assert!(
            template.contains(&format!(r#"href=\"{expected_href}\""#)),
            "plugin component template should use CDN href: {template}"
        );
    }

    #[test]
    fn test_css_public_base_emits_fast_plugin_template_links() {
        let app = create_app_dir(&[
            ("index.html", "<my-card>Hello</my-card>"),
            ("my-card.html", "<div><slot></slot></div>"),
            ("my-card.css", ".card { color: red; }"),
        ]);
        let mut options = default_options(app.path());
        options.plugin = Some(Plugin::FastV3);
        options.css_file_name_template = "[name]-[hash].[ext]".to_string();
        options.css_public_base = Some("https://cdn.example.com/assets".to_string());
        let result = build(options).unwrap();

        let filename = &result.css_files[0].0;
        let expected_href = format!("https://cdn.example.com/assets/{filename}");
        let template = &result.protocol.components["my-card"].template;
        assert!(
            template.contains(&format!(r#"href="{expected_href}""#)),
            "FAST component template should use CDN href: {template}"
        );
    }

    #[test]
    fn test_invalid_css_template_is_rejected() {
        let app = create_app_dir(&[
            ("index.html", "<my-card>Hello</my-card>"),
            ("my-card.html", "<div><slot></slot></div>"),
            ("my-card.css", ".card { color: red; }"),
        ]);
        let mut options = default_options(app.path());
        options.css_file_name_template = "[name]-[bogus].[ext]".to_string();
        let result = build(options);

        assert!(matches!(result, Err(WebUIError::InvalidBuildOptions(_))));
    }

    #[test]
    fn test_css_filename_collision_is_rejected() {
        let app = create_app_dir(&[
            ("index.html", "<card-a>A</card-a><card-b>B</card-b>"),
            ("card-a.html", "<div><slot></slot></div>"),
            ("card-a.css", ".x { color: red; }"),
            ("card-b.html", "<div><slot></slot></div>"),
            ("card-b.css", ".x { color: red; }"),
        ]);
        let mut options = default_options(app.path());
        options.css_file_name_template = "[hash].[ext]".to_string();
        let result = build(options);

        assert!(matches!(result, Err(WebUIError::InvalidBuildOptions(_))));
    }

    #[test]
    fn test_build_returns_static_component_asset_files() {
        let app = create_app_dir(&[
            ("index.html", "<app-shell></app-shell>"),
            ("app-shell.html", "<div></div>"),
            (
                "lazy-panel.html",
                r#"<if condition="ready"><p>{{title}}</p></if>"#,
            ),
            ("lazy-panel.ts", "export {};"),
        ]);
        let mut options = default_options(app.path());
        options.plugin = Some(Plugin::WebUI);
        options.component_asset_roots = vec!["lazy-panel".to_string()];

        let result = build(options).unwrap();

        assert_eq!(result.component_asset_files.len(), 1);
        assert_eq!(result.component_asset_files[0].name, "lazy-panel.webui.js");
        assert!(result.component_asset_files[0]
            .content
            .contains(r#""type":"webui-component-asset""#));
        assert!(result.component_asset_files[0]
            .content
            .contains(r#""templateFunctions":{"lazy-panel":"#));
        assert!(result.protocol.fragments.contains_key("lazy-panel"));
        assert!(
            !result
                .protocol
                .fragments
                .keys()
                .any(|key| key.starts_with("__webui_asset_root_")),
            "synthetic asset root fragments must not be serialized"
        );
    }

    #[test]
    fn test_component_asset_filename_collision_with_css_is_rejected() {
        let app = create_app_dir(&[
            ("index.html", "<app-shell></app-shell>"),
            ("app-shell.html", "<div></div>"),
            ("lazy-panel.html", "<p>Lazy</p>"),
            ("lazy-panel.css", ".panel { color: red; }"),
        ]);
        let mut options = default_options(app.path());
        options.plugin = Some(Plugin::WebUI);
        options.component_asset_roots = vec!["lazy-panel".to_string()];
        options.css_file_name_template = "[name]".to_string();

        let result = build(options);

        assert!(matches!(result, Err(WebUIError::InvalidBuildOptions(_))));
    }

    #[test]
    fn test_build_to_disk_rejects_protocol_name_collision_before_writing() {
        let app = create_app_dir(&[
            ("index.html", "<app-shell></app-shell>"),
            ("app-shell.html", "<div></div>"),
            ("lazy-panel.html", "<p>Lazy</p>"),
        ]);
        let out = TempDir::new().unwrap();
        let mut options = default_options(app.path());
        options.plugin = Some(Plugin::WebUI);
        options.component_asset_roots = vec!["lazy-panel".to_string()];
        options.css_file_name_template = "protocol.bin".to_string();

        let result = build_to_disk(options, out.path());

        assert!(matches!(result, Err(WebUIError::InvalidBuildOptions(_))));
        assert!(!out.path().join("protocol.bin").exists());
    }

    #[test]
    fn test_css_href_set_for_light_dom_link_strategy() {
        let app = create_app_dir(&[
            ("index.html", "<has-css>A</has-css><no-css>B</no-css>"),
            ("has-css.html", "<p><slot></slot></p>"),
            ("has-css.css", ".yes { color: green; }"),
            ("no-css.html", "<p><slot></slot></p>"),
        ]);
        let mut options = default_options(app.path());
        options.dom = DomStrategy::Light;
        let result = build(options).unwrap();

        let href = result
            .protocol
            .components
            .get("has-css")
            .map(|c| c.css_href.as_str())
            .unwrap_or("");
        assert_eq!(
            href, "has-css.css",
            "Light×Link component with CSS should have css_href"
        );

        let no_href = result
            .protocol
            .components
            .get("no-css")
            .map(|c| c.css_href.as_str())
            .unwrap_or("");
        assert!(
            no_href.is_empty(),
            "Component without CSS should have empty css_href"
        );
    }

    #[test]
    fn test_shadow_dom_link_strategy_sets_css_href() {
        // Shadow×Link: css_href is always set for Link-strategy components.
        // The handler uses protocol.css_strategy + dom_strategy to decide
        // whether to emit preload (Shadow) or stylesheet (Light) in <head>.
        let app = create_app_dir(&[
            ("index.html", "<my-card>A</my-card>"),
            ("my-card.html", "<p><slot></slot></p>"),
            ("my-card.css", ".card { color: red; }"),
        ]);
        let result = build(default_options(app.path())).unwrap();

        let comp = result.protocol.components.get("my-card").unwrap();
        assert_eq!(
            comp.css_href, "my-card.css",
            "Shadow×Link should set css_href"
        );

        // Strategy fields on protocol
        assert_eq!(
            result.protocol.css_strategy(),
            webui_protocol::CssStrategy::Link,
        );
        assert_eq!(
            result.protocol.dom_strategy(),
            webui_protocol::DomStrategy::Shadow,
        );

        // CSS file should still be emitted for the server to serve
        assert_eq!(
            result.css_files.len(),
            1,
            "CSS file should still be produced"
        );
    }

    #[test]
    fn test_css_href_empty_for_style_strategy() {
        let app = create_app_dir(&[
            ("index.html", "<my-card>A</my-card>"),
            ("my-card.html", "<p><slot></slot></p>"),
            ("my-card.css", ".card { color: red; }"),
        ]);
        let mut options = default_options(app.path());
        options.css = CssStrategy::Style;
        let result = build(options).unwrap();

        let href = result
            .protocol
            .components
            .get("my-card")
            .map(|c| c.css_href.as_str())
            .unwrap_or("");
        assert!(href.is_empty(), "Style-strategy should not set css_href");
    }

    #[test]
    fn test_css_href_empty_for_module_strategy() {
        let app = create_app_dir(&[
            ("index.html", "<my-card>A</my-card>"),
            ("my-card.html", "<p><slot></slot></p>"),
            ("my-card.css", ".card { color: red; }"),
        ]);
        let mut options = default_options(app.path());
        options.css = CssStrategy::Module;
        let result = build(options).unwrap();

        let href = result
            .protocol
            .components
            .get("my-card")
            .map(|c| c.css_href.as_str())
            .unwrap_or("");
        assert!(href.is_empty(), "Module-strategy should not set css_href");
        assert!(
            !result.protocol.components["my-card"].css.is_empty(),
            "Module-strategy should still populate css content"
        );
    }

    #[test]
    fn test_build_to_disk_writes_files() {
        let app = create_app_dir(&[
            ("index.html", "<my-card>Hello</my-card>"),
            ("my-card.html", "<div><slot></slot></div>"),
            ("my-card.css", ".card { color: red; }"),
            ("my-card.ts", "export {};"),
        ]);
        let out = TempDir::new().unwrap();

        let stats = build_to_disk(default_options(app.path()), out.path()).unwrap();

        assert!(out.path().join("protocol.bin").exists());
        assert!(out.path().join("my-card.css").exists());
        assert_eq!(stats.css_file_count, 1);
        assert!(stats.fragment_count > 0);
    }

    #[test]
    fn test_build_to_disk_creates_nested_output_dir() {
        let app = create_app_dir(&[("index.html", "<h1>Hello</h1>")]);
        let out = TempDir::new().unwrap();
        let nested = out.path().join("nested").join("output");

        let stats = build_to_disk(default_options(app.path()), &nested).unwrap();
        assert!(nested.join("protocol.bin").exists());
        assert!(stats.fragment_count > 0);
    }

    #[test]
    fn test_build_missing_entry() {
        let app = create_app_dir(&[("other.html", "<h1>Not index</h1>")]);
        let result = build(default_options(app.path()));

        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(matches!(err, WebUIError::Io { .. }));
    }

    #[test]
    fn test_build_stats_populated() {
        let app = create_app_dir(&[("index.html", "<h1>{{title}}</h1>")]);
        let result = build(default_options(app.path())).unwrap();

        assert!(result.stats.fragment_count > 0);
        assert!(result.stats.protocol_size_bytes > 0);
        assert_eq!(result.stats.css_file_count, 0);
    }

    #[test]
    fn test_build_inline_css() {
        let app = create_app_dir(&[
            ("index.html", "<my-card>Hello</my-card>"),
            ("my-card.html", "<div><slot></slot></div>"),
            ("my-card.css", ".card { color: red; }"),
        ]);
        let mut options = default_options(app.path());
        options.css = CssStrategy::Style;

        let result = build(options).unwrap();
        // Inline mode embeds CSS in <style> tags — no external CSS files
        assert!(result.css_files.is_empty());
        assert_eq!(result.stats.css_file_count, 0);
        assert!(result.stats.fragment_count > 0);
    }

    #[test]
    fn test_build_module_css() {
        let app = create_app_dir(&[
            ("index.html", "<my-card>Hello</my-card>"),
            ("my-card.html", "<div><slot></slot></div>"),
            ("my-card.css", ".card { color: red; }"),
        ]);
        let mut options = default_options(app.path());
        options.css = CssStrategy::Module;

        let result = build(options).unwrap();
        // Module mode emits <script type="importmap"> data URIs — no external CSS files
        assert!(result.css_files.is_empty());
        assert_eq!(result.stats.css_file_count, 0);
        assert!(result.stats.fragment_count > 0);
    }

    #[test]
    fn test_build_custom_entry() {
        let app = create_app_dir(&[("page.html", "<h1>Custom</h1>")]);
        let mut options = default_options(app.path());
        options.entry = "page.html".to_string();

        let result = build(options).unwrap();
        assert!(result.protocol.fragments.contains_key("page.html"));
        assert!(!result.protocol.fragments.contains_key("index.html"));
    }

    #[test]
    fn test_build_protocol_roundtrip() {
        let app = create_app_dir(&[("index.html", "<h1>{{title}}</h1>")]);
        let result = build(default_options(app.path())).unwrap();

        let restored = WebUIProtocol::from_protobuf(&result.protocol_bytes).unwrap();
        assert!(restored.fragments.contains_key("index.html"));
    }

    #[test]
    fn prepared_protocol_is_available_from_facade() {
        let prepared = PreparedProtocol::new(WebUIProtocol::new(HashMap::new()));
        assert!(prepared.protocol().fragments.is_empty());
    }

    #[test]
    fn test_inspect_bytes_valid() {
        let app = create_app_dir(&[("index.html", "<h1>Hello</h1>")]);
        let result = build(default_options(app.path())).unwrap();

        let json_str = inspect_bytes(&result.protocol_bytes).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&json_str).unwrap();
        assert!(parsed.get("fragments").is_some());
    }

    #[test]
    fn test_inspect_bytes_invalid() {
        let result = inspect_bytes(b"not a protobuf");
        assert!(result.is_err());
        assert!(matches!(result.unwrap_err(), WebUIError::Protocol(_)));
    }

    #[test]
    fn test_inspect_file() {
        let app = create_app_dir(&[("index.html", "<h1>Hello</h1>")]);
        let out = TempDir::new().unwrap();
        build_to_disk(default_options(app.path()), out.path()).unwrap();

        let json_str = inspect(&out.path().join("protocol.bin")).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&json_str).unwrap();
        assert!(parsed["fragments"]["index.html"]["fragments"].is_array());
    }

    #[test]
    fn test_inspect_missing_file() {
        let result = inspect(Path::new("/nonexistent/protocol.bin"));
        assert!(result.is_err());
        assert!(matches!(result.unwrap_err(), WebUIError::Io { .. }));
    }

    #[test]
    fn test_build_with_components_local_path() {
        let app = create_app_dir(&[("index.html", "<ext-card>Hello</ext-card>")]);
        let ext_dir = TempDir::new().unwrap();
        fs::write(
            ext_dir.path().join("ext-card.html"),
            "<div class=\"card\"><slot></slot></div>",
        )
        .unwrap();
        fs::write(
            ext_dir.path().join("ext-card.css"),
            ".card { border: 1px solid #ccc; }",
        )
        .unwrap();

        let mut options = default_options(app.path());
        options.components = vec![ext_dir.path().to_string_lossy().to_string()];

        let result = build(options).unwrap();
        assert!(result.protocol.fragments.contains_key("index.html"));
        assert_eq!(result.css_files.len(), 1);
        assert!(result.css_files[0].1.contains("border"));
    }

    #[test]
    fn test_build_with_scriptless_component_path_emits_dormant_template() {
        let app = create_app_dir(&[("index.html", "<ext-card></ext-card>")]);
        let ext_dir = TempDir::new().unwrap();
        fs::write(ext_dir.path().join("ext-card.html"), "<p>{{title}}</p>").unwrap();

        let mut options = default_options(app.path());
        options.plugin = Some(Plugin::WebUI);
        options.components = vec![ext_dir.path().to_string_lossy().to_string()];

        let result = build(options).unwrap();
        assert!(result
            .component_templates
            .iter()
            .any(|template| template.tag_name == "ext-card"));
        let component = result
            .protocol
            .components
            .get("ext-card")
            .expect("external scriptless component metadata should be retained");
        assert!(component.hydration_keys.is_empty());
        assert_eq!(component.navigation_keys, ["title"]);
        assert!(component.template_json.contains(r#""th":1"#));
    }

    #[test]
    fn test_build_with_scripted_component_path_emits_client_artifact() {
        let app = create_app_dir(&[("index.html", "<ext-card></ext-card>")]);
        let ext_dir = TempDir::new().unwrap();
        fs::write(ext_dir.path().join("ext-card.html"), "<p>{{title}}</p>").unwrap();
        fs::write(ext_dir.path().join("ext-card.ts"), "export {};").unwrap();

        let mut options = default_options(app.path());
        options.plugin = Some(Plugin::WebUI);
        options.components = vec![ext_dir.path().to_string_lossy().to_string()];

        let result = build(options).unwrap();
        let template = &result.protocol.components["ext-card"].template_json;
        assert!(!template.is_empty(), "template should be emitted");
    }

    #[test]
    fn test_build_rejects_projection_manifest_with_non_webui_plugin() {
        let app = create_app_dir(&[
            ("index.html", "<demo-card></demo-card>"),
            ("demo-card.html", "<p>{{name}}</p>"),
            ("demo-card.ts", "export {};"),
        ]);
        let manifest_json = projection::test_support::build_valid_manifest_json(
            app.path(),
            &[("demo-card.ts", "export {};")],
            &[("projection-bundle.js", "output")],
            &[(
                "demo-card",
                "demo-card.ts",
                &["projection-bundle.js"],
                &["name"],
                &["name"],
            )],
        );
        let manifest = projection::test_support::write_manifest(
            app.path(),
            "webui-projection.json",
            &manifest_json,
        );
        let mut options = default_options(app.path());
        // No plugin selected: default/FAST builds are incompatible with
        // projection manifests (`PROJ-B002`).
        options.projection_manifests = vec![manifest.into()];
        let err = build(options).expect_err("non-WebUI plugin + manifest should be rejected");
        assert!(
            err.chain_message().contains("PROJ-B002"),
            "msg: {}",
            err.chain_message()
        );
    }

    #[test]
    fn test_build_rejects_missing_scripted_component_coverage() {
        let app = create_app_dir(&[
            ("index.html", "<demo-card></demo-card>"),
            ("demo-card.html", "<p>{{name}}</p>"),
            ("demo-card.ts", "export {};"),
        ]);
        // Manifest is valid but never mentions `demo-card`, the only
        // compiled scripted component in this build.
        let manifest_json =
            projection::test_support::build_valid_manifest_json(app.path(), &[], &[], &[]);
        let manifest = projection::test_support::write_manifest(
            app.path(),
            "webui-projection.json",
            &manifest_json,
        );
        let mut options = default_options(app.path());
        options.plugin = Some(Plugin::WebUI);
        options.projection_manifests = vec![manifest.into()];
        let err = build(options).expect_err("missing coverage should fail the build");
        let message = err.chain_message();
        assert!(message.contains("PROJ-B001"), "msg: {message}");
        assert!(message.contains("demo-card"), "msg: {message}");
    }

    #[test]
    fn test_build_allows_unused_discovered_scripted_component_without_coverage() {
        let app = create_app_dir(&[
            ("index.html", "<demo-card></demo-card>"),
            ("demo-card.html", "<p>{{name}}</p>"),
            ("demo-card.ts", "export {};"),
            // `unused-card` is a scripted component discovered on disk but
            // never referenced from `index.html`, so it is not compiled
            // into the protocol and does not require manifest coverage.
            ("unused-card.html", "<p>{{title}}</p>"),
            ("unused-card.ts", "export {};"),
        ]);
        let manifest_json = projection::test_support::build_valid_manifest_json(
            app.path(),
            &[("demo-card.ts", "export {};")],
            &[("projection-bundle.js", "output")],
            &[(
                "demo-card",
                "demo-card.ts",
                &["projection-bundle.js"],
                &["name"],
                &["name"],
            )],
        );
        let manifest = projection::test_support::write_manifest(
            app.path(),
            "webui-projection.json",
            &manifest_json,
        );
        let mut options = default_options(app.path());
        options.plugin = Some(Plugin::WebUI);
        options.projection_manifests = vec![manifest.into()];
        let result = build(options).unwrap();

        assert!(!result.protocol.fragments.contains_key("unused-card"));
        let component = result.protocol.components.get("demo-card").unwrap();
        assert_eq!(component.hydration_keys, vec!["name"]);
    }

    #[test]
    fn test_build_merges_multiple_projection_manifest_fragments() {
        let app = create_app_dir(&[
            (
                "index.html",
                "<html><body><card-a></card-a><card-b></card-b></body></html>",
            ),
            ("card-a.html", "<p>{{alpha}}</p>"),
            ("card-a.ts", "export {};"),
            ("card-b.html", "<p>{{beta}}</p>"),
            ("card-b.ts", "export {};"),
        ]);
        let json_a = projection::test_support::build_valid_manifest_json(
            app.path(),
            &[("card-a.ts", "export {};")],
            &[("card-a.js", "output-a")],
            &[(
                "card-a",
                "card-a.ts",
                &["card-a.js"],
                &["alpha"],
                &["alpha"],
            )],
        );
        let manifest_a =
            projection::test_support::write_manifest(app.path(), "fragment-a.json", &json_a);
        let json_b = projection::test_support::build_valid_manifest_json(
            app.path(),
            &[("card-b.ts", "export {};")],
            &[("card-b.js", "output-b")],
            &[("card-b", "card-b.ts", &["card-b.js"], &["beta"], &["beta"])],
        );
        let manifest_b =
            projection::test_support::write_manifest(app.path(), "fragment-b.json", &json_b);

        let mut options = default_options(app.path());
        options.plugin = Some(Plugin::WebUI);
        options.projection_manifests = vec![manifest_a.into(), manifest_b.into()];
        let result = build(options).unwrap();

        assert_eq!(
            result.protocol.initial_state_strategy,
            webui_protocol::InitialStateStrategy::Components as i32
        );
        let card_a = result.protocol.components.get("card-a").unwrap();
        assert_eq!(card_a.hydration_keys, vec!["alpha"]);
        let card_b = result.protocol.components.get("card-b").unwrap();
        assert_eq!(card_b.hydration_keys, vec!["beta"]);
    }

    #[test]
    fn test_build_hello_world_example() {
        let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let app_dir = manifest_dir.join("../../examples/app/hello-world/src");

        let result = build(BuildOptions {
            app_dir,
            entry: "index.html".to_string(),
            ..BuildOptions::default()
        })
        .unwrap();

        let index = &result.protocol.fragments["index.html"].fragments;
        assert!(index.iter().any(
            |f| matches!(f.fragment.as_ref(), Some(Fragment::ForLoop(fl)) if fl.collection == "people")
        ));
        assert!(index
            .iter()
            .any(|f| matches!(f.fragment.as_ref(), Some(Fragment::IfCond(_)))));
    }

    // ── Security tests ───────────────────────────────────────────────

    #[test]
    fn test_css_filename_sanitizes_path_separators() {
        // Even if a component tag somehow contains path separators,
        // the filename should be sanitized to prevent directory traversal.
        let app = create_app_dir(&[("index.html", "<h1>Hello</h1>")]);
        let result = build(default_options(app.path())).unwrap();

        // Verify all CSS filenames are plain file names (no path separators)
        for (filename, _) in &result.css_files {
            assert!(
                !filename.contains('/') && !filename.contains('\\'),
                "CSS filename contains path separator: {filename}"
            );
        }
    }

    #[test]
    fn test_build_to_disk_css_stays_in_output_dir() {
        let app = create_app_dir(&[
            ("index.html", "<my-card>Hello</my-card>"),
            ("my-card.html", "<div><slot></slot></div>"),
            ("my-card.css", ".card { color: red; }"),
        ]);
        let out = TempDir::new().unwrap();

        build_to_disk(default_options(app.path()), out.path()).unwrap();

        // Verify CSS file is written inside out_dir
        let css_path = out.path().join("my-card.css");
        assert!(css_path.exists());
        let canonical = css_path.canonicalize().unwrap();
        let out_canonical = out.path().canonicalize().unwrap();
        assert!(
            canonical.starts_with(&out_canonical),
            "CSS file escaped output directory"
        );
    }

    // ── Performance / edge case tests ────────────────────────────────

    #[test]
    fn test_build_empty_html() {
        let app = create_app_dir(&[("index.html", "")]);
        let result = build(default_options(app.path())).unwrap();

        assert!(result.protocol.fragments.contains_key("index.html"));
        assert_eq!(result.stats.fragment_count, 0);
        assert!(result.stats.protocol_size_bytes > 0);
    }

    #[test]
    fn test_build_large_fragment_count() {
        // Verify stats are accurate with many fragments
        let mut html = String::with_capacity(2000);
        for i in 0..50 {
            html.push_str(&format!("<p>Item {i}</p>\n"));
        }
        let app = create_app_dir(&[("index.html", &html)]);
        let result = build(default_options(app.path())).unwrap();

        assert!(result.stats.fragment_count > 0);
        assert_eq!(result.stats.css_file_count, 0);
    }

    #[test]
    fn test_build_with_fast_plugin() {
        let app = create_app_dir(&[("index.html", "<h1>Hello</h1>")]);
        let mut options = default_options(app.path());
        options.plugin = Some(Plugin::FastV3);

        let result = build(options).unwrap();
        assert!(result.protocol.fragments.contains_key("index.html"));
        assert_eq!(
            result.protocol.initial_state_strategy,
            webui_protocol::InitialStateStrategy::Full as i32
        );
    }

    #[test]
    fn test_build_to_disk_inline_mode_no_css_files() {
        let app = create_app_dir(&[
            ("index.html", "<my-card>Hello</my-card>"),
            ("my-card.html", "<div><slot></slot></div>"),
            ("my-card.css", ".card { color: red; }"),
        ]);
        let out = TempDir::new().unwrap();
        let mut options = default_options(app.path());
        options.css = CssStrategy::Style;

        let stats = build_to_disk(options, out.path()).unwrap();

        assert!(out.path().join("protocol.bin").exists());
        assert!(!out.path().join("my-card.css").exists());
        assert_eq!(stats.css_file_count, 0);
    }

    #[test]
    fn test_build_stats_duration_is_nonzero() {
        let app = create_app_dir(&[("index.html", "<h1>Hello {{name}}</h1>")]);
        let result = build(default_options(app.path())).unwrap();

        assert!(!result.stats.duration.is_zero());
    }

    #[test]
    fn test_build_multiple_components_css() {
        let app = create_app_dir(&[
            ("index.html", "<card-a>A</card-a><card-b>B</card-b>"),
            ("card-a.html", "<div><slot></slot></div>"),
            ("card-a.css", ".a { color: red; }"),
            ("card-b.html", "<span><slot></slot></span>"),
            ("card-b.css", ".b { color: blue; }"),
        ]);
        let result = build(default_options(app.path())).unwrap();

        assert_eq!(result.css_files.len(), 2);
        assert_eq!(result.stats.css_file_count, 2);
        let filenames: Vec<&str> = result.css_files.iter().map(|(n, _)| n.as_str()).collect();
        assert!(filenames.contains(&"card-a.css"));
        assert!(filenames.contains(&"card-b.css"));
    }

    #[test]
    fn test_build_unused_component_css_not_emitted() {
        // card-b is registered but not referenced in index.html
        let app = create_app_dir(&[
            ("index.html", "<card-a>A</card-a>"),
            ("card-a.html", "<div><slot></slot></div>"),
            ("card-a.css", ".a { color: red; }"),
            ("card-b.html", "<span><slot></slot></span>"),
            ("card-b.css", ".b { color: blue; }"),
        ]);
        let result = build(default_options(app.path())).unwrap();

        assert_eq!(result.css_files.len(), 1);
        assert_eq!(result.css_files[0].0, "card-a.css");
    }

    #[test]
    fn test_inspect_roundtrip_preserves_content() {
        let app = create_app_dir(&[("index.html", "<h1>{{title}}</h1>")]);
        let result = build(default_options(app.path())).unwrap();

        let json_str = inspect_bytes(&result.protocol_bytes).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&json_str).unwrap();

        let fragments = &parsed["fragments"]["index.html"]["fragments"];
        assert!(fragments.is_array());
        assert!(!fragments.as_array().unwrap().is_empty());
    }

    #[test]
    fn test_build_nonexistent_app_dir() {
        let options = BuildOptions {
            app_dir: PathBuf::from("/nonexistent/path/that/does/not/exist"),
            ..BuildOptions::default()
        };
        let result = build(options);
        assert!(result.is_err());
    }

    #[test]
    fn test_build_allows_fallback_chain_when_all_theme_tokens_exist() {
        let app = create_app_dir(&[
            ("index.html", "<my-card></my-card>"),
            ("my-card.html", "<div>Card</div>"),
            (
                "my-card.css",
                ":host { color: var(--token-a, var(--token-b, var(--token-c))); }",
            ),
        ]);
        let mut options = default_options(app.path());
        options.theme = Some(TokenFile {
            themes: HashMap::from([(
                "light".to_string(),
                HashMap::from([
                    ("token-a".to_string(), "red".to_string()),
                    ("token-b".to_string(), "blue".to_string()),
                    ("token-c".to_string(), "green".to_string()),
                ]),
            )]),
        });

        let result = build(options).expect("all fallback tokens are defined");
        assert!(result.protocol.tokens.contains(&"token-a".to_string()));
        assert!(result.protocol.tokens.contains(&"token-b".to_string()));
        assert!(result.protocol.tokens.contains(&"token-c".to_string()));
    }

    #[test]
    fn test_build_allows_literal_fallback_token_absent_from_theme() {
        let app = create_app_dir(&[
            ("index.html", "<my-card></my-card>"),
            ("my-card.html", "<div>Card</div>"),
            ("my-card.css", ":host { color: var(--brand, #000); }"),
        ]);
        let mut options = default_options(app.path());
        options.theme = Some(TokenFile {
            themes: HashMap::from([("light".to_string(), HashMap::new())]),
        });

        // The CSS literal fallback (`#000`) means `--brand` is not required from
        // the theme, so the build must succeed — but the token is still hoisted
        // so the runtime resolves it when a theme provides it.
        let result = build(options).expect("literal fallback should not fail the build");
        assert!(result.protocol.tokens.contains(&"brand".to_string()));
        // `--brand` is absent from every theme and only used with a literal
        // fallback, so it is surfaced as a non-fatal typo advisory.
        assert_eq!(result.warnings.len(), 1, "warnings: {:?}", result.warnings);
        assert!(result.warnings[0].body().contains("--brand"));
    }

    #[test]
    fn test_build_ancestor_inline_style_definition_after_comment_satisfies_descendant() {
        // `--foo-bar` is defined in the entry's inline <style> after a signal
        // comment and consumed inside a component. Custom properties inherit
        // through Shadow DOM, so the ancestor definition must satisfy the
        // descendant and the token must not be required from the theme.
        let app = create_app_dir(&[
            (
                "index.html",
                "<html><head><style>:root {\n  /*{{{tokens.light}}}*/\n  --foo-bar: 100px;\n}</style></head><body><my-card></my-card></body></html>",
            ),
            ("my-card.html", "<div>Card</div>"),
            ("my-card.css", ":host { padding: var(--foo-bar); }"),
        ]);
        let mut options = default_options(app.path());
        options.theme = Some(TokenFile {
            themes: HashMap::from([("light".to_string(), HashMap::new())]),
        });

        let result = build(options)
            .expect("ancestor inline-style definition (after a comment) should satisfy the child");
        assert!(
            !result.protocol.tokens.contains(&"foo-bar".to_string()),
            "ancestor-defined token should not be hoisted: {:?}",
            result.protocol.tokens
        );
    }

    #[test]
    fn test_build_no_warning_when_literal_fallback_token_is_themed() {
        let app = create_app_dir(&[
            ("index.html", "<my-card></my-card>"),
            ("my-card.html", "<div>Card</div>"),
            ("my-card.css", ":host { color: var(--brand, #000); }"),
        ]);
        let mut options = default_options(app.path());
        options.theme = Some(TokenFile {
            themes: HashMap::from([(
                "light".to_string(),
                HashMap::from([("brand".to_string(), "#123456".to_string())]),
            )]),
        });

        let result = build(options).expect("themed literal-fallback token builds");
        assert!(
            result.warnings.is_empty(),
            "a themed token must not warn: {:?}",
            result.warnings
        );
    }

    #[test]
    fn test_build_without_theme_produces_no_warnings() {
        let app = create_app_dir(&[
            ("index.html", "<my-card></my-card>"),
            ("my-card.html", "<div>Card</div>"),
            ("my-card.css", ":host { color: var(--brand, #000); }"),
        ]);
        let result = build(default_options(app.path())).unwrap();
        assert!(result.warnings.is_empty());
    }

    #[test]
    fn test_build_excludes_tokens_defined_by_ancestor_component_css() {
        let app = create_app_dir(&[
            ("index.html", "<component-a></component-a>"),
            ("component-a.html", "<component-b></component-b>"),
            ("component-a.css", ":host { --brand-color: red; }"),
            ("component-b.html", "<div>Child</div>"),
            ("component-b.css", ":host { color: var(--brand-color); }"),
        ]);
        let mut options = default_options(app.path());
        options.theme = Some(TokenFile {
            themes: HashMap::from([("light".to_string(), HashMap::new())]),
        });

        let result =
            build(options).expect("ancestor component custom property should satisfy child");
        assert!(
            !result.protocol.tokens.contains(&"brand-color".to_string()),
            "ancestor-defined token should not be exposed: {:?}",
            result.protocol.tokens
        );
    }

    #[test]
    fn test_build_does_not_use_sibling_component_definitions_for_theme_validation() {
        let app = create_app_dir(&[
            (
                "index.html",
                "<component-a></component-a><component-b></component-b>",
            ),
            ("component-a.html", "<div>A</div>"),
            ("component-a.css", ":host { --brand-color: red; }"),
            ("component-b.html", "<div>B</div>"),
            ("component-b.css", ":host { color: var(--brand-color); }"),
        ]);
        let mut options = default_options(app.path());
        options.theme = Some(TokenFile {
            themes: HashMap::from([("light".to_string(), HashMap::new())]),
        });

        let Err(err) = build(options) else {
            panic!("sibling custom property must not satisfy component-b");
        };
        let message = err.chain_message();
        assert!(message.contains("missing theme token"), "msg: {message}");
        assert!(message.contains("missing-theme-token"), "msg: {message}");
        assert!(message.contains("--brand-color"), "msg: {message}");
    }

    #[test]
    fn test_build_rejects_missing_theme_token_after_local_definition_filter() {
        let app = create_app_dir(&[
            ("index.html", "<my-card></my-card>"),
            ("my-card.html", "<div>Card</div>"),
            (
                "my-card.css",
                ":host { --token-a: red; --foo-bar: var(--token-a, var(--token-b, var(--token-c))); }",
            ),
        ]);
        let mut options = default_options(app.path());
        options.theme = Some(TokenFile {
            themes: HashMap::from([(
                "light".to_string(),
                HashMap::from([("token-b".to_string(), "green".to_string())]),
            )]),
        });

        let Err(err) = build(options) else {
            panic!("missing --token-c in the theme must fail the build");
        };
        let message = err.chain_message();
        assert!(message.contains("missing-theme-token"), "msg: {message}");
        assert!(message.contains("--token-c"), "msg: {message}");
        // The error demands the missing `--token-c`, never the locally-defined
        // `--token-a` (which only appears in the source snippet as context).
        assert!(!message.contains("add --token-a"), "msg: {message}");
    }

    #[test]
    fn test_build_to_disk_returns_accurate_stats() {
        let app = create_app_dir(&[
            ("index.html", "<my-card>Hello</my-card><p>{{name}}</p>"),
            ("my-card.html", "<div><slot></slot></div>"),
            ("my-card.css", ".card { color: red; }"),
        ]);
        let out = TempDir::new().unwrap();

        let stats = build_to_disk(default_options(app.path()), out.path()).unwrap();

        assert!(stats.fragment_count > 0);
        assert_eq!(stats.css_file_count, 1);
        assert!(stats.component_count > 0);
        assert!(stats.protocol_size_bytes > 0);
        assert!(!stats.duration.is_zero());
    }
}

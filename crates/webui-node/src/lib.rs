// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

//! Node.js native addon for the WebUI framework via napi-rs.
//!
//! Provides high-performance server-side rendering by compiling the Rust
//! WebUI handler directly into a `.node` native addon — no C ABI intermediary.
//!
//! The `Protocol` class decodes pre-compiled protobuf data from `webui build`
//! once and provides buffered and streaming render methods.
//!
//! ## Usage (from Node.js)
//!
//! ```js
//! import fs from 'node:fs';
//!
//! // Load the native addon
//! const mod = { exports: {} };
//! process.dlopen(mod, './target/release/libwebui_node.dylib');
//! const { Protocol } = mod.exports;
//!
//! // Read pre-compiled protocol (from `webui build`)
//! const protocol = new Protocol(fs.readFileSync('./dist/protocol.bin'), 'webui');
//! const state = '{"name": "WebUI"}';
//!
//! // Stream rendered fragments
//! protocol.renderStream(state, 'index.html', '/', (chunk) => process.stdout.write(chunk));
//! ```

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use napi::bindgen_prelude::{Buffer, Either, External, Function};
use napi::Error as NapiError;
use napi_derive::napi;
use serde_json::Value;
use webui_handler::plugin::fast_v2::FastV2HydrationPlugin;
use webui_handler::plugin::fast_v3::FastV3HydrationPlugin;
use webui_handler::plugin::webui::WebUIHydrationPlugin;
use webui_handler::{
    BoundaryDescriptor, BoundaryInstanceId, BoundaryKey, BoundaryMode, HandlerError,
    Protocol as HandlerProtocol, RenderOptions, ResponseWriter, SessionOptions,
    StreamStep as HandlerStreamStep, StreamingSession as HandlerStreamingSession, WebUIHandler,
};
#[cfg(test)]
use webui_protocol::WebUIProtocol;

const STREAM_CHUNK_SIZE: usize = 16 * 1024;
const INITIAL_RENDER_CAPACITY: usize = 4 * 1024;
const MAX_RENDER_CAPACITY_HINT: usize = 1024 * 1024;
const RENDER_CAPACITY_BUCKETS: usize = 64;
const RENDER_CAPACITY_MASK: usize = (1 << 21) - 1;
const _: () = assert!(RENDER_CAPACITY_BUCKETS.is_power_of_two());
const _: () = assert!(MAX_RENDER_CAPACITY_HINT <= RENDER_CAPACITY_MASK);

#[cfg(target_pointer_width = "64")]
const CAPACITY_HASH_OFFSET: usize = 14_695_981_039_346_656_037;
#[cfg(target_pointer_width = "64")]
const CAPACITY_HASH_PRIME: usize = 1_099_511_628_211;
#[cfg(target_pointer_width = "32")]
const CAPACITY_HASH_OFFSET: usize = 2_166_136_261;
#[cfg(target_pointer_width = "32")]
const CAPACITY_HASH_PRIME: usize = 16_777_619;

// A fixed-size direct-mapped cache keeps memory bounded. Each atomic packs the
// capacity with a fingerprint, so bucket collisions become misses rather than
// cross-route over-allocation. Races affect only an advisory allocation size.
struct RenderCapacityHints {
    buckets: [AtomicUsize; RENDER_CAPACITY_BUCKETS],
}

impl RenderCapacityHints {
    fn new() -> Self {
        Self {
            buckets: std::array::from_fn(|_| AtomicUsize::new(0)),
        }
    }

    fn load(&self, entry_id: &str, request_path: &str) -> usize {
        let hash = capacity_hint_hash(entry_id, request_path);
        let encoded = self.buckets[hash & (RENDER_CAPACITY_BUCKETS - 1)].load(Ordering::Relaxed);
        if encoded & !RENDER_CAPACITY_MASK == capacity_hint_fingerprint(hash) {
            encoded & RENDER_CAPACITY_MASK
        } else {
            INITIAL_RENDER_CAPACITY
        }
    }

    fn store(&self, entry_id: &str, request_path: &str, capacity: usize) {
        let hash = capacity_hint_hash(entry_id, request_path);
        let encoded = capacity_hint_fingerprint(hash)
            | capacity.clamp(INITIAL_RENDER_CAPACITY, MAX_RENDER_CAPACITY_HINT);
        self.buckets[hash & (RENDER_CAPACITY_BUCKETS - 1)].store(encoded, Ordering::Relaxed);
    }
}

fn capacity_hint_hash(entry_id: &str, request_path: &str) -> usize {
    let mut hash = CAPACITY_HASH_OFFSET;
    for byte in entry_id
        .bytes()
        .chain(std::iter::once(u8::MAX))
        .chain(request_path.bytes())
    {
        hash ^= usize::from(byte);
        hash = hash.wrapping_mul(CAPACITY_HASH_PRIME);
    }
    hash
}

fn capacity_hint_fingerprint(hash: usize) -> usize {
    let fingerprint = hash.rotate_left(7) & !RENDER_CAPACITY_MASK;
    if fingerprint == 0 {
        RENDER_CAPACITY_MASK + 1
    } else {
        fingerprint
    }
}

/// Build statistics returned from the build function.
#[napi(object)]
pub struct JsBuildStats {
    /// Build duration in milliseconds.
    pub duration_ms: f64,
    /// Total number of protocol fragments.
    pub fragment_count: u32,
    /// Number of registered components.
    pub component_count: u32,
    /// Number of CSS files produced.
    pub css_file_count: u32,
    /// Size of the serialized protocol in bytes.
    pub protocol_size_bytes: u32,
    /// Number of unique CSS tokens discovered.
    pub token_count: u32,
}

/// Result of a successful build operation.
#[napi(object)]
pub struct JsBuildResult {
    /// Serialized protocol (protobuf binary).
    pub protocol: Buffer,
    /// CSS files as alternating [filename, content, filename, content, ...].
    pub css_files: Vec<String>,
    /// Static component asset files as alternating [filename, content, filename, content, ...].
    pub component_asset_files: Vec<String>,
    /// Esbuild-compatible component asset metafile JSON when requested.
    pub metafile: Option<String>,
    /// Non-fatal build advisories (plain text), e.g. CSS tokens used only with a
    /// literal `var()` fallback and absent from every theme.
    pub warnings: Vec<String>,
    /// Build statistics.
    pub stats: JsBuildStats,
}

/// Inline projection manifest transported through N-API.
#[napi(object)]
pub struct JsProjectionManifest {
    /// Logical manifest path used to resolve `root` and stale file checks.
    pub path: String,
    /// Canonical manifest JSON.
    pub json: String,
}

/// Build options for the webui build API.
#[napi(object)]
pub struct JsBuildOptions {
    /// Path to the application folder containing templates.
    pub app_dir: String,
    /// Entry HTML file name (defaults to "index.html").
    pub entry: Option<String>,
    /// CSS mode: "link" (default), "style", or "module".
    pub css: Option<String>,
    /// Fallback DOM strategy: "shadow" (default) or "light".
    pub dom: Option<String>,
    /// Merge component stylesheets into shared bundled chunks.
    ///
    /// Composes with `css`: bundling decides how stylesheets are grouped, `css`
    /// decides how they reach the page.
    pub css_bundle: Option<bool>,
    /// Plugin identifier (see crate documentation for available identifiers).
    pub plugin: Option<String>,
    /// Additional component sources (npm packages or local paths).
    pub components: Option<Vec<String>>,
    /// Root component tags emitted as static `.webui.js` ESM assets.
    pub component_asset_roots: Option<Vec<String>>,
    /// Generate and return an esbuild-compatible component asset metafile.
    pub metafile: Option<bool>,
    /// Link-mode CSS filename template using [name], [hash], [ext].
    pub css_file_name_template: Option<String>,
    /// Optional base URL/path prefix for Link-mode css hrefs.
    pub css_public_base: Option<String>,
    /// Legal comment handling: "inline" (default) or "none".
    pub legal_comments: Option<String>,
    /// Design token theme: a JSON file path or npm package name.
    pub theme: Option<String>,
    /// Projection manifest paths.
    pub projection_manifests: Option<Vec<String>>,
    /// Inline manifest objects with their logical paths.
    pub projection_manifest_objects: Option<Vec<JsProjectionManifest>>,
}

/// Build a WebUI application from an app directory.
///
/// Returns the compiled protocol bytes, CSS files, and build statistics.
#[napi]
#[allow(clippy::cast_possible_truncation)] // stats are bounded by component/file counts
pub fn build(options: JsBuildOptions) -> napi::Result<JsBuildResult> {
    let css = options
        .css
        .map(|s| s.parse::<webui::CssStrategy>())
        .transpose()
        .map_err(NapiError::from_reason)?
        .unwrap_or_default();
    let dom = options
        .dom
        .map(|value| value.parse::<webui::DomStrategy>())
        .transpose()
        .map_err(NapiError::from_reason)?
        .unwrap_or_default();

    let plugin = options
        .plugin
        .map(|s| s.parse::<webui::Plugin>())
        .transpose()
        .map_err(NapiError::from_reason)?;

    let legal_comments = options
        .legal_comments
        .map(|s| s.parse::<webui::LegalComments>())
        .transpose()
        .map_err(NapiError::from_reason)?
        .unwrap_or_default();

    let app_dir = std::path::PathBuf::from(&options.app_dir);
    let theme = options
        .theme
        .as_deref()
        .map(|theme| load_theme(theme, &app_dir))
        .transpose()?;
    let mut projection_manifests: Vec<webui::ProjectionManifestSource> = options
        .projection_manifests
        .unwrap_or_default()
        .into_iter()
        .map(std::path::PathBuf::from)
        .map(Into::into)
        .collect();
    projection_manifests.extend(
        options
            .projection_manifest_objects
            .unwrap_or_default()
            .into_iter()
            .map(|manifest| webui::ProjectionManifestSource::Inline {
                manifest_path: std::path::PathBuf::from(manifest.path),
                json: manifest.json,
            }),
    );

    let build_options = webui::BuildOptions {
        app_dir,
        entry: options.entry.unwrap_or_else(|| "index.html".to_string()),
        css,
        dom,
        css_bundle: options.css_bundle.unwrap_or(false),
        plugin,
        components: options.components.unwrap_or_default(),
        component_asset_roots: options.component_asset_roots.unwrap_or_default(),
        metafile: options.metafile.unwrap_or(false),
        css_file_name_template: options
            .css_file_name_template
            .unwrap_or_else(|| webui::DEFAULT_CSS_FILE_NAME_TEMPLATE.to_string()),
        css_public_base: options.css_public_base,
        legal_comments,
        theme,
        projection_manifests,
    };

    let result = webui::build(build_options)
        .map_err(|e| NapiError::from_reason(format!("Build error: {}", e.chain_message())))?;

    // Flatten css_files into alternating [filename, content, ...] for JS interop
    let css_files: Vec<String> = result
        .css_files
        .into_iter()
        .flat_map(|(name, content)| [name, content])
        .collect();
    let component_asset_files: Vec<String> = result
        .component_asset_files
        .into_iter()
        .flat_map(|file| [file.name, file.content])
        .collect();
    let warnings: Vec<String> = result.warnings.iter().map(|d| d.to_string()).collect();

    Ok(JsBuildResult {
        protocol: Buffer::from(result.protocol_bytes),
        css_files,
        component_asset_files,
        metafile: result.metafile,
        warnings,
        stats: JsBuildStats {
            duration_ms: result.stats.duration.as_secs_f64() * 1000.0,
            fragment_count: result.stats.fragment_count as u32,
            component_count: result.stats.component_count as u32,
            css_file_count: result.stats.css_file_count as u32,
            protocol_size_bytes: result.stats.protocol_size_bytes as u32,
            token_count: result.stats.token_count as u32,
        },
    })
}

fn load_theme(theme: &str, search_root: &std::path::Path) -> napi::Result<webui::TokenFile> {
    let resolved = webui::resolve_theme_path(theme, search_root)
        .map_err(|e| NapiError::from_reason(format!("Theme resolution error: {e}")))?;
    webui::load_token_file(&resolved).map_err(|e| {
        NapiError::from_reason(format!("Theme load error for {}: {e}", resolved.display()))
    })
}

/// Inspect protocol bytes and return a JSON representation.
#[napi]
pub fn inspect(protocol_data: Buffer) -> napi::Result<String> {
    webui::inspect_bytes(&protocol_data)
        .map_err(|e| NapiError::from_reason(format!("Inspect error: {e}")))
}

/// A decoded protocol and its reusable deterministic indices.
///
/// Create this once when a Node server loads `protocol.bin`, then reuse it for
/// full renders, partial navigation, component loading, and token queries. The
/// selected hydration plugin is bound once at construction.
#[napi]
pub struct Protocol {
    inner: Arc<HandlerProtocol>,
    handler: Arc<WebUIHandler>,
    output_capacity_hints: RenderCapacityHints,
}

#[napi]
impl Protocol {
    /// Decode a protocol and bind its render plugin for repeated rendering.
    #[napi(constructor)]
    pub fn new(protocol_data: Buffer, plugin: Option<String>) -> napi::Result<Self> {
        let inner = Arc::new(decode_protocol(&protocol_data)?);
        let handler = Arc::new(create_handler(plugin)?);
        Ok(Self {
            inner,
            handler,
            output_capacity_hints: RenderCapacityHints::new(),
        })
    }

    /// Render from an existing JSON string into a UTF-8 Node.js buffer.
    #[napi]
    pub fn render(
        &self,
        state_json: String,
        entry: String,
        request_path: String,
    ) -> napi::Result<Buffer> {
        let state = parse_state_json(&state_json)?;
        let options = RenderOptions::new(&entry, &request_path);
        self.render_buffer(&state, &options)
    }

    /// Parse and retain an immutable, process-local state snapshot.
    ///
    /// The returned N-API external owns the parsed state until its JavaScript
    /// handle is garbage-collected.
    #[napi]
    pub fn prepare_state(&self, state_json: String) -> napi::Result<External<Value>> {
        let size_hint = state_json.len().saturating_mul(2);
        Ok(External::new_with_size_hint(
            parse_state_json(&state_json)?,
            size_hint,
        ))
    }

    /// Render an immutable state snapshot prepared by this process.
    #[napi]
    pub fn render_prepared(
        &self,
        state: &External<Value>,
        entry: String,
        request_path: String,
    ) -> napi::Result<Buffer> {
        let options = RenderOptions::new(&entry, &request_path);
        self.render_buffer(state.as_ref(), &options)
    }

    /// Stream an existing JSON string in bounded chunks.
    #[napi]
    pub fn render_stream(
        &self,
        state_json: String,
        entry: String,
        request_path: String,
        on_chunk: Function<String>,
    ) -> napi::Result<()> {
        let state = parse_state_json(&state_json)?;
        let options = RenderOptions::new(&entry, &request_path);
        render_to_callback(&self.handler, &self.inner, &state, &options, &on_chunk)
    }

    /// Produce a complete partial-navigation response.
    #[napi]
    pub fn render_partial(
        &self,
        state_json: String,
        entry_id: String,
        request_path: String,
        inventory_hex: String,
    ) -> napi::Result<String> {
        self.inner
            .render_partial(&state_json, &entry_id, &request_path, &inventory_hex)
            .map_err(|e| NapiError::from_reason(format!("render_partial failed: {e}")))
    }

    /// Render component templates and styles for on-demand loading.
    #[napi]
    pub fn render_component_templates(
        &self,
        component_tags: Vec<String>,
        inventory_hex: String,
    ) -> napi::Result<String> {
        let tag_refs: Vec<&str> = component_tags.iter().map(String::as_str).collect();
        let result = self
            .inner
            .render_component_templates(&tag_refs, &inventory_hex)
            .map_err(|e| {
                NapiError::from_reason(format!("render_component_templates failed: {e}"))
            })?;
        serde_json::to_string(&result)
            .map_err(|e| NapiError::from_reason(format!("JSON serialize error: {e}")))
    }

    /// Return CSS token names in build order.
    #[napi]
    pub fn tokens(&self) -> Vec<String> {
        self.inner.tokens().to_vec()
    }

    /// Open a host-driven progressive response for a streaming entry.
    ///
    /// Unlike `renderStream`, which pushes every chunk during one synchronous
    /// call, the returned session hands each chunk back so the Node server
    /// owns the socket, the write order, and backpressure.
    #[napi]
    pub fn stream_response(
        &self,
        entry: String,
        request_path: String,
        options: Option<JsStreamOptions>,
    ) -> napi::Result<StreamingSession> {
        let options = options.unwrap_or_default();
        let mut session_options = SessionOptions::new(entry, request_path);
        session_options.nonce = options.nonce;
        session_options.head_inject = options.head_inject;
        session_options.body_inject = options.body_inject;
        let inner = HandlerStreamingSession::new(
            Arc::clone(&self.handler),
            Arc::clone(&self.inner),
            session_options,
        )
        .map_err(streaming_error)?;
        Ok(StreamingSession { inner })
    }
}

impl Protocol {
    fn render_buffer(&self, state: &Value, options: &RenderOptions<'_>) -> napi::Result<Buffer> {
        let capacity = self
            .output_capacity_hints
            .load(options.entry_id, options.request_path);
        let mut html = render_to_string(&self.handler, &self.inner, state, options, capacity)?;
        self.output_capacity_hints
            .store(options.entry_id, options.request_path, html.len());
        // Bound a stale same-route hint without reallocating normally sized output.
        if html.capacity() > html.len().saturating_mul(2) {
            html.shrink_to_fit();
        }
        // napi-rs can expose the existing Vec as an external Buffer on Node.
        Ok(Buffer::from(html.into_bytes()))
    }
}

/// Optional per-response streaming settings.
#[napi(object)]
#[derive(Default)]
pub struct JsStreamOptions {
    /// CSP nonce applied to generated inline `<script>` tags.
    pub nonce: Option<String>,
    /// HTML injected at the structural `head_end` boundary.
    pub head_inject: Option<String>,
    /// HTML injected at the structural `body_end` boundary.
    pub body_inject: Option<String>,
}

/// A progressive HTML response driven one semantic step at a time from Node.
///
/// `start()`, `resume()`, and `advance()` return bytes, completion state, and
/// the next runtime boundary occurrence (if any). Boundary keys retain their
/// authored JSON type: strings are JavaScript strings and finite numbers are
/// JavaScript numbers.
///
/// ```js
/// const session = protocol.streamResponse('index.html', '/');
/// let step = session.start(JSON.stringify(shellState));
/// res.write(step.bytes);
/// while (!step.done) {
///   const { instanceId, name, key } = step.boundary;
///   const state = await loadBoundary(name, key);
///   step = session.resume(instanceId, JSON.stringify(state), 'updatable');
///   res.write(step.bytes);
///   step = session.advance();
///   res.write(step.bytes);
/// }
/// ```
#[napi]
pub struct StreamingSession {
    inner: HandlerStreamingSession,
}

/// Runtime boundary occurrence returned by a streaming step.
#[napi(object)]
pub struct JsBoundaryDescriptor {
    /// Gapless response-local occurrence ID passed to `resume()` and `update()`.
    pub instance_id: u32,
    /// Stable compiler declaration ID.
    pub declaration_id: u32,
    /// Entry or component template that owns this declaration.
    pub owner: String,
    /// Free-form authored boundary name.
    pub name: String,
    /// Evaluated boundary key, preserving string-versus-number identity.
    pub key: Option<Either<String, f64>>,
}

/// Bytes and continuation state returned by one streaming session step.
#[napi(object)]
pub struct JsStreamStep {
    /// Complete bytes produced by this semantic step.
    pub bytes: Buffer,
    /// Whether the terminal record was emitted.
    pub done: bool,
    /// Next runtime boundary occurrence waiting for `resume()`.
    pub boundary: Option<JsBoundaryDescriptor>,
}

#[napi]
impl StreamingSession {
    /// Render until the first runtime boundary occurrence or terminal.
    #[napi]
    pub fn start(&mut self, state_json: String) -> napi::Result<JsStreamStep> {
        let state = parse_state_json(&state_json)?;
        self.inner
            .start_owned(state)
            .and_then(stream_step)
            .map_err(streaming_error)
    }

    /// Commit the pending occurrence through its checkpoint, then stop.
    ///
    /// `mode` is `"final"` (default) or `"updatable"`. Only updatable
    /// boundaries accept later `update()` calls.
    #[napi]
    pub fn resume(
        &mut self,
        instance_id: u32,
        state_json: String,
        mode: Option<String>,
    ) -> napi::Result<JsStreamStep> {
        let state = parse_state_json(&state_json)?;
        let mode = parse_boundary_mode(mode.as_deref())?;
        self.inner
            .resume_owned(BoundaryInstanceId::from_raw(instance_id), state, mode)
            .and_then(stream_step)
            .map_err(streaming_error)
    }

    /// Write the parent bytes after the committed occurrence.
    ///
    /// Valid only after `resume()`. Returns the next boundary occurrence or
    /// completes the document tail.
    #[napi]
    pub fn advance(&mut self) -> napi::Result<JsStreamStep> {
        self.inner
            .advance()
            .and_then(stream_step)
            .map_err(streaming_error)
    }

    /// Push a projected state patch to a committed updatable boundary.
    #[napi]
    pub fn update(&mut self, instance_id: u32, patch_json: String) -> napi::Result<Buffer> {
        let patch = parse_state_json(&patch_json)?;
        self.inner
            .update(BoundaryInstanceId::from_raw(instance_id), &patch)
            .map(Buffer::from)
            .map_err(streaming_error)
    }
}

fn stream_step(step: HandlerStreamStep) -> webui_handler::Result<JsStreamStep> {
    Ok(JsStreamStep {
        bytes: Buffer::from(step.bytes),
        done: step.done,
        boundary: step.boundary.map(boundary_descriptor).transpose()?,
    })
}

fn boundary_descriptor(
    boundary: BoundaryDescriptor,
) -> webui_handler::Result<JsBoundaryDescriptor> {
    Ok(JsBoundaryDescriptor {
        instance_id: boundary.instance_id.raw(),
        declaration_id: boundary.declaration_id,
        owner: boundary.owner.to_string(),
        name: boundary.name.to_string(),
        key: boundary.key.map(boundary_key).transpose()?,
    })
}

fn boundary_key(key: BoundaryKey) -> webui_handler::Result<Either<String, f64>> {
    match key {
        BoundaryKey::String(value) => Ok(Either::A(value)),
        BoundaryKey::Number(value) => value.as_f64().map(Either::B).ok_or_else(|| {
            HandlerError::Invariant(
                "boundary key cannot be represented as a JavaScript number".into(),
            )
        }),
    }
}

fn parse_boundary_mode(mode: Option<&str>) -> napi::Result<BoundaryMode> {
    match mode {
        None | Some("final") => Ok(BoundaryMode::Final),
        Some("updatable") => Ok(BoundaryMode::Updatable),
        Some(other) => Err(NapiError::from_reason(format!(
            "unknown boundary mode '{other}'; expected 'final' or 'updatable'"
        ))),
    }
}

fn streaming_error(error: HandlerError) -> NapiError {
    NapiError::from_reason(error.to_string())
}

fn decode_protocol(protocol_data: &[u8]) -> napi::Result<HandlerProtocol> {
    HandlerProtocol::from_protobuf(protocol_data)
        .map_err(|e| NapiError::from_reason(format!("Protocol decode error: {e}")))
}

fn parse_state_json(state_json: &str) -> napi::Result<Value> {
    serde_json::from_str(state_json)
        .map_err(|e| NapiError::from_reason(format!("State JSON error: {e}")))
}

fn create_handler(plugin: Option<String>) -> napi::Result<WebUIHandler> {
    let plugin = plugin
        .map(|value| value.parse::<webui::Plugin>())
        .transpose()
        .map_err(NapiError::from_reason)?;
    Ok(match plugin {
        Some(webui::Plugin::Fast | webui::Plugin::FastV2) => {
            WebUIHandler::with_plugin(|| Box::new(FastV2HydrationPlugin::new()))
        }
        Some(webui::Plugin::FastV3) => {
            WebUIHandler::with_plugin(|| Box::new(FastV3HydrationPlugin::new()))
        }
        Some(webui::Plugin::WebUI) => {
            WebUIHandler::with_plugin(|| Box::new(WebUIHydrationPlugin::new()))
        }
        None => WebUIHandler::new(),
    })
}

webui_handler::define_string_response_writer!(BufferedWriter, output);

fn render_to_string(
    handler: &WebUIHandler,
    protocol: &HandlerProtocol,
    state: &Value,
    options: &RenderOptions<'_>,
    capacity: usize,
) -> napi::Result<String> {
    let mut writer = BufferedWriter::with_capacity(capacity);
    handler
        .render(protocol, state, options, &mut writer)
        .map_err(|e| NapiError::from_reason(format!("Render error: {e}")))?;
    Ok(writer.output)
}

/// A writer that batches rendered fragments before crossing into JavaScript.
struct CallbackWriter<F> {
    callback: F,
    buffer: String,
    error: Option<NapiError>,
}

impl<F> CallbackWriter<F>
where
    F: FnMut(String) -> napi::Result<()>,
{
    fn new(callback: F) -> Self {
        Self {
            callback,
            buffer: String::with_capacity(STREAM_CHUNK_SIZE),
            error: None,
        }
    }

    fn flush(&mut self) -> webui_handler::Result<()> {
        if self.error.is_some() {
            return Err(callback_writer_error());
        }
        if self.buffer.is_empty() {
            return Ok(());
        }

        let chunk = std::mem::replace(&mut self.buffer, String::with_capacity(STREAM_CHUNK_SIZE));
        if let Err(error) = (self.callback)(chunk) {
            self.error = Some(error);
            return Err(callback_writer_error());
        }
        Ok(())
    }
}

impl<F> ResponseWriter for CallbackWriter<F>
where
    F: FnMut(String) -> napi::Result<()>,
{
    fn write(&mut self, content: &str) -> webui_handler::Result<()> {
        if self.error.is_some() {
            return Err(callback_writer_error());
        }
        self.buffer.push_str(content);
        if self.buffer.len() >= STREAM_CHUNK_SIZE {
            self.flush()?;
        }
        Ok(())
    }

    fn write_attribute(&mut self, name: &str, value: &str) -> webui_handler::Result<()> {
        if self.error.is_some() {
            return Err(callback_writer_error());
        }
        webui_handler::append_attribute_to_string(&mut self.buffer, name, value);
        if self.buffer.len() >= STREAM_CHUNK_SIZE {
            self.flush()?;
        }
        Ok(())
    }

    fn write_boolean_attribute(&mut self, name: &str) -> webui_handler::Result<()> {
        if self.error.is_some() {
            return Err(callback_writer_error());
        }
        webui_handler::append_boolean_attribute_to_string(&mut self.buffer, name);
        if self.buffer.len() >= STREAM_CHUNK_SIZE {
            self.flush()?;
        }
        Ok(())
    }

    fn end(&mut self) -> webui_handler::Result<()> {
        self.flush()
    }
}

#[cold]
#[inline(never)]
fn callback_writer_error() -> HandlerError {
    HandlerError::Writer("Node chunk callback failed".to_owned())
}

fn render_to_callback(
    handler: &WebUIHandler,
    protocol: &HandlerProtocol,
    state: &Value,
    options: &RenderOptions<'_>,
    on_chunk: &Function<String>,
) -> napi::Result<()> {
    let mut writer = CallbackWriter::new(|chunk| on_chunk.call(chunk).map(drop));
    let render_result = handler.render(protocol, state, options, &mut writer);
    if let Some(error) = writer.error {
        return Err(error);
    }
    render_result.map_err(|e| NapiError::from_reason(format!("Render error: {e}")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;
    use webui_parser::HtmlParser;
    use webui_protocol::projection_manifest::{
        ProjectionAdapter, ProjectionComponent, ProjectionManifest, ProjectionProducer,
        PRODUCER_NAME, SCHEMA_ID,
    };

    /// Helper: parse HTML into protobuf bytes for testing.
    fn build_protocol(html: &str) -> Vec<u8> {
        let mut parser = HtmlParser::new();
        parser.parse("index.html", html).expect("parse failed");
        let tokens = parser.take_tokens();
        let protocol = WebUIProtocol::with_tokens(parser.into_fragment_records(), tokens);
        protocol.to_protobuf().expect("protobuf encode failed")
    }

    /// Helper: render protocol bytes + state, collecting output into a String.
    fn render_to_string(protocol_bytes: &[u8], state_json: &str) -> Result<String, String> {
        let protocol = HandlerProtocol::from_protobuf(protocol_bytes).map_err(|e| e.to_string())?;
        let state: Value = serde_json::from_str(state_json).map_err(|e| e.to_string())?;

        let mut output = String::with_capacity(1024);
        let handler = WebUIHandler::new();

        struct StringWriter<'a> {
            output: &'a mut String,
        }
        impl ResponseWriter for StringWriter<'_> {
            fn write(&mut self, content: &str) -> webui_handler::Result<()> {
                self.output.push_str(content);
                Ok(())
            }
            fn end(&mut self) -> webui_handler::Result<()> {
                Ok(())
            }
        }

        let mut writer = StringWriter {
            output: &mut output,
        };
        handler
            .render(
                &protocol,
                &state,
                &RenderOptions::new("index.html", "/"),
                &mut writer,
            )
            .map_err(|e| e.to_string())?;
        Ok(output)
    }

    #[test]
    fn render_capacity_hints_isolate_entries_and_routes() {
        let hints = RenderCapacityHints::new();

        hints.store("index.html", "/contacts", MAX_RENDER_CAPACITY_HINT);

        assert_eq!(
            hints.load("index.html", "/"),
            INITIAL_RENDER_CAPACITY,
            "a small route must not inherit a large route's capacity"
        );
        assert_eq!(
            hints.load("contacts.html", "/contacts"),
            INITIAL_RENDER_CAPACITY,
            "another entry must not inherit the large entry's capacity"
        );
        assert_eq!(
            hints.load("index.html", "/contacts"),
            MAX_RENDER_CAPACITY_HINT
        );
    }

    #[test]
    fn render_capacity_hints_do_not_leak_across_bucket_collisions() {
        let entry_id = "index.html";
        let large_path = "/contacts";
        let large_hash = capacity_hint_hash(entry_id, large_path);
        let colliding_path = (0..RENDER_CAPACITY_BUCKETS * 16)
            .map(|index| format!("/route-{index}"))
            .find(|path| {
                let hash = capacity_hint_hash(entry_id, path);
                hash & (RENDER_CAPACITY_BUCKETS - 1) == large_hash & (RENDER_CAPACITY_BUCKETS - 1)
                    && capacity_hint_fingerprint(hash) != capacity_hint_fingerprint(large_hash)
            })
            .unwrap_or_else(|| panic!("a same-bucket route should be found"));
        let hints = RenderCapacityHints::new();

        hints.store(entry_id, large_path, MAX_RENDER_CAPACITY_HINT);
        assert_eq!(
            hints.load(entry_id, &colliding_path),
            INITIAL_RENDER_CAPACITY
        );

        hints.store(entry_id, &colliding_path, INITIAL_RENDER_CAPACITY * 2);
        assert_eq!(
            hints.load(entry_id, large_path),
            INITIAL_RENDER_CAPACITY,
            "an overwritten bucket must miss rather than return another route's hint"
        );
        assert_eq!(
            hints.load(entry_id, &colliding_path),
            INITIAL_RENDER_CAPACITY * 2
        );
    }

    #[test]
    fn render_capacity_hints_clamp_retained_sizes() {
        let hints = RenderCapacityHints::new();

        hints.store("index.html", "/", 0);
        assert_eq!(hints.load("index.html", "/"), INITIAL_RENDER_CAPACITY);

        hints.store("index.html", "/", usize::MAX);
        assert_eq!(hints.load("index.html", "/"), MAX_RENDER_CAPACITY_HINT);
    }

    #[test]
    fn learned_render_capacity_avoids_string_growth_without_changing_bytes() {
        struct ReallocationWriter {
            output: String,
            reallocations: usize,
        }

        impl ResponseWriter for ReallocationWriter {
            fn write(&mut self, content: &str) -> webui_handler::Result<()> {
                let previous_capacity = self.output.capacity();
                self.output.push_str(content);
                self.reallocations += usize::from(self.output.capacity() != previous_capacity);
                Ok(())
            }

            fn end(&mut self) -> webui_handler::Result<()> {
                Ok(())
            }
        }

        fn render_with_capacity(protocol: &HandlerProtocol, capacity: usize) -> (String, usize) {
            let mut writer = ReallocationWriter {
                output: String::with_capacity(capacity),
                reallocations: 0,
            };
            WebUIHandler::new()
                .render(
                    protocol,
                    &Value::Null,
                    &RenderOptions::new("index.html", "/"),
                    &mut writer,
                )
                .expect("render should succeed");
            (writer.output, writer.reallocations)
        }

        let source = format!("<main>{}</main>", "<p>capacity</p>".repeat(1024));
        let protocol =
            HandlerProtocol::from_protobuf(&build_protocol(&source)).expect("protocol should load");
        let (cold_output, cold_reallocations) =
            render_with_capacity(&protocol, INITIAL_RENDER_CAPACITY);
        let (warm_output, warm_reallocations) = render_with_capacity(&protocol, cold_output.len());

        assert!(cold_reallocations > 0);
        assert_eq!(warm_reallocations, 0);
        assert_eq!(warm_output, cold_output);
    }

    #[test]
    fn test_simple_passthrough() {
        let proto = build_protocol("<p>Hello</p>");
        let result = render_to_string(&proto, "{}");
        assert_eq!(result.as_deref(), Ok("<p>Hello</p>"));
    }

    #[test]
    fn test_signal_substitution() {
        let proto = build_protocol("Hello, {{name}}!");
        let result = render_to_string(&proto, r#"{"name": "WebUI"}"#);
        assert_eq!(result.as_deref(), Ok("Hello, WebUI!"));
    }

    #[test]
    fn protocol_reuses_decoded_protocol_for_json_state() {
        let proto = build_protocol("Hello, {{name}}!");
        let protocol = Protocol::new(Buffer::from(proto), None).expect("protocol should load");

        let first = protocol
            .render(
                r#"{"name":"First"}"#.to_string(),
                "index.html".to_string(),
                "/".to_string(),
            )
            .expect("first render should succeed");
        let second = protocol
            .render(
                r#"{"name":"世界"}"#.to_string(),
                "index.html".to_string(),
                "/".to_string(),
            )
            .expect("second render should succeed");

        assert_eq!(first.as_ref(), b"Hello, First!");
        assert_eq!(second.as_ref(), "Hello, 世界!".as_bytes());
    }

    #[test]
    fn streaming_steps_preserve_key_types_and_checkpoint_segments() {
        let proto = build_protocol(concat!(
            "<html><head></head><body>",
            r#"<boundary name="first" key="{{firstId}}"><p>{{firstLabel}}</p></boundary>"#,
            "<span>between</span>",
            r#"<boundary name="second" key="{{secondId}}"><p>{{secondLabel}}</p></boundary>"#,
            "<footer>tail</footer>",
            "</body></html>",
        ));
        let protocol = Protocol::new(Buffer::from(proto), None).expect("protocol should load");
        let mut session = protocol
            .stream_response("index.html".to_string(), "/".to_string(), None)
            .expect("session should open");
        let state = r#"{"firstId":"alpha","firstLabel":"a","secondId":20,"secondLabel":"b"}"#;

        let first = session
            .start(state.to_string())
            .expect("start should discover first boundary");
        assert!(!first.done);
        assert!(!first.bytes.is_empty());
        let first_boundary = first.boundary.expect("first boundary should be returned");
        assert_eq!(first_boundary.instance_id, 0);
        assert_eq!(first_boundary.declaration_id, 0);
        assert_eq!(first_boundary.owner, "index.html");
        assert_eq!(first_boundary.name, "first");
        assert!(matches!(
            first_boundary.key,
            Some(Either::A(value)) if value == "alpha"
        ));

        let resumed = session
            .resume(
                first_boundary.instance_id,
                state.to_string(),
                Some("final".to_string()),
            )
            .expect("resume should commit first boundary");
        assert!(!resumed.done);
        assert!(resumed.boundary.is_none());
        let resumed_text =
            std::str::from_utf8(&resumed.bytes).expect("resume output should be UTF-8");
        assert!(resumed_text.contains(">a<"));
        assert!(!resumed_text.contains("between"));

        let next = session
            .advance()
            .expect("advance should discover second boundary");
        assert!(!next.done);
        let next_text = std::str::from_utf8(&next.bytes).expect("advance output should be UTF-8");
        assert!(next_text.contains("between"));
        assert!(!next_text.contains(">b<"));
        let second_boundary = next.boundary.expect("second boundary should be returned");
        assert_eq!(second_boundary.instance_id, 1);
        assert_eq!(second_boundary.declaration_id, 1);
        assert_eq!(second_boundary.name, "second");
        assert!(matches!(
            second_boundary.key,
            Some(Either::B(value)) if value == 20.0
        ));

        let resumed = session
            .resume(second_boundary.instance_id, state.to_string(), None)
            .expect("resume should commit second boundary");
        assert!(!resumed.done);
        assert!(resumed.boundary.is_none());
        let resumed_text =
            std::str::from_utf8(&resumed.bytes).expect("resume output should be UTF-8");
        assert!(resumed_text.contains(">b<"));
        assert!(!resumed_text.contains("tail"));

        let done = session.advance().expect("final advance should complete");
        assert!(done.done);
        assert!(done.boundary.is_none());
        assert!(std::str::from_utf8(&done.bytes)
            .expect("advance output should be UTF-8")
            .contains("tail"));
    }

    #[test]
    fn streaming_update_returns_buffer_for_updatable_occurrence() {
        let proto = build_protocol(concat!(
            "<html><head></head><body>",
            r#"<boundary name="first"><p>{{count}}</p></boundary>"#,
            r#"<boundary name="second"><p>done</p></boundary>"#,
            "</body></html>",
        ));
        let protocol = Protocol::new(Buffer::from(proto), None).expect("protocol should load");
        let mut session = protocol
            .stream_response("index.html".to_string(), "/".to_string(), None)
            .expect("session should open");
        let first = session
            .start(r#"{"count":1}"#.to_string())
            .expect("start should discover first boundary")
            .boundary
            .expect("first boundary should be returned");
        let resumed = session
            .resume(
                first.instance_id,
                r#"{"count":1}"#.to_string(),
                Some("updatable".to_string()),
            )
            .expect("resume should commit updatable boundary");
        assert!(!resumed.done);
        assert!(resumed.boundary.is_none());

        let update = session
            .update(first.instance_id, r#"{"count":2}"#.to_string())
            .expect("update should render");
        assert!(!update.is_empty());
        let update_text = std::str::from_utf8(&update).expect("update should be UTF-8");
        assert!(update_text.contains(r#""count":2"#));

        let second = session
            .advance()
            .expect("advance should discover second boundary")
            .boundary
            .expect("second boundary should be returned");
        let resumed = session
            .resume(second.instance_id, "{}".to_string(), None)
            .expect("second resume should commit boundary");
        assert!(!resumed.done);
        assert!(resumed.boundary.is_none());
        let done = session.advance().expect("final advance should complete");
        assert!(done.done);
    }

    #[test]
    fn streaming_advance_rejects_out_of_order_calls() {
        let proto = build_protocol(concat!(
            "<html><head></head><body>",
            r#"<boundary name="first"><p>first</p></boundary>"#,
            "</body></html>",
        ));
        let protocol = Protocol::new(Buffer::from(proto), None).expect("protocol should load");
        let mut session = protocol
            .stream_response("index.html".to_string(), "/".to_string(), None)
            .expect("session should open");

        let before_start = session
            .advance()
            .err()
            .expect("advance before start should fail");
        assert!(before_start
            .to_string()
            .contains("start must be called before this operation"));

        let start = session
            .start("{}".to_string())
            .expect("start should succeed");
        let before_resume = session
            .advance()
            .err()
            .expect("advance before resume should fail");
        assert!(before_resume
            .to_string()
            .contains("there is no committed boundary to advance past"));

        let boundary = start.boundary.expect("first boundary should be returned");
        session
            .resume(boundary.instance_id, "{}".to_string(), None)
            .expect("resume should still succeed after rejected advance");
        assert!(session.advance().expect("advance should complete").done);
    }

    #[test]
    fn streaming_start_returns_done_for_boundary_free_document() {
        let proto = build_protocol("<html><head></head><body><p>done</p></body></html>");
        let protocol = Protocol::new(Buffer::from(proto), None).expect("protocol should load");
        let mut session = protocol
            .stream_response("index.html".to_string(), "/".to_string(), None)
            .expect("session should open");

        let step = session
            .start("{}".to_string())
            .expect("boundary-free start should complete");
        assert!(step.done);
        assert!(step.boundary.is_none());
        assert!(std::str::from_utf8(&step.bytes)
            .expect("output should be UTF-8")
            .contains("<p>done</p>"));
    }

    #[test]
    fn test_for_loop() {
        let proto = build_protocol("<ul><for each=\"item in items\"><li>{{item}}</li></for></ul>");
        let result = render_to_string(&proto, r#"{"items": ["a", "b", "c"]}"#);
        assert_eq!(
            result.as_deref(),
            Ok("<ul><li>a</li><li>b</li><li>c</li></ul>")
        );
    }

    #[test]
    fn test_if_condition_true() {
        let proto = build_protocol("<if condition=\"show\"><p>Visible</p></if>");
        let result = render_to_string(&proto, r#"{"show": true}"#);
        assert_eq!(result.as_deref(), Ok("<p>Visible</p>"));
    }

    #[test]
    fn test_if_condition_false() {
        let proto = build_protocol("<if condition=\"show\"><p>Hidden</p></if>");
        let result = render_to_string(&proto, r#"{"show": false}"#);
        assert_eq!(result.as_deref(), Ok(""));
    }

    #[test]
    fn test_html_escaping() {
        let proto = build_protocol("<div>{{content}}</div>");
        let state = r#"{"content": "<script>alert('xss')</script>"}"#;
        let result = render_to_string(&proto, state).expect("render should succeed");
        assert!(!result.contains("<script>"));
        assert!(result.contains("&lt;script&gt;"));
    }

    #[test]
    fn test_raw_signal() {
        let proto = build_protocol("<div>{{{content}}}</div>");
        let result = render_to_string(&proto, r#"{"content": "<b>bold</b>"}"#);
        assert_eq!(result.as_deref(), Ok("<div><b>bold</b></div>"));
    }

    #[test]
    fn test_invalid_json() {
        let proto = build_protocol("<p>hi</p>");
        let result = render_to_string(&proto, "NOT JSON");
        assert!(result.is_err());
    }

    #[test]
    fn test_empty_state() {
        let proto = build_protocol("<p>static</p>");
        let result = render_to_string(&proto, "{}");
        assert_eq!(result.as_deref(), Ok("<p>static</p>"));
    }

    #[test]
    fn test_nested_object_signal() {
        let proto = build_protocol("{{user.name}}");
        let result = render_to_string(&proto, r#"{"user": {"name": "Alice"}}"#);
        assert_eq!(result.as_deref(), Ok("Alice"));
    }

    #[test]
    fn test_invalid_protobuf() {
        let result = render_to_string(&[0xFF, 0xFF, 0xFF], "{}");
        assert!(result.is_err());
    }

    /// Parse `html`, attach a sorted hydration `schema`, and encode to protobuf.
    fn build_projected_protocol(html: &str, schema: &[&str]) -> Vec<u8> {
        let mut parser = HtmlParser::new();
        parser.parse("index.html", html).expect("parse failed");
        let tokens = parser.take_tokens();
        let mut protocol = WebUIProtocol::with_tokens(parser.into_fragment_records(), tokens);
        protocol.fragments.insert(
            "client-card".to_string(),
            webui_protocol::FragmentList {
                fragments: vec![webui_protocol::WebUIFragment::raw("<p>client</p>")],
                contains_boundary: false,
            },
        );
        protocol
            .fragments
            .get_mut("index.html")
            .expect("index fragment should exist")
            .fragments
            .insert(1, webui_protocol::WebUIFragment::component("client-card"));
        protocol.initial_state_strategy = webui_protocol::InitialStateStrategy::Components as i32;
        protocol.components.insert(
            "client-card".to_string(),
            webui_protocol::ComponentData {
                hydration_mode: webui_protocol::StateProjectionMode::Keys as i32,
                hydration_keys: schema.iter().map(|key| (*key).to_string()).collect(),
                ..Default::default()
            },
        );
        protocol.to_protobuf().expect("protobuf encode failed")
    }

    /// Render protocol bytes with the WebUI hydration plugin so the `#webui-data`
    /// bootstrap block (and its projected state) is emitted — this mirrors the
    /// production `render(..., plugin = "webui")` path.
    fn render_with_webui_plugin(protocol_bytes: &[u8], state_json: &str) -> Result<String, String> {
        let protocol = HandlerProtocol::from_protobuf(protocol_bytes).map_err(|e| e.to_string())?;
        let state: Value = serde_json::from_str(state_json).map_err(|e| e.to_string())?;

        let mut output = String::with_capacity(1024);
        struct StringWriter<'a> {
            output: &'a mut String,
        }
        impl ResponseWriter for StringWriter<'_> {
            fn write(&mut self, content: &str) -> webui_handler::Result<()> {
                self.output.push_str(content);
                Ok(())
            }
            fn end(&mut self) -> webui_handler::Result<()> {
                Ok(())
            }
        }
        let mut writer = StringWriter {
            output: &mut output,
        };
        let handler = WebUIHandler::with_plugin(|| Box::new(WebUIHydrationPlugin::new()));
        handler
            .render(
                &protocol,
                &state,
                &RenderOptions::new("index.html", "/"),
                &mut writer,
            )
            .map_err(|e| e.to_string())?;
        Ok(output)
    }

    #[test]
    fn render_projects_state_to_component_hydration_keys() {
        // Full document so the parser emits a `body_end` signal, which makes the
        // WebUI plugin emit the #webui-data bootstrap block.
        let bytes =
            build_projected_protocol("<html><body><p>{{kept}}</p></body></html>", &["kept"]);
        let out =
            render_with_webui_plugin(&bytes, r#"{"kept":"KEPT_VALUE","dropped":"DROPPED_VALUE"}"#)
                .expect("render should succeed");

        // Only the hydratable key reaches the bootstrap state block...
        assert!(
            out.contains(r#""kept":"KEPT_VALUE""#),
            "hydratable key missing from bootstrap state: {out}"
        );
        // ...the non-hydratable key is projected out entirely.
        assert!(
            !out.contains("DROPPED_VALUE"),
            "server-only value leaked: {out}"
        );
        assert!(
            !out.contains("dropped"),
            "server-only key name leaked: {out}"
        );
    }

    // ── Tests for build() and inspect() napi exports ─────────────────

    fn projection_manifest_json() -> String {
        const EMPTY_SHA256: &str =
            "sha256:e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855";
        let mut manifest = ProjectionManifest {
            schema: SCHEMA_ID.to_string(),
            producer: ProjectionProducer {
                name: PRODUCER_NAME.to_string(),
                version: "0.0.18".to_string(),
            },
            adapter: ProjectionAdapter {
                name: "test".to_string(),
                bundler: "test@1.0.0".to_string(),
            },
            root: ".".to_string(),
            analysis_hash: format!("sha256:{}", "1".repeat(64)),
            build_id: String::new(),
            inputs: BTreeMap::from([("demo-card.ts".to_string(), EMPTY_SHA256.to_string())]),
            outputs: BTreeMap::from([("bundle.js".to_string(), EMPTY_SHA256.to_string())]),
            components: BTreeMap::from([(
                "demo-card".to_string(),
                ProjectionComponent {
                    module: "demo-card.ts".to_string(),
                    outputs: vec!["bundle.js".to_string()],
                    hydration_keys: vec!["name".to_string()],
                    navigation_keys: vec!["label".to_string(), "name".to_string()],
                },
            )]),
            entry_closures: BTreeMap::new(),
        };
        manifest.build_id = manifest.compute_build_id();
        serde_json::to_string(&manifest).unwrap()
    }

    fn projection_build_options(app_dir: &std::path::Path) -> JsBuildOptions {
        JsBuildOptions {
            app_dir: app_dir.to_string_lossy().to_string(),
            entry: None,
            css: None,
            dom: None,
            css_bundle: None,
            plugin: Some("webui".to_string()),
            components: None,
            component_asset_roots: None,
            metafile: None,
            css_file_name_template: None,
            css_public_base: None,
            legal_comments: None,
            theme: None,
            projection_manifests: None,
            projection_manifest_objects: None,
        }
    }

    #[test]
    fn test_build_accepts_projection_paths_and_inline_objects() {
        let dir = tempfile::TempDir::new().unwrap();
        std::fs::write(dir.path().join("index.html"), "<demo-card></demo-card>").unwrap();
        std::fs::write(dir.path().join("demo-card.html"), "<p>{{name}}</p>").unwrap();
        std::fs::write(dir.path().join("demo-card.ts"), "").unwrap();
        std::fs::write(dir.path().join("bundle.js"), "").unwrap();
        let manifest_path = dir.path().join("projection.json");
        let json = projection_manifest_json();
        std::fs::write(&manifest_path, &json).unwrap();

        let mut path_options = projection_build_options(dir.path());
        path_options.projection_manifests = Some(vec![manifest_path.to_string_lossy().to_string()]);
        let path_result = build(path_options).unwrap();
        let path_protocol = WebUIProtocol::from_protobuf(&path_result.protocol).unwrap();
        assert_eq!(
            path_protocol.components["demo-card"].hydration_keys,
            ["name"]
        );

        std::fs::remove_file(&manifest_path).unwrap();
        let mut inline_options = projection_build_options(dir.path());
        inline_options.projection_manifest_objects = Some(vec![JsProjectionManifest {
            path: manifest_path.to_string_lossy().to_string(),
            json,
        }]);
        let inline_result = build(inline_options).unwrap();
        let inline_protocol = WebUIProtocol::from_protobuf(&inline_result.protocol).unwrap();
        assert_eq!(
            inline_protocol.components["demo-card"].navigation_keys,
            ["label", "name"]
        );
    }

    #[test]
    fn test_build_simple_app() {
        let dir = tempfile::TempDir::new().unwrap();
        std::fs::write(dir.path().join("index.html"), "<h1>Hello</h1>").unwrap();

        let options = JsBuildOptions {
            app_dir: dir.path().to_string_lossy().to_string(),
            entry: None,
            css: None,
            dom: None,
            css_bundle: None,
            plugin: None,
            components: None,
            component_asset_roots: None,
            metafile: None,
            css_file_name_template: None,
            css_public_base: None,
            legal_comments: None,
            theme: None,
            projection_manifests: None,
            projection_manifest_objects: None,
        };

        let result = build(options).unwrap();
        assert!(!result.protocol.is_empty());
        assert!(result.stats.fragment_count > 0);
        assert!(result.stats.protocol_size_bytes > 0);
        assert!(result.stats.duration_ms >= 0.0);
    }

    #[test]
    fn test_build_with_custom_entry() {
        let dir = tempfile::TempDir::new().unwrap();
        std::fs::write(dir.path().join("page.html"), "<p>Custom</p>").unwrap();

        let options = JsBuildOptions {
            app_dir: dir.path().to_string_lossy().to_string(),
            entry: Some("page.html".to_string()),
            css: None,
            dom: None,
            css_bundle: None,
            plugin: None,
            components: None,
            component_asset_roots: None,
            metafile: None,
            css_file_name_template: None,
            css_public_base: None,
            legal_comments: None,
            theme: None,
            projection_manifests: None,
            projection_manifest_objects: None,
        };

        let result = build(options).unwrap();
        assert!(!result.protocol.is_empty());
    }

    #[test]
    fn test_build_missing_app_dir() {
        let options = JsBuildOptions {
            app_dir: "/nonexistent/path".to_string(),
            entry: None,
            css: None,
            dom: None,
            css_bundle: None,
            plugin: None,
            components: None,
            component_asset_roots: None,
            metafile: None,
            css_file_name_template: None,
            css_public_base: None,
            legal_comments: None,
            theme: None,
            projection_manifests: None,
            projection_manifest_objects: None,
        };

        let result = build(options);
        assert!(result.is_err());
    }

    #[test]
    fn test_build_invalid_css() {
        let dir = tempfile::TempDir::new().unwrap();
        std::fs::write(dir.path().join("index.html"), "<h1>Hello</h1>").unwrap();

        let options = JsBuildOptions {
            app_dir: dir.path().to_string_lossy().to_string(),
            entry: None,
            css: Some("bogus".to_string()),
            dom: None,
            css_bundle: None,
            plugin: None,
            components: None,
            component_asset_roots: None,
            metafile: None,
            css_file_name_template: None,
            css_public_base: None,
            legal_comments: None,
            theme: None,
            projection_manifests: None,
            projection_manifest_objects: None,
        };

        let result = build(options);
        assert!(result.is_err());
    }

    #[test]
    fn test_build_invalid_dom_strategy() {
        let dir = tempfile::TempDir::new().unwrap();
        std::fs::write(dir.path().join("index.html"), "<h1>Hello</h1>").unwrap();

        let options = JsBuildOptions {
            app_dir: dir.path().to_string_lossy().to_string(),
            entry: None,
            css: None,
            dom: Some("closed".to_string()),
            css_bundle: None,
            plugin: None,
            components: None,
            component_asset_roots: None,
            metafile: None,
            css_file_name_template: None,
            css_public_base: None,
            legal_comments: None,
            theme: None,
            projection_manifests: None,
            projection_manifest_objects: None,
        };

        assert!(build(options).is_err());
    }

    #[test]
    fn test_build_with_components_css() {
        let dir = tempfile::TempDir::new().unwrap();
        std::fs::write(dir.path().join("index.html"), "<my-card>Hello</my-card>").unwrap();
        std::fs::write(dir.path().join("my-card.html"), "<div>content</div>").unwrap();
        std::fs::write(dir.path().join("my-card.css"), ".card { color: red; }").unwrap();

        let options = JsBuildOptions {
            app_dir: dir.path().to_string_lossy().to_string(),
            entry: None,
            css: Some("link".to_string()),
            dom: None,
            css_bundle: None,
            plugin: None,
            components: None,
            component_asset_roots: None,
            metafile: None,
            css_file_name_template: None,
            css_public_base: None,
            legal_comments: None,
            theme: None,
            projection_manifests: None,
            projection_manifest_objects: None,
        };

        let result = build(options).unwrap();
        // css_files is flattened: [filename, content, filename, content, ...]
        assert_eq!(result.css_files.len(), 2);
        assert_eq!(result.css_files[0], "my-card.css");
        assert!(result.css_files[1].contains("color: red"));
        assert_eq!(result.stats.css_file_count, 1);
    }

    #[test]
    fn test_build_with_theme_missing_token_errors() {
        let dir = tempfile::TempDir::new().unwrap();
        std::fs::write(dir.path().join("index.html"), "<my-card></my-card>").unwrap();
        std::fs::write(dir.path().join("my-card.html"), "<div>Card</div>").unwrap();
        std::fs::write(
            dir.path().join("my-card.css"),
            ":host { --token-a: red; --foo-bar: var(--token-a, var(--token-b, var(--token-c))); }",
        )
        .unwrap();
        let theme_path = dir.path().join("theme.json");
        std::fs::write(&theme_path, r#"{"themes":{"light":{"token-b":"green"}}}"#).unwrap();

        let options = JsBuildOptions {
            app_dir: dir.path().to_string_lossy().to_string(),
            entry: None,
            css: Some("link".to_string()),
            dom: None,
            css_bundle: None,
            plugin: None,
            components: None,
            component_asset_roots: None,
            metafile: None,
            css_file_name_template: None,
            css_public_base: None,
            legal_comments: None,
            theme: Some(theme_path.to_string_lossy().to_string()),
            projection_manifests: None,
            projection_manifest_objects: None,
        };

        let Err(err) = build(options) else {
            panic!("missing theme token must fail");
        };
        let message = err.to_string();
        assert!(message.contains("missing-theme-token"), "msg: {message}");
        assert!(message.contains("--token-c"), "msg: {message}");
    }

    #[test]
    fn test_build_returns_component_asset_files() {
        let dir = tempfile::TempDir::new().unwrap();
        std::fs::write(dir.path().join("index.html"), "<app-shell></app-shell>").unwrap();
        std::fs::write(dir.path().join("app-shell.html"), "<div></div>").unwrap();
        std::fs::write(dir.path().join("lazy-panel.html"), "<p>{{title}}</p>").unwrap();
        std::fs::write(dir.path().join("lazy-panel.ts"), "export {};").unwrap();

        let options = JsBuildOptions {
            app_dir: dir.path().to_string_lossy().to_string(),
            entry: None,
            css: Some("link".to_string()),
            dom: None,
            css_bundle: None,
            plugin: Some("webui".to_string()),
            components: None,
            component_asset_roots: Some(vec!["lazy-panel".to_string()]),
            metafile: Some(true),
            css_file_name_template: None,
            css_public_base: None,
            legal_comments: None,
            theme: None,
            projection_manifests: None,
            projection_manifest_objects: None,
        };

        let result = build(options).unwrap();

        assert_eq!(result.component_asset_files.len(), 2);
        assert_eq!(result.component_asset_files[0], "lazy-panel.webui.js");
        assert!(result.component_asset_files[1].contains("webui-component-asset"));
        assert!(result.component_asset_files[1].contains("export default asset;"));
        let metafile = result
            .metafile
            .as_deref()
            .expect("requested metafile must be returned");
        let parsed: serde_json::Value = serde_json::from_str(metafile).unwrap();
        assert!(parsed["inputs"].get("webui:component/lazy-panel").is_some());
        assert!(parsed["outputs"].get("lazy-panel.webui.js").is_some());
    }

    #[test]
    fn test_build_legal_comments_none_strips_legal_css() {
        let dir = tempfile::TempDir::new().unwrap();
        std::fs::write(dir.path().join("index.html"), "<my-card>Hello</my-card>").unwrap();
        std::fs::write(
            dir.path().join("my-card.html"),
            r#"<template shadowrootmode="open"><div>content</div></template>"#,
        )
        .unwrap();
        std::fs::write(
            dir.path().join("my-card.css"),
            "/*! @license MIT */ .card { color: red; }",
        )
        .unwrap();

        let options = JsBuildOptions {
            app_dir: dir.path().to_string_lossy().to_string(),
            entry: None,
            css: Some("link".to_string()),
            dom: None,
            css_bundle: None,
            plugin: None,
            components: None,
            component_asset_roots: None,
            metafile: None,
            css_file_name_template: None,
            css_public_base: None,
            legal_comments: Some("none".to_string()),
            theme: None,
            projection_manifests: None,
            projection_manifest_objects: None,
        };

        let result = build(options).unwrap();
        assert_eq!(result.css_files[1], " .card { color: red; }");
    }

    #[test]
    fn test_build_invalid_legal_comments() {
        let dir = tempfile::TempDir::new().unwrap();
        std::fs::write(dir.path().join("index.html"), "<h1>Hello</h1>").unwrap();

        let options = JsBuildOptions {
            app_dir: dir.path().to_string_lossy().to_string(),
            entry: None,
            css: None,
            dom: None,
            css_bundle: None,
            plugin: None,
            components: None,
            component_asset_roots: None,
            metafile: None,
            css_file_name_template: None,
            css_public_base: None,
            legal_comments: Some("linked".to_string()),
            theme: None,
            projection_manifests: None,
            projection_manifest_objects: None,
        };

        let result = build(options);
        assert!(result.is_err());
    }

    #[test]
    fn test_build_with_unwrapped_component_defaults_to_shadow() {
        let dir = tempfile::TempDir::new().unwrap();
        std::fs::write(dir.path().join("index.html"), "<my-card>Hi</my-card>").unwrap();
        std::fs::write(dir.path().join("my-card.html"), "<div>content</div>").unwrap();

        let options = JsBuildOptions {
            app_dir: dir.path().to_string_lossy().to_string(),
            entry: None,
            css: None,
            dom: None,
            css_bundle: None,
            plugin: None,
            components: None,
            component_asset_roots: None,
            metafile: None,
            css_file_name_template: None,
            css_public_base: None,
            legal_comments: None,
            theme: None,
            projection_manifests: None,
            projection_manifest_objects: None,
        };

        let result = build(options).unwrap();
        let json = inspect(result.protocol).unwrap();
        assert!(json.contains("shadowrootmode"));
    }

    #[test]
    fn test_build_with_light_dom_omits_generated_shadow_root() {
        let dir = tempfile::TempDir::new().unwrap();
        std::fs::write(dir.path().join("index.html"), "<my-card>Hi</my-card>").unwrap();
        std::fs::write(dir.path().join("my-card.html"), "<div>content</div>").unwrap();

        let options = JsBuildOptions {
            app_dir: dir.path().to_string_lossy().to_string(),
            entry: None,
            css: None,
            dom: Some("light".to_string()),
            css_bundle: None,
            plugin: None,
            components: None,
            component_asset_roots: None,
            metafile: None,
            css_file_name_template: None,
            css_public_base: None,
            legal_comments: None,
            theme: None,
            projection_manifests: None,
            projection_manifest_objects: None,
        };

        let result = build(options).unwrap();
        let json = inspect(result.protocol).unwrap();
        assert!(!json.contains("shadowrootmode"));
    }

    #[test]
    fn test_build_with_authored_shadow_dom_preserves_shadow_root_template() {
        let dir = tempfile::TempDir::new().unwrap();
        std::fs::write(dir.path().join("index.html"), "<my-card>Hi</my-card>").unwrap();
        std::fs::write(
            dir.path().join("my-card.html"),
            r#"<template shadowrootmode="open"><div><slot></slot></div></template>"#,
        )
        .unwrap();

        let options = JsBuildOptions {
            app_dir: dir.path().to_string_lossy().to_string(),
            entry: None,
            css: None,
            dom: None,
            css_bundle: None,
            plugin: None,
            components: None,
            component_asset_roots: None,
            metafile: None,
            css_file_name_template: None,
            css_public_base: None,
            legal_comments: None,
            theme: None,
            projection_manifests: None,
            projection_manifest_objects: None,
        };

        let result = build(options).unwrap();
        let json = inspect(result.protocol).unwrap();
        assert!(
            json.contains("shadowrootmode"),
            "authored Shadow DOM wrapper should be preserved, got: {json}"
        );
    }

    #[test]
    fn test_build_rejects_invalid_shadow_wrapper() {
        let dir = tempfile::TempDir::new().unwrap();
        std::fs::write(dir.path().join("index.html"), "<my-card></my-card>").unwrap();
        std::fs::write(
            dir.path().join("my-card.html"),
            r#"<template shadowrootmode="closed"><p>invalid</p></template>"#,
        )
        .unwrap();

        let options = JsBuildOptions {
            app_dir: dir.path().to_string_lossy().to_string(),
            entry: None,
            css: None,
            dom: None,
            css_bundle: None,
            plugin: None,
            components: None,
            component_asset_roots: None,
            metafile: None,
            css_file_name_template: None,
            css_public_base: None,
            legal_comments: None,
            theme: None,
            projection_manifests: None,
            projection_manifest_objects: None,
        };

        let result = build(options);
        assert!(result.is_err());
    }

    #[test]
    fn test_inspect_valid_protocol() {
        let proto = build_protocol("<h1>Hello {{name}}</h1>");
        let json = inspect(napi::bindgen_prelude::Buffer::from(proto)).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert!(parsed.get("fragments").is_some());
    }

    #[test]
    fn test_inspect_invalid_protocol() {
        let result = inspect(napi::bindgen_prelude::Buffer::from(vec![0xFF, 0xFF]));
        assert!(result.is_err());
    }

    #[test]
    fn test_protocol_tokens_empty() {
        let proto = build_protocol("<p>Hello</p>");
        let protocol = Protocol::new(Buffer::from(proto), None).unwrap();
        let tokens = protocol.tokens();
        assert!(tokens.is_empty());
    }

    #[test]
    fn test_protocol_tokens_returns_parsed_tokens() {
        // Build from a protocol that has CSS tokens via with_tokens constructor.
        let mut parser = HtmlParser::new();
        parser.parse("index.html", "<p>Hi</p>").expect("parse");
        let protocol = WebUIProtocol::with_tokens(
            parser.into_fragment_records(),
            vec![
                "colorBrandBackground".to_string(),
                "fontSizeBase300".to_string(),
            ],
        );
        let proto = protocol.to_protobuf().expect("encode");
        let protocol = Protocol::new(Buffer::from(proto), None).unwrap();
        let tokens = protocol.tokens();
        assert_eq!(tokens, vec!["colorBrandBackground", "fontSizeBase300"]);
    }

    #[test]
    fn test_protocol_tokens_invalid_protobuf() {
        let result = Protocol::new(Buffer::from(vec![0xFF, 0xFF]), None);
        assert!(result.is_err());
    }
}

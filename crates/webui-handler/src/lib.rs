// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

//! WebUI Handler implementation for Rust.
//!
//! This crate provides functionality to process and render WebUI protocols
//! into final HTML output based on provided data.

pub mod css_module;
pub(crate) mod html_encode;
pub mod plugin;
pub mod route_handler;
pub mod route_matcher;
pub(crate) mod route_renderer;
pub(crate) mod streaming;

pub use route_handler::Protocol;

/// Minimal HTML escaper for the 6 XSS-critical characters
/// (`& < > " ' /`). Returns `Cow::Borrowed` when no escaping is
/// needed (zero allocation on the happy path), `Cow::Owned` when
/// any character had to be replaced.
///
/// Re-exported here so external callers of `RenderOptions::with_head_inject`
/// / `with_body_inject` can pre-escape untrusted content with the
/// same escaper the handler uses internally for SSR text content,
/// without having to pull in a separate HTML-escape crate.
pub use html_encode::encode_safe;

use plugin::BootstrapExtensionContext;
use plugin::HandlerPlugin;
use plugin::WebUiTemplatePayload;
use route_matcher::CompiledRouteIndex;
use serde::ser::SerializeMap;
use serde::Serialize;
use serde_json::Value;
use std::borrow::Cow;
use std::collections::{HashMap, HashSet};
use streaming::{
    consume_streaming_component_root, ensure_no_pending_streaming_root,
    prepare_generated_streaming_root, record_checkpoint_tag, streaming_template_already_sent,
    validate_pending_streaming_root, validate_streaming_root_opening, ComponentHostOrigin,
    StreamingRenderState,
};
pub use streaming::{
    BoundaryId, BoundaryMode, BufferSink, SessionOptions, StreamingResponse, StreamingSession,
};
use thiserror::Error;
use webui_expressions::{evaluate_with_resolver, ExpressionError};
use webui_protocol::{
    web_ui_fragment::Fragment, InitialStateStrategy, StateProjectionMode, WebUIFragment,
    WebUIProtocol,
};
use webui_state::find_value_by_dotted_path_ref;

/// Error types for the WebUI handler.
#[derive(Debug, Error)]
pub enum HandlerError {
    #[error("Rendering error: {0}")]
    Rendering(String),

    #[error("Rendering invariant error: {0}")]
    Invariant(String),

    #[error("Missing fragment: {0}")]
    MissingFragment(String),

    #[error("Missing data field: {0}")]
    MissingData(String),

    #[error("Type error: {0}")]
    TypeError(String),

    #[error("Protocol error: {0}")]
    Protocol(#[from] webui_protocol::ProtocolError),

    #[error("Evaluation error: {0}")]
    Evaluation(String),

    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    #[error("Writer error: {0}")]
    Writer(String),

    #[error("Plugin data error: {0}")]
    PluginData(String),

    /// The HTTP client disconnected before the render completed.
    ///
    /// Streaming `ResponseWriter` implementations return this from
    /// `write()` once their channel/socket is closed, so the handler
    /// can abort the render rather than do CPU work that has nowhere
    /// to go. Allocation-free (the variant carries no payload).
    #[error("client disconnected")]
    ClientDisconnected,

    /// The streaming writer's flush exceeded its configured deadline.
    ///
    /// Indicates a slow/unresponsive consumer (slow-loris client,
    /// stuck proxy, etc.). The render thread is freed; downstream
    /// telemetry should distinguish this from `ClientDisconnected`
    /// so ops can alert on slow-client attacks.
    #[error("streaming flush timed out")]
    StreamTimeout,

    /// A streaming-boundary signal was malformed or arrived out of order.
    ///
    /// The payload is boxed so this cold, streaming-only variant does not
    /// widen [`HandlerError`] — and therefore `Result<(), HandlerError>`
    /// threaded through the entire hot legacy render path — with two inline
    /// `String`s.
    #[error("invalid streaming boundary signal `{}`: {}", .0.signal, .0.reason)]
    StreamingBoundary(Box<StreamingBoundaryError>),

    /// A streaming render ended without its structural `body_end` signal.
    #[error("streaming render ended before `body_end`; no terminal boundary record was emitted")]
    MissingStreamingBodyEnd,

    /// Streaming initialization cannot be emitted before document content.
    #[error(
        "streaming protocol is missing the required `head_start` signal before `{before}`; \
         rebuild the protocol with streaming-boundary parser support"
    )]
    MissingStreamingHeadStart {
        /// Structural point that made initialization too late.
        before: &'static str,
    },

    /// A malformed protocol emitted streaming initialization more than once.
    #[error("streaming protocol emitted duplicate `head_start` signals")]
    DuplicateStreamingHeadStart,
}

/// Boxed payload for [`HandlerError::StreamingBoundary`].
///
/// Kept behind a `Box` so the streaming-validation variant contributes only a
/// pointer-sized payload to [`HandlerError`], keeping the common
/// `Result<(), HandlerError>` small on the ordinary render path where these
/// boundary errors never occur.
#[derive(Debug)]
pub struct StreamingBoundaryError {
    /// The logical structural token (the internal namespace is stripped).
    pub signal: String,
    /// Actionable validation failure.
    pub reason: String,
}

pub type Result<T> = std::result::Result<T, HandlerError>;

#[cold]
#[inline(never)]
fn invalid_fragment_range_error(
    range: &std::ops::Range<usize>,
    fragment_count: usize,
) -> HandlerError {
    HandlerError::Invariant(format!(
        "fragment range {}..{} exceeds fragment count {fragment_count}",
        range.start, range.end
    ))
}

#[cold]
#[inline(never)]
fn route_style_plan_missing_error() -> HandlerError {
    HandlerError::Invariant(
        "matched route style targets were computed without a route chain".to_string(),
    )
}

#[cold]
#[inline(never)]
fn route_style_plan_length_error(routes: usize, targets: usize) -> HandlerError {
    HandlerError::Invariant(format!(
        "matched route style target count {targets} does not match route count {routes}"
    ))
}

/// Interface for writing rendered output
pub trait ResponseWriter {
    /// Write content to the output
    fn write(&mut self, content: &str) -> Result<()>;

    /// Finalize the output
    fn end(&mut self) -> Result<()>;

    /// Hand buffered bytes to the transport at a committed streaming boundary.
    ///
    /// Internal hook used only by the progressive streaming render path so the
    /// request-local sink can flush its concrete transport without a shared-cell
    /// borrow or a second virtual dispatch on every write. Ordinary writers keep
    /// the no-op default; hosts must not override or depend on this — implement
    /// [`FlushWriter`] and use [`WebUIHandler::render_streaming`] instead.
    #[doc(hidden)]
    fn stream_flush(&mut self) -> Result<()> {
        Ok(())
    }
}

/// A response writer that can hand buffered bytes to its transport immediately.
///
/// Progressive hydration requires this semantic flush at every committed
/// boundary. Hosts that cannot provide it must use [`WebUIHandler::render`].
pub trait FlushWriter: ResponseWriter {
    /// Hand all currently buffered bytes to the underlying transport.
    fn flush(&mut self) -> Result<()>;
}

/// Options controlling how the handler renders a protocol.
///
/// The handler performs server-side route matching: matched routes are rendered
/// visible with content; non-matched routes are rendered hidden and empty.
pub struct RenderOptions<'a> {
    /// The fragment ID to start rendering from (e.g., `"index.html"`).
    pub entry_id: &'a str,
    /// The URL path to match routes against (e.g., `"/contacts/42"`).
    pub request_path: &'a str,
    /// Optional CSP nonce for inline `<script>` tags.
    /// When set, all inline scripts include `nonce="VALUE"` and a
    /// `<meta name="webui-nonce">` tag is emitted for the client router.
    pub nonce: Option<&'a str>,
    /// Optional HTML to emit immediately before the document's
    /// `</head>` close. Used for per-request `<link rel="preload">`
    /// hints, CSP `<meta>` tags beyond the built-in nonce, etc.
    /// Inserted at the structural `head_end` boundary identified by
    /// the parser — never matched against a byte pattern, so cannot
    /// be tricked by `</head>` literals appearing in HTML comments,
    /// `srcdoc` attributes, or inline scripts.
    pub head_inject: Option<&'a str>,
    /// Optional HTML to emit immediately before the document's
    /// `</body>` close. Used for dev livereload `<script>`, analytics
    /// snippets, OpenTelemetry trace IDs, etc.
    /// Same structural-boundary guarantee as [`head_inject`](Self::head_inject).
    pub body_inject: Option<&'a str>,
}

/// Reserved top-level state key carrying host-supplied boundary HTML.
///
/// The leading `$` marks this key as reserved host metadata rather than
/// ordinary application state.
///
/// Recognized members, each an optional string:
///
/// | Member | Emitted at |
/// | --- | --- |
/// | `headEnd` | immediately before `</head>` |
/// | `bodyStart` | immediately after `<body>` |
/// | `bodyEnd` | immediately before `</body>` |
///
/// The key is always honored — it is part of the render state the host
/// already supplies — and is stripped from the client hydration payload.
pub const STATE_INJECT_KEY: &str = "$webui";

/// Host-supplied boundary HTML resolved once per render from
/// [`STATE_INJECT_KEY`].
///
/// Resolution happens when the render context is built, so each structural
/// hook costs one `Option` check instead of a state map lookup. Every field
/// borrows from the caller's state value — no clone, no allocation.
#[derive(Clone, Copy, Default)]
pub(crate) struct StateInject<'state> {
    pub(crate) head_end: Option<&'state str>,
    pub(crate) body_start: Option<&'state str>,
    pub(crate) body_end: Option<&'state str>,
}

impl<'state> StateInject<'state> {
    /// Resolve the reserved namespace from a render state value.
    ///
    /// Returns the empty set when the key is absent, or when any member is
    /// missing, null, empty, or not a string. Malformed input is inert rather than an error: the reserved
    /// key is an optional side channel, and a render must not fail because a
    /// host wrote the wrong shape into it.
    pub(crate) fn resolve(state: &'state Value) -> Self {
        let Some(Value::Object(map)) = state.get(STATE_INJECT_KEY) else {
            return Self::default();
        };
        let field = |name: &str| {
            map.get(name)
                .and_then(Value::as_str)
                .filter(|html| !html.is_empty())
        };
        Self {
            head_end: field("headEnd"),
            body_start: field("bodyStart"),
            body_end: field("bodyEnd"),
        }
    }
}

impl<'a> RenderOptions<'a> {
    /// Create render options for the given entry fragment and request path.
    #[must_use]
    pub fn new(entry_id: &'a str, request_path: &'a str) -> Self {
        Self {
            entry_id,
            request_path,
            nonce: None,
            head_inject: None,
            body_inject: None,
        }
    }

    /// Set the CSP nonce for inline scripts. Pass an empty string to
    /// disable (`None` semantics) — empty `<meta name="webui-nonce"
    /// content="">` would be browser-ignored noise.
    #[must_use]
    pub fn with_nonce(mut self, nonce: &'a str) -> Self {
        self.nonce = if nonce.is_empty() { None } else { Some(nonce) };
        self
    }

    /// Set HTML to emit immediately before `</head>`.
    /// Pass an empty string to disable (`None` semantics).
    ///
    /// # Safety (XSS warning)
    ///
    /// The provided HTML is written verbatim — **no HTML escaping is
    /// performed**. Callers MUST ensure the content is fully trusted
    /// (typically a `&'static str` or build-time-derived bytes such as
    /// dev livereload script, image preload `<link>` tags, or A/B test
    /// markers). Passing user-controlled or attacker-influenced content
    /// here is a direct cross-site scripting vulnerability. If your
    /// caller path may include untrusted data, escape with the host's
    /// HTML escaper (e.g. [`webui_handler::encode_safe`](crate::encode_safe))
    /// **before** calling this builder.
    #[must_use]
    pub fn with_head_inject(mut self, html: &'a str) -> Self {
        self.head_inject = if html.is_empty() { None } else { Some(html) };
        self
    }

    /// Set HTML to emit immediately before `</body>`.
    /// Pass an empty string to disable (`None` semantics).
    ///
    /// # Safety (XSS warning)
    ///
    /// Same contract as [`with_head_inject`](Self::with_head_inject):
    /// the HTML is written verbatim with **no escaping**, so callers
    /// MUST ensure the content is fully trusted. Untrusted content is
    /// a direct XSS vector.
    #[must_use]
    pub fn with_body_inject(mut self, html: &'a str) -> Self {
        self.body_inject = if html.is_empty() { None } else { Some(html) };
        self
    }
}

/// The main WebUI handler that processes protocols and renders them.
///
/// The handler is stateless: plugin instances are created per-render from
/// the stored factory function, allowing concurrent renders with `&self`.
pub struct WebUIHandler {
    plugin_factory: Option<fn() -> Box<dyn HandlerPlugin>>,
}

#[derive(Clone, Copy, Eq, PartialEq)]
enum StyleClosureInstall {
    Static,
    Routed,
}

pub(crate) struct ShadowStyleRoot {
    component_index: u32,
    static_closure_emitted: bool,
    routed_resources: Vec<u32>,
}

/// Context object for processing WebUI fragments
pub(crate) struct WebUIProcessContext<'protocol, 'state, 'output> {
    pub(crate) protocol: &'protocol WebUIProtocol,
    pub(crate) state: &'state Value,
    pub(crate) writer: &'output mut dyn ResponseWriter,
    pub(crate) local_vars: HashMap<String, Value>,
    /// Accumulates component attribute values between attrStart and the component fragment.
    pub(crate) component_attrs: HashMap<String, Value>,
    /// URL path for server-side route matching. Borrowed from
    /// `RenderOptions<'a>::request_path` — zero-copy.
    pub(crate) request_path: &'protocol str,
    /// Base path for resolving relative route paths (`./`).
    /// Updated as the handler descends into nested matched routes.
    /// `Cow` keeps the initial `"/"` literal zero-copy; nested-route
    /// descent owns the recomputed path.
    pub(crate) route_base: Cow<'protocol, str>,
    /// Component names visited during rendering (for selective f-template emission
    /// and CSS module dedup — only the first render of each component emits
    /// its `<script type="importmap">` data-URI tag).
    pub(crate) rendered_components: HashSet<String>,
    /// Per-render plugin instance created from the handler's factory.
    pub(crate) plugin: Option<Box<dyn HandlerPlugin>>,
    /// Current position in the route tree for outlet-based rendering.
    /// Contains the children of the currently matched route fragment.
    pub(crate) route_children: Vec<webui_protocol::WebUiFragmentRoute>,
    /// Entry fragment ID — used to compute the initial inventory at head_end.
    /// Borrowed from `RenderOptions<'a>::entry_id` — zero-copy.
    pub(crate) entry_id: &'protocol str,
    /// CSP nonce for inline `<script>` tags (None = no nonce attribute).
    /// Borrowed from `RenderOptions<'a>::nonce` — zero-copy.
    pub(crate) nonce: Option<&'protocol str>,
    /// Component-name → bit-position map built once when the runtime
    /// [`Protocol`] is created and shared by every render.
    pub(crate) component_index: &'protocol HashMap<String, u32>,
    /// Style-resource ID → request-local dedup bit built with [`Protocol`].
    pub(crate) style_resource_index: &'protocol HashMap<String, u32>,
    /// Component tag → covering bundle chunk, built once per render.
    ///
    /// Empty (and allocation-free) for unbundled builds. Every style delivery
    /// path resolves closure members through this so they all agree on what a
    /// chunk already ships.
    pub(crate) style_chunk_index: HashMap<&'protocol str, u32>,
    /// CSS strategy declared by the compiled protocol.
    pub(crate) css_strategy: webui_protocol::CssStrategy,
    /// HTML emitted at the structural `head_end` boundary (before
    /// `</head>`), after the built-in nonce/CSS emissions.
    /// Zero-copy borrow of the caller's `RenderOptions<'a>::head_inject`
    /// (no per-render clone — saves an allocation when the host passes
    /// a `&'static str` such as a dev livereload script).
    pub(crate) head_inject: Option<&'protocol str>,
    /// HTML emitted at the structural `body_end` boundary (before
    /// `</body>`), after the built-in template metadata emissions.
    /// Same zero-copy borrow as [`head_inject`](Self::head_inject).
    pub(crate) body_inject: Option<&'protocol str>,
    /// Host HTML resolved once per render from the reserved
    /// [`STATE_INJECT_KEY`] state namespace. Emitted after the built-in
    /// emissions at each structural boundary, and after the corresponding
    /// `RenderOptions` inject at `head_end` and `body_end`.
    pub(crate) state_inject: StateInject<'state>,
    /// Tracks whether the `head_end` hook has already fired in this
    /// render. Defends against malformed protocols that emit the
    /// signal more than once (e.g., a template with multiple `<head>`
    /// tags) — without this, host-supplied `head_inject` HTML, CSS
    /// resources, and the CSP `<meta>` nonce would be
    /// duplicated, which can be a CSP-bypass / cache-bloat vector.
    pub(crate) head_end_emitted: bool,
    /// Tracks whether the `body_start` hook has already fired in this
    /// render. Defends against malformed protocols emitting the signal
    /// twice — without this, state-supplied `bodyStart` HTML would be
    /// duplicated.
    pub(crate) body_start_emitted: bool,
    /// Tracks whether the `body_end` hook has already fired in this
    /// render. Defends against malformed protocols emitting the
    /// signal twice — without this, hydration `<script>` blocks and
    /// host-supplied `body_inject` would be duplicated.
    pub(crate) body_end_emitted: bool,
    /// Immutable authored route patterns compiled when [`Protocol`] is loaded.
    pub(crate) route_index: &'protocol CompiledRouteIndex,
    /// Counter for `data-ri` attributes on matched route elements.
    /// Incremented each time a matched route is rendered, allowing O(1) element
    /// binding on the client side instead of DOM-walking.
    pub(crate) route_chain_index: usize,
    /// Matched route metadata computed at most once when head styles or body
    /// bootstrap first need it, then shared by streaming checkpoints.
    pub(crate) route_chain: Option<Vec<crate::route_handler::RouteChainEntry>>,
    /// Whether each matched route closure installs into the Document CSS tree.
    ///
    /// Kept parallel to `route_chain`; the route graph computes both in one walk.
    pub(crate) route_document_style_targets: Vec<bool>,
    /// Request-reachable components in deterministic first-discovery order.
    ///
    /// Resolved once at `head_end` for Shadow Link preloads, then consumed at
    /// `body_end` for hydration metadata so the fragment graph is not walked
    /// twice per request.
    pub(crate) reachable_components: Option<Vec<String>>,
    /// Present only for the opt-in progressive streaming render path.
    pub(crate) streaming: Option<&'output mut StreamingRenderState<'protocol>>,
    /// Reusable JSON serialization scratch buffer, owned by the render context.
    /// The bootstrap/field/template helpers borrow it so each render reuses one
    /// buffer across every serialized field and every streaming checkpoint
    /// instead of allocating per value. It grows lazily on first serialization
    /// and is dropped with the context at request end — no per-thread
    /// high-water buffer is retained between requests.
    pub(crate) json_scratch: Vec<u8>,
    /// Small request-local pool of cleared scope maps reused across sibling
    /// component roots. `process_component` recycles each finished component's
    /// local/attr map here instead of dropping it, so a sibling reuses the
    /// bucket capacity rather than reallocating a fresh `HashMap`. Bounded
    /// ([`SCOPE_POOL_CAP`]) and dropped with the context at request end.
    pub(crate) scope_pool: Vec<HashMap<String, Value>>,
    /// Resources delivered into the Document CSS tree. The empty set does not
    /// allocate, and streaming retains it across checkpoints.
    pub(crate) document_style_resources: HashSet<String>,
    /// Active Shadow roots and their route-activated resource indexes. The
    /// route vector allocates only when a matched Light route contributes CSS.
    pub(crate) shadow_style_roots: Vec<ShadowStyleRoot>,
}

/// Compiler-owned signal namespace. The leading `}}}` cannot be produced by
/// authored double- or triple-brace expressions because it closes the binding.
pub(crate) const STRUCTURAL_SIGNAL_PREFIX: &str = "}}}webui:";

pub(crate) fn structural_signal_value(
    signal: &webui_protocol::WebUIFragmentSignal,
) -> Option<&str> {
    if !signal.raw {
        return None;
    }
    signal.value.strip_prefix(STRUCTURAL_SIGNAL_PREFIX)
}

/// Find the end of a leading doctype without treating quoted `>` bytes in
/// legacy PUBLIC or SYSTEM identifiers as the declaration close.
fn doctype_prefix_end(raw: &str) -> Option<usize> {
    const PREFIX: &[u8] = b"<!doctype";

    let bytes = raw.as_bytes();
    let mut cursor = usize::from(bytes.starts_with(b"\xEF\xBB\xBF")) * 3;
    while bytes
        .get(cursor)
        .is_some_and(|byte| byte.is_ascii_whitespace())
    {
        cursor += 1;
    }

    let prefix_end = cursor.checked_add(PREFIX.len())?;
    if !bytes.get(cursor..prefix_end)?.eq_ignore_ascii_case(PREFIX) {
        return None;
    }
    let separator = *bytes.get(prefix_end)?;
    if separator != b'>' && !separator.is_ascii_whitespace() {
        return None;
    }

    let mut quote = None;
    for (offset, byte) in bytes[prefix_end..].iter().copied().enumerate() {
        match quote {
            Some(active) if byte == active => quote = None,
            Some(_) => {}
            None if matches!(byte, b'\'' | b'"') => quote = Some(byte),
            None if byte == b'>' => return Some(prefix_end + offset + 1),
            None => {}
        }
    }
    None
}

/// Maximum scope maps retained in the request-local pool. Small: sibling
/// component roots rarely nest deeply, so the cap keeps retained capacity bounded.
const SCOPE_POOL_CAP: usize = 8;

/// Take a cleared scope map from the pool, or a fresh empty one when the pool is
/// empty. A fresh `HashMap` does not allocate until its first insert.
fn take_scope_map(pool: &mut Vec<HashMap<String, Value>>) -> HashMap<String, Value> {
    pool.pop().unwrap_or_default()
}

/// Return a spent scope map to the pool, clearing it but retaining its bucket
/// capacity for a sibling root to reuse. Drops the map once the pool is full so
/// retained memory stays bounded.
fn recycle_scope_map(pool: &mut Vec<HashMap<String, Value>>, mut map: HashMap<String, Value>) {
    if pool.len() < SCOPE_POOL_CAP {
        map.clear();
        pool.push(map);
    }
}

pub(crate) struct WebUiBootstrap<'a> {
    pub(crate) state: &'a Value,
    pub(crate) state_selection: StateSelection<'a>,
    pub(crate) chain: &'a [Value],
    pub(crate) inventory: &'a str,
    pub(crate) nonce: Option<&'a str>,
    pub(crate) css_hrefs: &'a [&'a str],
    pub(crate) style_specs: &'a [&'a str],
    pub(crate) component_styles: &'a Value,
    pub(crate) templates: &'a [WebUiTemplatePayload<'a>],
}

/// Get the component attribute name, stripping `:` prefix and converting to camelCase.
///
/// Uses `webui_protocol::attrs::attribute_to_camel` which handles irregular
/// attributes (multi-word ARIA and global HTML attributes like `readonly`,
/// `tabindex`) via the shared lookup table.
fn component_attr_name(name: &str) -> String {
    let stripped = name.strip_prefix(':').unwrap_or(name);
    webui_protocol::attrs::attribute_to_camel(stripped)
}

/// Write a usize as decimal digits directly to the writer, avoiding `format!` allocation.
pub(crate) fn write_usize(writer: &mut dyn ResponseWriter, mut n: usize) -> Result<()> {
    if n == 0 {
        return writer.write("0");
    }
    // Max digits for a 64-bit usize is 20.
    let mut buf = [0u8; 20];
    let mut pos = buf.len();
    while n > 0 {
        pos -= 1;
        // n % 10 is always in 0..=9, fits in u8 without truncation.
        #[allow(clippy::cast_possible_truncation)]
        let digit = (n % 10) as u8;
        buf[pos] = b'0' + digit;
        n /= 10;
    }
    // Digits are always valid ASCII/UTF-8.
    match std::str::from_utf8(&buf[pos..]) {
        Ok(s) => writer.write(s),
        Err(_) => writer.write("0"),
    }
}

pub(crate) fn write_script_safe_json<T>(
    writer: &mut dyn ResponseWriter,
    scratch: &mut Vec<u8>,
    value: &T,
) -> Result<()>
where
    T: Serialize + ?Sized,
{
    // Serialize into the caller's request-local `scratch`. The streaming path
    // emits one bootstrap envelope per committed boundary and the ordinary
    // body_end bootstrap serializes several fields, so reusing one buffer across
    // the render avoids a fresh allocation per value. The buffer grows lazily on
    // first use (no allocation until serialization needs it) and is dropped with
    // the render context — capacity is reused within a request but never
    // retained across requests.
    scratch.clear();
    serde_json::to_writer(&mut *scratch, value)
        .map_err(|error| HandlerError::Rendering(format!("failed to serialize JSON: {error}")))?;
    let json = std::str::from_utf8(scratch)
        .map_err(|error| HandlerError::Rendering(format!("invalid JSON UTF-8: {error}")))?;
    write_script_safe_json_str(writer, json)
}

fn write_script_safe_json_str(writer: &mut dyn ResponseWriter, json: &str) -> Result<()> {
    let mut start = 0;
    while start < json.len() {
        let rest = &json[start..];
        let Some(offset) = rest.find("</") else {
            writer.write(rest)?;
            return Ok(());
        };

        if offset > 0 {
            writer.write(&rest[..offset])?;
        }
        writer.write("<\\/")?;
        start += offset + 2;
    }
    Ok(())
}

fn write_json_field_name(
    writer: &mut dyn ResponseWriter,
    wrote_field: &mut bool,
    name: &str,
) -> Result<()> {
    if *wrote_field {
        writer.write(",")?;
    }
    *wrote_field = true;
    writer.write("\"")?;
    writer.write(name)?;
    writer.write("\":")
}

fn write_json_field<T>(
    writer: &mut dyn ResponseWriter,
    scratch: &mut Vec<u8>,
    wrote_field: &mut bool,
    name: &str,
    value: &T,
) -> Result<()>
where
    T: Serialize + ?Sized,
{
    write_json_field_name(writer, wrote_field, name)?;
    write_script_safe_json(writer, scratch, value)
}

/// Serialize wrapper that projects an SSR state object down to only the
/// keys present in the build-time hydration allowlist.
///
/// This is the runtime half of the projected-hydration design: instead of
/// serializing the entire application state (potentially megabytes) on every
/// full-HTML render, only the fields a component actually hydrates are
/// emitted. The request allowlist conservatively includes every reachable
/// component's hydration keys so no field a component needs is dropped.
///
/// Projection is a payload boundary, not a secrecy boundary. Any key selected
/// by compiled client metadata is browser-facing, so hosts must never place
/// secrets in browser render state.
///
/// `keys` MUST be sorted and deduplicated. Projection iterates whichever side
/// is smaller: hydration keys with direct map lookup for wide states, or state
/// entries with binary-search membership for compact states. Non-object states
/// carry nothing hydratable and serialize as an empty object.
struct ProjectedState<'a> {
    value: &'a Value,
    keys: &'a [&'a str],
}

impl Serialize for ProjectedState<'_> {
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        let Value::Object(map) = self.value else {
            return serializer.serialize_map(Some(0))?.end();
        };

        let mut out = serializer.serialize_map(None)?;
        if self.keys.len() < map.len() {
            let mut previous = None;
            for key in self.keys {
                if *key == STATE_INJECT_KEY {
                    continue;
                }
                if previous == Some(*key) {
                    continue;
                }
                previous = Some(*key);
                if let Some(value) = map.get(*key) {
                    out.serialize_entry(key, value)?;
                }
            }
        } else {
            for (key, value) in map {
                if key == STATE_INJECT_KEY {
                    continue;
                }
                if self
                    .keys
                    .binary_search_by(|candidate| candidate.cmp(&key.as_str()))
                    .is_ok()
                {
                    out.serialize_entry(key, value)?;
                }
            }
        }
        out.end()
    }
}

/// Write the SSR `state` into the bootstrap block according to the protocol's
/// build-time selection and escape it for safe embedding in a `<script>`.
///
/// [`ProjectedState`] serializes only the allowlisted keys, so for the typical
/// payload — a large state with a small hydratable surface — serde ever only
/// touches the projected subset. Serialization reuses the proven
/// [`write_script_safe_json`] path (serde's fast `Vec<u8>` target plus a single
/// SIMD-accelerated `</` escape pass), which matches the pre-projection cost
/// when every key is hydratable and collapses to a few bytes when it is not.
/// Buffering the projected bytes and escaping once is measurably faster than
/// streaming through a per-token `io::Write` adapter, and the projected buffer
/// is tiny in the common case.
/// Serialize a state object with the reserved [`STATE_INJECT_KEY`] entry
/// omitted.
///
/// The reserved key carries host-supplied boundary HTML, not application
/// state, so it must never reach the client hydration payload — shipping it
/// would both duplicate the markup as a JSON string and expose the host's
/// inject channel to client code. Filtering happens during serialization so
/// the state tree is never cloned.
struct StateWithoutReservedKey<'a> {
    value: &'a Value,
}

impl Serialize for StateWithoutReservedKey<'_> {
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        let Value::Object(map) = self.value else {
            return self.value.serialize(serializer);
        };
        let mut out = serializer.serialize_map(Some(map.len().saturating_sub(1)))?;
        for (key, value) in map {
            if key == STATE_INJECT_KEY {
                continue;
            }
            out.serialize_entry(key, value)?;
        }
        out.end()
    }
}

/// Write the complete state, stripping the reserved inject key when present.
///
/// The membership test is a single map lookup and the common case — no
/// reserved key — takes the original zero-overhead path unchanged.
fn write_full_state(
    writer: &mut dyn ResponseWriter,
    scratch: &mut Vec<u8>,
    state: &Value,
) -> Result<()> {
    if matches!(state, Value::Object(map) if map.contains_key(STATE_INJECT_KEY)) {
        return write_script_safe_json(writer, scratch, &StateWithoutReservedKey { value: state });
    }
    write_script_safe_json(writer, scratch, state)
}

pub(crate) fn write_selected_state(
    writer: &mut dyn ResponseWriter,
    scratch: &mut Vec<u8>,
    state: &Value,
    selection: &StateSelection<'_>,
) -> Result<()> {
    let keys = match selection {
        StateSelection::Full => return write_full_state(writer, scratch, state),
        StateSelection::Keys(keys) => keys.as_slice(),
        StateSelection::BorrowedKeys(keys) => *keys,
    };
    if keys.is_empty() {
        return writer.write("{}");
    }

    // Projection membership may use binary search, so a mis-sorted key set
    // would silently drop hydration keys. The key allowlist is produced sorted +
    // deduped at build time; this guard makes hand-built protocols that violate
    // the invariant fail loudly in tests at zero release cost.
    debug_assert!(
        keys.windows(2).all(|pair| pair[0] <= pair[1]),
        "hydration keys must be sorted for binary-search projection"
    );
    if let Value::Object(map) = state {
        let selects_entire_map =
            keys.len() == map.len() && keys.iter().copied().eq(map.keys().map(String::as_str));
        if selects_entire_map {
            return write_full_state(writer, scratch, state);
        }
    }
    write_script_safe_json(writer, scratch, &ProjectedState { value: state, keys })
}

// Covers the common route surface without trusting protocol-derived counts for
// an eager allocation; larger key sets grow only as actual keys are visited.
pub(crate) const INITIAL_KEY_CAPACITY: usize = 16;

/// Request-scoped state selection derived from reachable component metadata.
pub(crate) enum StateSelection<'a> {
    /// Preserve the complete state value.
    Full,
    /// Project an object to a sorted, deduplicated key allowlist.
    Keys(Vec<&'a str>),
    /// Project using request-local scratch owned by the streaming render.
    BorrowedKeys(&'a [&'a str]),
}

#[derive(Clone, Copy)]
enum ComponentStateSurface {
    Hydration,
    Navigation,
}

/// Select initial state for the components reachable on this request path.
///
/// Non-WebUI protocols preserve full state without walking component surfaces.
/// WebUI protocols project exact surfaces, while any unknown surface restores
/// the full state for correctness.
pub(crate) fn collect_hydration_state<'a, 'b>(
    protocol: &'a WebUIProtocol,
    components: impl IntoIterator<Item = &'b str>,
) -> StateSelection<'a> {
    if protocol.initial_state_strategy != InitialStateStrategy::Components as i32 {
        return StateSelection::Full;
    }
    collect_component_state(protocol, components, ComponentStateSurface::Hydration)
}

pub(crate) fn collect_hydration_state_into<'a, 'b>(
    protocol: &'a WebUIProtocol,
    components: impl IntoIterator<Item = &'b str>,
    keys: &mut Vec<&'a str>,
) -> bool {
    if protocol.initial_state_strategy != InitialStateStrategy::Components as i32 {
        keys.clear();
        return true;
    }
    collect_component_state_into(protocol, components, ComponentStateSurface::Hydration, keys)
}

/// Select state for client-created components reachable during navigation.
pub(crate) fn collect_navigation_state<'a, 'b>(
    protocol: &'a WebUIProtocol,
    components: impl IntoIterator<Item = &'b str>,
) -> StateSelection<'a> {
    collect_component_state(protocol, components, ComponentStateSurface::Navigation)
}

fn collect_component_state<'a, 'b>(
    protocol: &'a WebUIProtocol,
    components: impl IntoIterator<Item = &'b str>,
    surface: ComponentStateSurface,
) -> StateSelection<'a> {
    let mut keys = Vec::with_capacity(INITIAL_KEY_CAPACITY);
    if collect_component_state_into(protocol, components, surface, &mut keys) {
        StateSelection::Full
    } else {
        StateSelection::Keys(keys)
    }
}

/// Fill a reusable state-key allowlist. Returns `true` when correctness
/// requires sending full state instead of the collected keys.
fn collect_component_state_into<'a, 'b>(
    protocol: &'a WebUIProtocol,
    components: impl IntoIterator<Item = &'b str>,
    surface: ComponentStateSurface,
    keys: &mut Vec<&'a str>,
) -> bool {
    keys.clear();
    for name in components {
        let Some(component) = protocol.components.get(name) else {
            return true;
        };
        let (mode, component_keys) = match surface {
            ComponentStateSurface::Hydration => {
                (component.hydration_mode, &component.hydration_keys)
            }
            ComponentStateSurface::Navigation => (
                match component.navigation_mode {
                    Some(mode) => mode,
                    None if !component.navigation_keys.is_empty() => {
                        StateProjectionMode::Keys as i32
                    }
                    None => return true,
                },
                &component.navigation_keys,
            ),
        };
        if mode == StateProjectionMode::All as i32 {
            return true;
        }
        if mode == StateProjectionMode::Keys as i32
            || (mode == StateProjectionMode::None as i32 && !component_keys.is_empty())
        {
            keys.extend(component_keys.iter().map(String::as_str));
        } else if mode != StateProjectionMode::None as i32 {
            return true;
        }
    }
    keys.sort_unstable();
    keys.dedup();
    false
}

pub(crate) fn write_webui_bootstrap(
    writer: &mut dyn ResponseWriter,
    scratch: &mut Vec<u8>,
    bootstrap: WebUiBootstrap<'_>,
) -> Result<()> {
    let mut wrote_field = false;

    writer.write("{")?;
    if !bootstrap.chain.is_empty() {
        write_json_field(writer, scratch, &mut wrote_field, "chain", bootstrap.chain)?;
    }
    // Definitions must be visible before template metadata can cause the
    // runtime to create a component root.
    write_json_field(
        writer,
        scratch,
        &mut wrote_field,
        "componentStyles",
        bootstrap.component_styles,
    )?;
    if !bootstrap.css_hrefs.is_empty() {
        write_json_field(
            writer,
            scratch,
            &mut wrote_field,
            "css",
            bootstrap.css_hrefs,
        )?;
    }
    write_json_field(
        writer,
        scratch,
        &mut wrote_field,
        "inventory",
        bootstrap.inventory,
    )?;
    if let Some(nonce) = bootstrap.nonce {
        write_json_field(writer, scratch, &mut wrote_field, "nonce", nonce)?;
    }
    write_json_field_name(writer, &mut wrote_field, "state")?;
    write_selected_state(writer, scratch, bootstrap.state, &bootstrap.state_selection)?;
    if !bootstrap.style_specs.is_empty() {
        write_json_field(
            writer,
            scratch,
            &mut wrote_field,
            "styles",
            bootstrap.style_specs,
        )?;
    }
    if bootstrap
        .templates
        .iter()
        .any(|template| !template.template_json.is_empty())
    {
        write_json_field_name(writer, &mut wrote_field, "templates")?;
        write_webui_template_json_map(writer, scratch, bootstrap.templates)?;
    }
    writer.write("}")
}

fn write_webui_data_block(
    writer: &mut dyn ResponseWriter,
    scratch: &mut Vec<u8>,
    bootstrap: WebUiBootstrap<'_>,
) -> Result<()> {
    writer.write("<script type=\"application/json\" id=\"webui-data\"")?;
    if let Some(nonce) = bootstrap.nonce {
        writer.write(" nonce=\"")?;
        writer.write(nonce)?;
        writer.write("\"")?;
    }
    writer.write(">")?;
    write_webui_bootstrap(writer, scratch, bootstrap)?;
    writer.write("</script>\n")
}

fn write_webui_template_json_map(
    writer: &mut dyn ResponseWriter,
    scratch: &mut Vec<u8>,
    templates: &[WebUiTemplatePayload<'_>],
) -> Result<()> {
    writer.write("{")?;
    let mut wrote = false;
    for template in templates {
        if template.template_json.is_empty() {
            continue;
        }
        if wrote {
            writer.write(",")?;
        }
        wrote = true;
        write_script_safe_json(writer, scratch, template.tag_name)?;
        writer.write(":")?;
        write_script_safe_json_str(writer, template.template_json)?;
    }
    writer.write("}")
}

fn resolve_value_from_sources<'ctx, 'state>(
    path: &str,
    local_vars: &'ctx HashMap<String, Value>,
    state: &'state Value,
) -> Option<Cow<'ctx, Value>>
where
    'state: 'ctx,
{
    if let Some(first_part) = path.split('.').next() {
        if let Some(local_value) = local_vars.get(first_part) {
            if first_part.len() == path.len() {
                return Some(Cow::Borrowed(local_value));
            }
            let remaining = &path[first_part.len() + 1..];
            if let Some(value) = find_value_by_dotted_path_ref(remaining, local_value) {
                return Some(value);
            }
        }
    }

    find_value_by_dotted_path_ref(path, state)
}

impl WebUIHandler {
    /// Create a new WebUI handler with no plugin.
    pub fn new() -> Self {
        Self {
            plugin_factory: None,
        }
    }

    /// Create a new WebUI handler with a plugin factory.
    ///
    /// Each render call creates a fresh plugin instance from the factory,
    /// enabling concurrent renders with `&self`.
    pub fn with_plugin(factory: fn() -> Box<dyn HandlerPlugin>) -> Self {
        Self {
            plugin_factory: Some(factory),
        }
    }

    #[cfg(test)]
    fn handle(
        &self,
        document: &WebUIProtocol,
        state: &Value,
        options: &RenderOptions<'_>,
        writer: &mut dyn ResponseWriter,
    ) -> Result<()> {
        let protocol = Protocol::new(document.clone());
        self.render(&protocol, state, options, writer)
    }

    /// Process a fragment by its ID.
    ///
    /// The `context` parameter contains scope-local variables that are accessible during rendering,
    /// such as loop iteration variables. This is separate from the global `state`.
    fn process_fragment_id<'data>(
        &self,
        fragment_id: &str,
        context: &mut WebUIProcessContext<'data, '_, '_>,
    ) -> Result<()> {
        if let Some(fragment_list) = context.protocol.fragments.get(fragment_id) {
            self.process_fragment(&fragment_list.fragments, context)
        } else {
            Err(HandlerError::MissingFragment(fragment_id.to_string()))
        }
    }

    /// Process a vector of fragments.
    ///
    /// The `context` maintains scope-specific variables that can be accessed by fragments
    /// during rendering, while `state` contains the global application state.
    fn process_fragment<'data>(
        &self,
        fragments: &'data [WebUIFragment],
        context: &mut WebUIProcessContext<'data, '_, '_>,
    ) -> Result<()> {
        self.process_fragment_from(fragments, 0, context)
    }

    fn process_fragment_from<'data>(
        &self,
        fragments: &'data [WebUIFragment],
        start: usize,
        context: &mut WebUIProcessContext<'data, '_, '_>,
    ) -> Result<()> {
        // Pre-scan: find the best matching route among sibling routes by specificity.
        // This ensures `/contacts/add` (2 literals) beats `/contacts/:id` (1 literal).
        // Resolves relative paths (`./`) using the current route_base.
        let best_route = route_renderer::find_best_route_match(
            fragments,
            context.request_path,
            &context.route_base,
            context.route_index,
        );
        self.process_fragment_range(fragments, start..fragments.len(), &best_route, context)
    }

    fn process_fragment_range<'data>(
        &self,
        fragments: &'data [WebUIFragment],
        range: std::ops::Range<usize>,
        best_route: &Option<(String, route_matcher::RouteMatch)>,
        context: &mut WebUIProcessContext<'data, '_, '_>,
    ) -> Result<()> {
        let Some(selected) = fragments.get(range.clone()) else {
            return Err(invalid_fragment_range_error(&range, fragments.len()));
        };
        for (offset, item) in selected.iter().enumerate() {
            let index = range.start + offset;
            if context.streaming.is_some() {
                validate_pending_streaming_root(item, context)?;
                validate_streaming_root_opening(&fragments[..index], item)?;
            }
            match item.fragment.as_ref() {
                Some(Fragment::Raw(raw)) => {
                    context.writer.write(&raw.value)?;
                }
                Some(Fragment::Component(component)) => {
                    self.process_component(
                        component,
                        ComponentHostOrigin::ParserProduced,
                        context,
                    )?;
                }
                Some(Fragment::ForLoop(for_loop)) => {
                    self.process_for_loop(for_loop, context)?;
                }
                Some(Fragment::Signal(signal)) => {
                    self.process_signal(signal, context)?;
                }
                Some(Fragment::IfCond(if_cond)) => {
                    self.process_if(if_cond, context)?;
                }
                Some(Fragment::Attribute(attr)) => {
                    self.process_attribute(attr, context)?;
                }
                Some(Fragment::Plugin(plugin_frag)) => {
                    if let Some(p) = &mut context.plugin {
                        p.on_element_data(&plugin_frag.data, context.writer)?;
                    }
                }
                Some(Fragment::Route(route_frag)) => {
                    self.process_route(route_frag, best_route, context)?;
                }
                Some(Fragment::Outlet(_)) => {
                    self.process_outlet(context)?;
                }
                None => {}
            }
        }
        ensure_no_pending_streaming_root(context, "the end of the containing fragment")
    }

    /// Process an `<outlet />` directive.
    ///
    /// Matches children from the currently active route's `children` field
    /// against the request path, renders the matched child `<webui-route>`
    /// elements directly at this position (no wrapper element).
    fn process_outlet(&self, context: &mut WebUIProcessContext) -> Result<()> {
        let mut children = std::mem::take(&mut context.route_children);
        if children.is_empty() {
            return Ok(());
        }

        // Find the best matching child route
        let request_segments = route_matcher::split_request_path(context.request_path);
        let mut best: Option<(usize, route_matcher::RouteMatch)> = None;
        for (idx, child) in children.iter().enumerate() {
            if let Some(m) = route_matcher::match_route_indexed_with_segments(
                context.route_index,
                &child.path,
                &context.route_base,
                &request_segments,
                child.exact,
            ) {
                let is_better = best
                    .as_ref()
                    .is_none_or(|(_, prev)| m.specificity > prev.specificity);
                if is_better {
                    best = Some((idx, m));
                }
            }
        }

        // Extract grandchildren from the matched child to avoid cloning.
        // We swap out the children vec so we can move it into context without
        // cloning, then swap an empty vec back for the sibling rendering pass.
        let grandchildren = if let Some((idx, _)) = &best {
            std::mem::take(&mut children[*idx].children)
        } else {
            Vec::new()
        };

        if let Some((idx, ref rm)) = best {
            let matched_child = &children[idx];
            let comp = &matched_child.fragment_id;

            if !comp.is_empty() {
                let saved_route_base = context.route_base.clone();
                let saved_route_children = std::mem::take(&mut context.route_children);

                if rm.consumed_segments > 0 {
                    context.route_base = Cow::Owned(route_matcher::compute_route_base(
                        context.request_path,
                        rm.consumed_segments,
                    ));
                }

                context.route_children = grandchildren;

                // Emit matched <webui-route>
                context.writer.write("<webui-route")?;
                if !matched_child.path.is_empty() {
                    context.writer.write(" path=\"")?;
                    context.writer.write(&matched_child.path)?;
                    context.writer.write("\"")?;
                }
                context.writer.write(" component=\"")?;
                context.writer.write(comp)?;
                context.writer.write("\"")?;
                if matched_child.exact {
                    context.writer.write(" exact")?;
                }
                route_renderer::write_route_navigation_attrs(context.writer, matched_child)?;
                // Emit data-ri for O(1) client-side element binding
                let ri = context.route_chain_index;
                context.route_chain_index += 1;
                context.writer.write(" data-ri=\"")?;
                write_usize(context.writer, ri)?;
                context.writer.write("\" active>")?;

                if !Self::component_owns_css_tree(comp, context.protocol) {
                    self.emit_component_style_closure(comp, StyleClosureInstall::Routed, context)?;
                }
                context.writer.write("<")?;
                context.writer.write(comp)?;
                self.write_light_dom_marker(comp, context)?;
                if let Some(p) = &context.plugin {
                    p.write_route_component_state(context.state, context.writer)?;
                }
                prepare_generated_streaming_root(comp, context)?;
                context.writer.write(">")?;

                self.process_component(
                    &webui_protocol::WebUIFragmentComponent {
                        fragment_id: comp.clone(),
                    },
                    ComponentHostOrigin::HandlerGenerated,
                    context,
                )?;

                context.writer.write("</")?;
                context.writer.write(comp)?;
                context.writer.write(">")?;
                context.writer.write("</webui-route>")?;

                context.route_base = saved_route_base;
                context.route_children = saved_route_children;
            }
        }

        // Render non-matched siblings as hidden
        for (idx, child) in children.iter().enumerate() {
            let is_matched = best.as_ref().is_some_and(|(bi, _)| *bi == idx);
            if !is_matched && !child.fragment_id.is_empty() {
                context.writer.write("<webui-route")?;
                if !child.path.is_empty() {
                    context.writer.write(" path=\"")?;
                    context.writer.write(&child.path)?;
                    context.writer.write("\"")?;
                }
                context.writer.write(" component=\"")?;
                context.writer.write(&child.fragment_id)?;
                context.writer.write("\"")?;
                if child.exact {
                    context.writer.write(" exact")?;
                }
                route_renderer::write_route_navigation_attrs(context.writer, child)?;
                context
                    .writer
                    .write(" style=\"display:none\"></webui-route>")?;
            }
        }

        Ok(())
    }

    /// Emit a `<script type="importmap">` tag that registers a component's
    /// CSS module under its specifier via a `data:text/css,…` URI.
    ///
    /// Requires Multiple Import Maps (Chrome 133+); each call emits an
    /// independent importmap that the browser merges at the document
    /// level. The per-render CSP nonce is applied when set (importmap
    /// scripts honor `script-src`).
    ///
    /// Example for `my-comp` with CSS `span{color:blue;}`:
    /// `<script type="importmap" nonce="..." data-webui-resource="my-comp">{"imports":{"my-comp":"data:text/css,span{color:blue;}"}}</script>`
    fn emit_css_module_importmap(
        &self,
        specifier: &str,
        css: &str,
        context: &mut WebUIProcessContext,
    ) -> Result<()> {
        let tag = crate::css_module::build_importmap_tag(specifier, css, context.nonce);
        context.writer.write(&tag)?;
        Ok(())
    }

    /// Install one compiler-ordered closure into the active CSS tree.
    fn emit_component_style_closure(
        &self,
        root: &str,
        install: StyleClosureInstall,
        context: &mut WebUIProcessContext,
    ) -> Result<()> {
        if context.protocol.style_closures.is_empty() {
            return Ok(());
        }
        let shadow_static_closure = if install == StyleClosureInstall::Routed {
            match context.shadow_style_roots.last() {
                Some(shadow_root) => {
                    if !shadow_root.static_closure_emitted {
                        return Err(HandlerError::Invariant(
                            "routed styles reached a ShadowRoot before its static style hook"
                                .to_string(),
                        ));
                    }
                    let root_name = context
                        .component_index
                        .iter()
                        .find_map(|(name, index)| {
                            (*index == shadow_root.component_index).then_some(name.as_str())
                        })
                        .ok_or_else(|| {
                            HandlerError::Invariant(
                                "active Shadow style root lost its protocol index".to_string(),
                            )
                        })?;
                    Some(
                        context
                            .protocol
                            .style_closures
                            .get(root_name)
                            .ok_or_else(|| {
                                HandlerError::Invariant(format!(
                            "component style closure metadata is missing Shadow root `{root_name}`"
                        ))
                            })?,
                    )
                }
                None => None,
            }
        } else {
            None
        };
        let closure = context.protocol.style_closures.get(root).ok_or_else(|| {
            HandlerError::Invariant(format!(
                "component style closure metadata is missing root `{root}`"
            ))
        })?;
        let is_document_tree = context.shadow_style_roots.is_empty();
        let strategy = context.css_strategy;

        // A bundled build delivers merged chunks; otherwise every component
        // delivers its own stylesheet. Both walk the closure in the same order,
        // so the emitted cascade is identical either way. Bundling is a
        // build-wide decision, so the two never mix within one protocol.
        let unit_count = WebUIProtocol::style_closure_unit_count(closure);

        for position in 0..unit_count {
            let unit = context
                .protocol
                .style_closure_unit(closure, &context.style_chunk_index, position)
                .ok_or_else(|| {
                    HandlerError::Invariant(format!(
                        "component style closure `{root}` references out-of-range unit {position}"
                    ))
                })?;
            let (name, chunk) = (unit.name, unit.chunk);
            let resource = unit.resource.ok_or_else(|| match chunk {
                Some(index) => HandlerError::Invariant(format!(
                    "component style closure `{root}` references missing style chunk {index}"
                )),
                None => HandlerError::Invariant(format!(
                    "component style closure `{root}` references missing resource `{name}`"
                )),
            })?;
            if is_document_tree && !context.document_style_resources.insert(name.to_string()) {
                continue;
            }
            if let Some(static_closure) = shadow_static_closure {
                let resource_index = match chunk {
                    Some(index) => {
                        if static_closure.style_chunks.contains(&index) {
                            continue;
                        }
                        index
                    }
                    None => {
                        if static_closure.component_tags.iter().any(|tag| tag == name) {
                            continue;
                        }
                        context.component_index.get(name).copied().ok_or_else(|| {
                            HandlerError::Invariant(format!(
                                "component style resource `{name}` is missing its protocol index"
                            ))
                        })?
                    }
                };
                let shadow_root = context.shadow_style_roots.last_mut().ok_or_else(|| {
                    HandlerError::Invariant(
                        "active Shadow style root disappeared during routed style delivery"
                            .to_string(),
                    )
                })?;
                if shadow_root.routed_resources.contains(&resource_index) {
                    continue;
                }
                shadow_root.routed_resources.push(resource_index);
            }

            match strategy {
                webui_protocol::CssStrategy::Link => {
                    context.writer.write("<link rel=\"stylesheet\" href=\"")?;
                    context
                        .writer
                        .write(&crate::html_encode::encode_safe(resource))?;
                    context.writer.write("\" data-webui-resource=\"")?;
                    context
                        .writer
                        .write(&crate::html_encode::encode_safe(name))?;
                    context.writer.write("\" data-webui-strategy=\"link\">")?;
                }
                strategy => {
                    context.writer.write("<style")?;
                    if let Some(nonce) = context.nonce {
                        context.writer.write(" nonce=\"")?;
                        context
                            .writer
                            .write(&crate::html_encode::encode_safe(nonce))?;
                        context.writer.write("\"")?;
                    }
                    context.writer.write(" data-webui-resource=\"")?;
                    context
                        .writer
                        .write(&crate::html_encode::encode_safe(name))?;
                    context.writer.write("\" data-webui-strategy=\"")?;
                    context
                        .writer
                        .write(if strategy == webui_protocol::CssStrategy::Module {
                            "module"
                        } else {
                            "style"
                        })?;
                    context.writer.write("\">")?;
                    crate::html_encode::write_style_text(context.writer, resource)?;
                    context.writer.write("</style>")?;
                }
            }
        }
        Ok(())
    }

    pub(crate) fn ensure_request_route_chain(context: &mut WebUIProcessContext) {
        if context.route_chain.is_none() {
            let plan = crate::route_handler::collect_route_chain_plan(
                context.protocol,
                context.entry_id,
                context.request_path,
                context.route_index,
            );
            context.route_document_style_targets = plan.document_style_targets;
            context.route_chain = Some(plan.entries);
        }
    }

    fn emit_active_route_styles(
        &self,
        preloaded: &mut Vec<u32>,
        context: &mut WebUIProcessContext,
    ) -> Result<()> {
        Self::ensure_request_route_chain(context);
        let Some(chain) = context.route_chain.take() else {
            return Err(route_style_plan_missing_error());
        };
        let document_targets = std::mem::take(&mut context.route_document_style_targets);
        if chain.len() != document_targets.len() {
            let error = route_style_plan_length_error(chain.len(), document_targets.len());
            context.route_document_style_targets = document_targets;
            context.route_chain = Some(chain);
            return Err(error);
        }

        let result = (|| {
            for (entry, targets_document) in chain.iter().zip(&document_targets) {
                if *targets_document {
                    self.emit_component_style_closure(
                        &entry.component,
                        StyleClosureInstall::Static,
                        context,
                    )?;
                    continue;
                }

                if context.css_strategy != webui_protocol::CssStrategy::Link {
                    continue;
                }
                self.emit_component_style_preloads(&entry.component, preloaded, context)?;
            }
            Ok(())
        })();

        context.route_document_style_targets = document_targets;
        context.route_chain = Some(chain);
        result
    }

    fn emit_component_style_preloads(
        &self,
        root: &str,
        preloaded: &mut Vec<u32>,
        context: &mut WebUIProcessContext,
    ) -> Result<()> {
        let Some(closure) = context.protocol.style_closures.get(root) else {
            return Ok(());
        };
        // A chunk shared by several tree-local roots needs one preload.
        let unit_count = WebUIProtocol::style_closure_unit_count(closure);
        for position in 0..unit_count {
            let Some(unit) =
                context
                    .protocol
                    .style_closure_unit(closure, &context.style_chunk_index, position)
            else {
                continue;
            };
            let Some(href) = unit.resource else {
                continue;
            };
            let name = unit.name;
            if context.document_style_resources.contains(name) {
                continue;
            }
            let Some(&resource_index) = context.style_resource_index.get(name) else {
                continue;
            };
            if preloaded.contains(&resource_index) {
                continue;
            }
            context
                .writer
                .write("<link rel=\"preload\" as=\"style\" href=\"")?;
            context
                .writer
                .write(&crate::html_encode::encode_safe(href))?;
            context.writer.write("\">")?;
            preloaded.push(resource_index);
        }
        Ok(())
    }

    fn emit_reachable_shadow_preloads(
        &self,
        preloaded: &mut Vec<u32>,
        context: &mut WebUIProcessContext,
    ) -> Result<()> {
        if context.css_strategy != webui_protocol::CssStrategy::Link {
            return Ok(());
        }
        let reachable = context.reachable_components.take().unwrap_or_else(|| {
            crate::route_handler::collect_reachable_component_order_for_request(
                context.protocol,
                context.entry_id,
                context.request_path,
                context.route_index,
            )
        });
        let result = (|| {
            for component in &reachable {
                if context.protocol.component_uses_shadow_dom(component) {
                    self.emit_component_style_preloads(component, preloaded, context)?;
                }
            }
            Ok(())
        })();
        context.reachable_components = Some(reachable);
        result
    }

    fn process_shadow_style_signal(
        &self,
        root: &str,
        context: &mut WebUIProcessContext,
    ) -> Result<()> {
        let Some(root_index) = context.component_index.get(root).copied() else {
            return Err(HandlerError::Invariant(format!(
                "Shadow style hook `{root}` references an unknown component"
            )));
        };
        if context
            .shadow_style_roots
            .last()
            .map(|shadow_root| shadow_root.component_index)
            != Some(root_index)
        {
            return Err(HandlerError::Invariant(format!(
                "Shadow style hook `{root}` does not match the active component root"
            )));
        }
        self.emit_component_style_closure(root, StyleClosureInstall::Static, context)?;
        let shadow_root = context.shadow_style_roots.last_mut().ok_or_else(|| {
            HandlerError::Invariant(
                "active Shadow style root disappeared after its static style hook".to_string(),
            )
        })?;
        shadow_root.static_closure_emitted = true;
        Ok(())
    }

    /// Emit a component's CSS module importmap on its first render
    /// (deduped by `rendered_components`) into the component's light DOM,
    /// so the browser registers it under the component's specifier
    /// before the shadow root template is parsed. See
    /// [`Self::emit_css_module_importmap`] for the emitted shape.
    ///
    /// Only components rendered on the current route get inline definitions;
    /// navigation responses carry later definitions in `componentStyles`.
    fn emit_css_module(
        &self,
        component: &webui_protocol::WebUIFragmentComponent,
        context: &mut WebUIProcessContext,
    ) -> Result<()> {
        if context.css_strategy != webui_protocol::CssStrategy::Module {
            return Ok(());
        }
        let metadata_already_streamed = context.streaming.as_ref().is_some_and(|streaming| {
            streaming_template_already_sent(
                streaming,
                context.component_index,
                &component.fragment_id,
            )
        });
        if !metadata_already_streamed
            && !context.rendered_components.contains(&component.fragment_id)
        {
            if let Some(css) = context
                .protocol
                .components
                .get(&component.fragment_id)
                .map(|c| c.css.as_str())
                .filter(|s| !s.is_empty())
            {
                self.emit_css_module_importmap(&component.fragment_id, css, context)?;
            }
        }
        Ok(())
    }

    /// Mark a handler-generated component host when it uses Light DOM.
    fn write_light_dom_marker(
        &self,
        component: &str,
        context: &mut WebUIProcessContext,
    ) -> Result<()> {
        if !context.protocol.component_uses_shadow_dom(component) {
            context.writer.write(" data-wl")?;
        }
        Ok(())
    }

    /// Process a route fragment — renders `<webui-route>` with matched/hidden state.
    fn process_route(
        &self,
        route_frag: &webui_protocol::WebUiFragmentRoute,
        best_route: &Option<(String, route_matcher::RouteMatch)>,
        context: &mut WebUIProcessContext,
    ) -> Result<()> {
        let is_matched = best_route
            .as_ref()
            .is_some_and(|(best_key, _)| *best_key == route_frag.fragment_id);

        context.writer.write("<webui-route")?;
        if !route_frag.path.is_empty() {
            context.writer.write(" path=\"")?;
            context.writer.write(&route_frag.path)?;
            context.writer.write("\"")?;
        }
        if !route_frag.fragment_id.is_empty() {
            context.writer.write(" component=\"")?;
            context.writer.write(&route_frag.fragment_id)?;
            context.writer.write("\"")?;
        }
        if route_frag.exact {
            context.writer.write(" exact")?;
        }
        route_renderer::write_route_navigation_attrs(context.writer, route_frag)?;

        if is_matched {
            // Emit data-ri for O(1) client-side element binding
            let ri = context.route_chain_index;
            context.route_chain_index += 1;
            context.writer.write(" data-ri=\"")?;
            write_usize(context.writer, ri)?;
            context.writer.write("\" active>")?;

            if !route_frag.fragment_id.is_empty() {
                let saved_route_base = context.route_base.clone();
                let saved_route_children = std::mem::take(&mut context.route_children);
                if let Some((_, ref rm)) = best_route {
                    context.route_base = Cow::Owned(route_matcher::compute_route_base(
                        context.request_path,
                        rm.consumed_segments,
                    ));
                }

                context.route_children = route_frag.children.clone();

                if !Self::component_owns_css_tree(&route_frag.fragment_id, context.protocol) {
                    self.emit_component_style_closure(
                        &route_frag.fragment_id,
                        StyleClosureInstall::Routed,
                        context,
                    )?;
                }
                let comp = webui_protocol::WebUIFragmentComponent {
                    fragment_id: route_frag.fragment_id.clone(),
                };

                context.writer.write("<")?;
                context.writer.write(&route_frag.fragment_id)?;
                self.write_light_dom_marker(&route_frag.fragment_id, context)?;
                if let Some(p) = &context.plugin {
                    p.write_route_component_state(context.state, context.writer)?;
                }
                prepare_generated_streaming_root(&route_frag.fragment_id, context)?;
                context.writer.write(">")?;

                self.process_component(&comp, ComponentHostOrigin::HandlerGenerated, context)?;

                context.writer.write("</")?;
                context.writer.write(&route_frag.fragment_id)?;
                context.writer.write(">")?;

                context.route_base = saved_route_base;
                context.route_children = saved_route_children;
            }
        } else {
            context.writer.write(" style=\"display:none\">")?;
        }

        context.writer.write("</webui-route>")?;
        Ok(())
    }

    /// Process a component fragment.
    fn process_component(
        &self,
        component: &webui_protocol::WebUIFragmentComponent,
        origin: ComponentHostOrigin,
        context: &mut WebUIProcessContext,
    ) -> Result<()> {
        if context.streaming.is_some() {
            consume_streaming_component_root(&component.fragment_id, origin, context)?;
            // Capture only after root parity succeeds, so malformed protocols
            // cannot contribute unmarked hosts to a checkpoint.
            record_checkpoint_tag(context, &component.fragment_id);
        }

        // Emit the component's CSS module importmap into its light DOM and track
        // the component as rendered on first encounter only. `rendered_components`
        // is a set, so gating the `insert` (and its `String` clone) behind the
        // first-encounter check avoids allocating a throwaway `String` for every
        // duplicate instance while keeping the set contents identical.
        if !context.rendered_components.contains(&component.fragment_id) {
            self.emit_css_module(component, context)?;
            context
                .rendered_components
                .insert(component.fragment_id.clone());
        }

        let owns_css_tree = Self::component_owns_css_tree(&component.fragment_id, context.protocol);
        if owns_css_tree {
            Self::push_shadow_style_root(&component.fragment_id, context)?;
        }

        // Save parent scope. `mem::take` leaves an alloc-free empty map behind.
        let saved_local_vars = std::mem::take(&mut context.local_vars);
        // The component's accumulated attrs become its local vars; the next
        // sibling accumulates into a recycled (capacity-preserving) map from the
        // request-local pool instead of a freshly allocated `HashMap`.
        let saved_component_attrs = std::mem::replace(
            &mut context.component_attrs,
            take_scope_map(&mut context.scope_pool),
        );
        context.local_vars = saved_component_attrs;

        if let Some(p) = &mut context.plugin {
            p.push_scope();
        }

        let render_result = self.process_fragment_id(&component.fragment_id, context);

        if owns_css_tree {
            Self::pop_shadow_style_root(&component.fragment_id, context)?;
        }
        render_result?;

        if let Some(p) = &mut context.plugin {
            p.pop_scope();
        }

        // Restore parent scope, recycling this component's local map (its
        // accumulated attrs) back into the pool so a sibling reuses its capacity.
        let used_locals = std::mem::replace(&mut context.local_vars, saved_local_vars);
        recycle_scope_map(&mut context.scope_pool, used_locals);
        // The attr accumulator (pulled from the pool above) is cleared for the
        // next sibling while retaining its bucket capacity.
        context.component_attrs.clear();

        Ok(())
    }

    #[inline]
    fn component_owns_css_tree(component: &str, protocol: &WebUIProtocol) -> bool {
        !protocol.style_closures.is_empty()
            && protocol
                .components
                .get(component)
                .is_some_and(|data| data.uses_shadow_dom)
    }

    fn push_shadow_style_root(component: &str, context: &mut WebUIProcessContext) -> Result<()> {
        if !context.protocol.style_closures.contains_key(component) {
            return Err(HandlerError::Invariant(format!(
                "component style closure metadata is missing Shadow root `{component}`"
            )));
        }
        let root_index = context
            .component_index
            .get(component)
            .copied()
            .ok_or_else(|| {
                HandlerError::Invariant(format!(
                    "Shadow component `{component}` is missing its protocol index"
                ))
            })?;
        context.shadow_style_roots.push(ShadowStyleRoot {
            component_index: root_index,
            static_closure_emitted: false,
            routed_resources: Vec::new(),
        });
        Ok(())
    }

    fn pop_shadow_style_root(component: &str, context: &mut WebUIProcessContext) -> Result<()> {
        context.shadow_style_roots.pop().ok_or_else(|| {
            HandlerError::Invariant(format!(
                "Shadow component `{component}` lost its active style root"
            ))
        })?;
        Ok(())
    }

    /// Resolve a dotted path value, checking local variables first, then global state.
    fn resolve_value(
        &self,
        path: &str,
        context: &WebUIProcessContext<'_, '_, '_>,
    ) -> Option<Value> {
        resolve_value_from_sources(path, &context.local_vars, context.state).map(Cow::into_owned)
    }

    /// Evaluate a condition expression against the current context.
    ///
    /// Uses a resolver closure that checks local variables first, then falls
    /// back to global state — avoiding a full clone of the state tree.
    /// Returns false if the condition references a missing value.
    fn evaluate_condition(
        &self,
        condition: &webui_protocol::ConditionExpr,
        context: &WebUIProcessContext,
    ) -> Result<bool> {
        let local_vars = &context.local_vars;
        let state = context.state;
        match evaluate_with_resolver(condition, |path| {
            resolve_value_from_sources(path, local_vars, state)
        }) {
            Ok(result) => Ok(result),
            Err(ExpressionError::MissingValue(_)) => Ok(false),
            Err(e) => Err(HandlerError::Evaluation(e.to_string())),
        }
    }

    /// Process a for loop fragment.
    ///
    /// Creates a new context for each iteration that includes the current loop item.
    /// This allows nested templates to access both the loop variable and any parent context.
    /// Example: `for item in items` makes "item" available in the loop body.
    fn process_for_loop(
        &self,
        for_loop: &webui_protocol::WebUIFragmentFor,
        context: &mut WebUIProcessContext,
    ) -> Result<()> {
        let collection_name = &for_loop.collection;

        // If the collection is missing, treat it as empty (0 iterations) — matches NodeJS behavior.
        // Hydration comments are always emitted regardless of collection presence.
        let items = match self.resolve_value(collection_name, context) {
            Some(Value::Array(arr)) => arr,
            Some(_) => {
                return Err(HandlerError::TypeError(format!(
                    "Collection '{}' is not an array",
                    collection_name
                )))
            }
            None => Vec::new(),
        };

        if let Some(p) = &mut context.plugin {
            p.on_for_start(&for_loop.fragment_id, context.writer)?;
        }

        // Hot-loop optimisation: the loop variable name is `String`-keyed
        // in `local_vars`. The naive impl re-inserts (and so re-allocates
        // the key) on every iteration — a 1000-item loop pays 2000 String
        // clones for the key alone. Instead, we save the outer-scope
        // value (if any) ONCE before the loop, install the key ONCE with
        // an empty placeholder, then overwrite the value in-place each
        // iteration via `get_mut`. Restoration at the end happens once.
        let item_name = for_loop.item.as_str();
        let saved_value = context.local_vars.remove(item_name);
        // Pre-insert the key so per-iteration `get_mut` is infallible.
        // Cost: at most one `String::from(item_name)` for the lifetime
        // of the loop, regardless of iteration count.
        if !items.is_empty() {
            context
                .local_vars
                .insert(item_name.to_string(), Value::Null);
        }
        for (i, item) in items.into_iter().enumerate() {
            if let Some(p) = &mut context.plugin {
                p.on_repeat_item_start(i, context.writer)?;
                p.push_scope();
            }

            // O(1) value swap; no key allocation.
            if let Some(slot) = context.local_vars.get_mut(item_name) {
                *slot = item;
            }
            self.process_fragment_id(&for_loop.fragment_id, context)?;

            if let Some(p) = &mut context.plugin {
                p.pop_scope();
                p.on_repeat_item_end(i, context.writer)?;
            }
        }
        // Restore outer scope (or remove the placeholder we installed).
        match saved_value {
            Some(value) => {
                context.local_vars.insert(item_name.to_string(), value);
            }
            None => {
                context.local_vars.remove(item_name);
            }
        }

        if let Some(p) = &mut context.plugin {
            p.on_for_end(&for_loop.fragment_id, context.writer)?;
        }

        Ok(())
    }

    /// Process a signal fragment.
    ///
    /// Looks up the value in the context first (for local variables), then in the global state.
    /// This prioritization allows local variables (like loop items) to override global state.
    /// If the value is not found in either scope, an empty string is returned.
    #[inline]
    fn process_signal<'data>(
        &self,
        signal: &'data webui_protocol::WebUIFragmentSignal,
        context: &mut WebUIProcessContext<'data, '_, '_>,
    ) -> Result<()> {
        if signal.raw {
            self.process_raw_signal(signal, context)
        } else {
            self.process_state_signal(signal, context)
        }
    }

    #[inline(never)]
    fn process_raw_signal<'data>(
        &self,
        signal: &'data webui_protocol::WebUIFragmentSignal,
        context: &mut WebUIProcessContext<'data, '_, '_>,
    ) -> Result<()> {
        let Some(structural_value) = structural_signal_value(signal) else {
            return self.process_state_signal(signal, context);
        };

        if let Some(root) = structural_value.strip_prefix("shadow_styles:") {
            return self.process_shadow_style_signal(root, context);
        }

        if context.streaming.is_some()
            && self.process_streaming_signal(structural_value, context)?
        {
            return Ok(());
        }

        // Hook: emit nonce meta and Document-owned CSS before </head>.
        // Guarded by `head_end_emitted` so a malformed protocol cannot
        // emit nonce/preloads/inject more than once per render.
        if structural_value == "head_end" && !context.head_end_emitted {
            context.head_end_emitted = true;
            if let Some(nonce) = context.nonce {
                context
                    .writer
                    .write("<meta name=\"webui-nonce\" content=\"")?;
                context
                    .writer
                    .write(&crate::html_encode::encode_safe(nonce))?;
                context.writer.write("\">")?;
            }

            // Render-policy CSS is emitted before component styles so an
            // authored declaration still wins on a tie.
            if !context.protocol.component_render_css.is_empty() {
                context.writer.write("<style data-webui-render-policy")?;
                if let Some(nonce) = context.nonce {
                    context.writer.write(" nonce=\"")?;
                    context
                        .writer
                        .write(&crate::html_encode::encode_safe(nonce))?;
                    context.writer.write("\"")?;
                }
                context.writer.write(">")?;
                context
                    .writer
                    .write(&context.protocol.component_render_css)?;
                context.writer.write("</style>")?;
            }

            if !context.protocol.style_closures.is_empty() {
                let entry_id = context.entry_id;
                self.emit_component_style_closure(entry_id, StyleClosureInstall::Static, context)?;
                let mut preloaded = Vec::new();
                self.emit_active_route_styles(&mut preloaded, context)?;
                self.emit_reachable_shadow_preloads(&mut preloaded, context)?;
            }

            // Compiler-resolved `modulepreload` hints for the shared chunks
            // the page's module entries statically import. Those chunks are
            // named only inside the entry's own bytes, so without this the
            // browser must download and parse the entry before it can even
            // discover them.
            //
            // Emitted after the CSS links on purpose: CSS is render-blocking
            // and owns first paint, while these own first interaction, so the
            // stylesheet requests go out first. The list arrives pre-ordered
            // (largest chunk first) and pre-resolved from the build, so this
            // is a straight write with no per-request work.
            if !context.protocol.module_preloads.is_empty() {
                for href in &context.protocol.module_preloads {
                    context
                        .writer
                        .write("<link rel=\"modulepreload\" href=\"")?;
                    context.writer.write(href)?;
                    context.writer.write("\">")?;
                }
            }

            // Per-render `head_inject` HTML — image preloads, A/B test
            // markers, etc. supplied by the host via RenderOptions.
            // Emitted at the structural head_end boundary, after the
            // built-in nonce + CSS-link emissions, so host injects
            // appear immediately before `</head>`.
            if let Some(html) = context.head_inject {
                context.writer.write(html)?;
            }

            // Reserved-state `headEnd` HTML, last at this boundary so a
            // host that sets both channels gets a deterministic order:
            // built-in emissions, then `RenderOptions`, then state.
            if let Some(html) = context.state_inject.head_end {
                context.writer.write(html)?;
            }
        }

        // Hook: emit state-supplied HTML immediately after `<body>`. Guarded
        // by its own dedup flag so a malformed protocol cannot duplicate it.
        if structural_value == "body_start" && !context.body_start_emitted {
            context.body_start_emitted = true;
            if let Some(html) = context.state_inject.body_start {
                context.writer.write(html)?;
            }
        }

        // Hook: emit component templates and host body_inject before </body>.
        // Single guarded block so the dedup flag protects both the
        // hydration emission and the host inject from a malformed
        // protocol that fires `body_end` more than once per render.
        if structural_value == "body_end" && !context.body_end_emitted {
            context.body_end_emitted = true;
            if context.plugin.is_some() {
                // Emit templates for all REACHABLE components on the current route,
                // not just those rendered in this SSR pass. Components inside false
                // <if> blocks or empty <for> loops are reachable via client-side
                // state changes and need their templates available without a server
                // round-trip. The graph walker follows conditional and loop branches
                // unconditionally, but only descends into the matched route chain —
                // components on other routes are delivered via SPA partial navigation.
                let reachable = context
                    .reachable_components
                    .take()
                    .unwrap_or_else(|| {
                        crate::route_handler::collect_reachable_component_order_for_request(
                            context.protocol,
                            context.entry_id,
                            context.request_path,
                            context.route_index,
                        )
                    })
                    .into_iter()
                    .collect::<HashSet<_>>();
                let state_selection =
                    collect_hydration_state(context.protocol, reachable.iter().map(String::as_str));

                // Emit CSS module importmaps for reachable-but-unrendered
                // components so the framework can adopt them when an `<if>`
                // condition flips true client-side.
                if context.css_strategy == webui_protocol::CssStrategy::Module {
                    for name in &reachable {
                        if !context.rendered_components.contains(name) {
                            if let Some(css) = context
                                .protocol
                                .components
                                .get(name)
                                .map(|c| c.css.as_str())
                                .filter(|s| !s.is_empty())
                            {
                                self.emit_css_module_importmap(name, css, context)?;
                            }
                        }
                    }
                }

                // Try to collect split WebUI template payloads. If the plugin
                // returns None (non-WebUI templates, e.g. FAST), fall back to
                // separate emission.
                let template_payloads = context
                    .plugin
                    .as_ref()
                    .and_then(|p| p.collect_template_payloads(context.protocol, &reachable));

                if template_payloads.is_none() {
                    // Non-JS templates (FAST plugins) - emit separately
                    if let Some(ref p) = context.plugin {
                        p.emit_templates(
                            context.protocol,
                            &reachable,
                            context.nonce,
                            context.writer,
                        )?;
                    }
                }

                // Compute the inventory hex from actually rendered components.
                let inventory_hex = crate::route_handler::encode_component_inventory(
                    &context.rendered_components,
                    context.component_index,
                );

                // Chain
                Self::ensure_request_route_chain(context);
                let chain = context.route_chain.as_deref().ok_or_else(|| {
                    HandlerError::Invariant(
                        "request route chain disappeared after collection".to_string(),
                    )
                })?;
                let chain_json: Vec<Value> = chain
                    .iter()
                    .map(crate::route_handler::RouteChainEntry::to_json)
                    .collect();

                // CSS hrefs emitted during SSR (Link-strategy components)
                let is_link = context.css_strategy == webui_protocol::CssStrategy::Link;
                let mut css_hrefs: Vec<&str> = Vec::new();
                if is_link {
                    for name in &reachable {
                        if let Some(href) = context
                            .protocol
                            .components
                            .get(name)
                            .map(|c| c.css_href.as_str())
                            .filter(|h| !h.is_empty())
                        {
                            css_hrefs.push(href);
                        }
                    }
                }

                // Module style specifiers emitted during SSR
                let mut style_specs: Vec<&str> = Vec::new();
                if context.css_strategy == webui_protocol::CssStrategy::Module {
                    for name in &reachable {
                        if context
                            .protocol
                            .components
                            .get(name)
                            .map(|c| !c.css.is_empty())
                            .unwrap_or(false)
                        {
                            style_specs.push(name);
                        }
                    }
                }

                let empty_payloads: [WebUiTemplatePayload<'_>; 0] = [];
                let payloads = template_payloads.as_deref().unwrap_or(&empty_payloads);
                let mut style_roots = Vec::with_capacity(reachable.len() + chain.len() * 3 + 1);
                style_roots.push(context.entry_id);
                for entry in chain {
                    style_roots.push(entry.component.as_str());
                    if !entry.pending_component.is_empty() {
                        style_roots.push(entry.pending_component.as_str());
                    }
                    if !entry.error_component.is_empty() {
                        style_roots.push(entry.error_component.as_str());
                    }
                }
                style_roots.extend(reachable.iter().map(String::as_str));
                let component_styles =
                    crate::route_handler::collect_component_styles(context.protocol, style_roots)?;
                write_webui_data_block(
                    context.writer,
                    &mut context.json_scratch,
                    WebUiBootstrap {
                        state: context.state,
                        state_selection,
                        chain: &chain_json,
                        inventory: &inventory_hex,
                        nonce: context.nonce,
                        css_hrefs: &css_hrefs,
                        style_specs: &style_specs,
                        component_styles: &component_styles,
                        templates: payloads,
                    },
                )?;

                // Let the active plugin emit any framework-specific executable
                // side channel. FAST plugins default to no-op; WebUI installs
                // templateFns. Client packages parse #webui-data lazily.
                if let Some(ref plugin) = context.plugin {
                    plugin.emit_bootstrap_extension(
                        BootstrapExtensionContext {
                            protocol: context.protocol,
                            components: &reachable,
                            payloads,
                            nonce: context.nonce,
                        },
                        context.writer,
                    )?;
                }
            }

            // Per-render `body_inject` HTML — dev livereload script,
            // analytics, etc. supplied by the host via RenderOptions.
            // Inside the dedup block but outside the plugin-only
            // sub-block above, so it fires regardless of whether a
            // hydration plugin is active. Appears immediately before
            // `</body>`.
            if let Some(html) = context.body_inject {
                context.writer.write(html)?;
            }

            // Reserved-state `bodyEnd` HTML, last at this boundary. Same
            // precedence as `head_end`: built-ins, `RenderOptions`, state.
            if let Some(html) = context.state_inject.body_end {
                context.writer.write(html)?;
            }
        }

        // Structural signals are never state lookups. In particular, ordinary
        // rendering ignores boundary/root markers byte-for-byte, while authored
        // raw bindings with the same visible key remain ordinary state.
        Ok(())
    }

    #[inline]
    fn process_state_signal(
        &self,
        signal: &webui_protocol::WebUIFragmentSignal,
        context: &mut WebUIProcessContext,
    ) -> Result<()> {
        if let Some(p) = &mut context.plugin {
            p.on_binding_start(&signal.value, context.writer)?;
        }

        if let Some(value) = self.resolve_value(&signal.value, context) {
            self.write_signal_value(&value, signal.raw, context.writer)?;
        }

        if let Some(p) = &mut context.plugin {
            p.on_binding_end(&signal.value, context.writer)?;
        }
        Ok(())
    }

    /// Write a signal value directly to the writer, avoiding intermediate String allocation.
    /// For HTML-escaped output, writes the Cow from `encode_safe` directly.
    fn write_signal_value(
        &self,
        value: &Value,
        raw: bool,
        writer: &mut dyn ResponseWriter,
    ) -> Result<()> {
        if raw {
            match value {
                Value::String(s) => writer.write(s),
                _ => writer.write(&value.to_string()),
            }
        } else {
            match value {
                Value::String(s) => writer.write(&crate::html_encode::encode_safe(s)),
                _ => {
                    let s = value.to_string();
                    writer.write(&crate::html_encode::encode_safe(&s))
                }
            }
        }
    }

    /// Process an if condition fragment.
    fn process_if(
        &self,
        if_cond: &webui_protocol::WebUIFragmentIf,
        context: &mut WebUIProcessContext,
    ) -> Result<()> {
        let condition = if_cond
            .condition
            .as_ref()
            .ok_or_else(|| HandlerError::Rendering("If fragment missing condition".to_string()))?;
        let condition_met = self.evaluate_condition(condition, context)?;

        if let Some(p) = &mut context.plugin {
            p.on_if_start(&if_cond.fragment_id, context.writer)?;
        }

        if condition_met {
            if let Some(p) = &mut context.plugin {
                p.push_scope();
            }

            self.process_fragment_id(&if_cond.fragment_id, context)?;

            if let Some(p) = &mut context.plugin {
                p.pop_scope();
            }
        }

        if let Some(p) = &mut context.plugin {
            p.on_if_end(&if_cond.fragment_id, context.writer)?;
        }

        Ok(())
    }

    /// Process an attribute fragment by rendering the attribute name/value pair.
    fn process_attribute(
        &self,
        attr: &webui_protocol::WebUIFragmentAttribute,
        context: &mut WebUIProcessContext,
    ) -> Result<()> {
        // Initialize component attribute accumulator on attrStart
        if attr.attr_start {
            context.component_attrs = HashMap::new();
        }

        // Boolean attribute with condition tree
        if let Some(condition) = &attr.condition_tree {
            let condition_met = self.evaluate_condition(condition, context)?;

            if !attr.attr_skip {
                let name = component_attr_name(&attr.name);
                context
                    .component_attrs
                    .insert(name, Value::Bool(condition_met));
            }

            if condition_met {
                context.writer.write(" ")?;
                context.writer.write(&attr.name)?;
            }
            return Ok(());
        }

        // Template attribute (mixed static + dynamic)
        if !attr.template.is_empty() {
            let raw_value = self.render_template_attr_value(&attr.template, context)?;
            let escaped = crate::html_encode::encode_safe(&raw_value);
            write_attr(context.writer, &attr.name, &escaped)?;

            if !attr.attr_skip {
                let name = component_attr_name(&attr.name);
                context
                    .component_attrs
                    .insert(name, Value::String(raw_value));
            }
            return Ok(());
        }

        // Simple attribute
        if !attr.value.is_empty() {
            if attr.raw_value {
                // Static attribute — value is the literal string
                write_attr(context.writer, &attr.name, &attr.value)?;
                if !attr.attr_skip {
                    let name = component_attr_name(&attr.name);
                    context
                        .component_attrs
                        .insert(name, Value::String(attr.value.clone()));
                }
            } else if attr.complex {
                // Complex attribute — resolve value, don't render to HTML, store as state
                if let Some(value) = self.resolve_value(&attr.value, context) {
                    if !attr.attr_skip {
                        let stripped = attr.name.strip_prefix(':').unwrap_or(&attr.name);
                        let name = component_attr_name(stripped);
                        context.component_attrs.insert(name, value);
                    }
                }
            } else {
                // Dynamic attribute — resolve and render
                let value = self.resolve_value(&attr.value, context);
                // Always emit the attribute so FAST hydration markers
                // (`data-fe`) match the DOM node structure.
                match &value {
                    Some(Value::String(s)) => {
                        write_attr(
                            context.writer,
                            &attr.name,
                            &crate::html_encode::encode_safe(s),
                        )?;
                    }
                    Some(Value::Null) | None => {
                        write_attr(context.writer, &attr.name, "")?;
                    }
                    Some(other) => {
                        let s = other.to_string();
                        write_attr(
                            context.writer,
                            &attr.name,
                            &crate::html_encode::encode_safe(&s),
                        )?;
                    }
                }

                if !attr.attr_skip {
                    let name = component_attr_name(&attr.name);
                    context
                        .component_attrs
                        .insert(name, value.unwrap_or(Value::String(String::new())));
                }
            }
        }

        Ok(())
    }

    /// Render a template attribute's fragments into a raw (unescaped) string.
    fn render_template_attr_value(
        &self,
        template_id: &str,
        context: &WebUIProcessContext,
    ) -> Result<String> {
        let fragments = context
            .protocol
            .fragments
            .get(template_id)
            .ok_or_else(|| HandlerError::MissingFragment(template_id.to_string()))?;
        let mut raw_value = String::new();
        for frag in &fragments.fragments {
            match frag.fragment.as_ref() {
                Some(Fragment::Raw(raw)) => raw_value.push_str(&raw.value),
                Some(Fragment::Signal(signal)) => {
                    if let Some(value) = self.resolve_value(&signal.value, context) {
                        match &value {
                            Value::String(s) => raw_value.push_str(s),
                            _ => raw_value.push_str(&value.to_string()),
                        }
                    }
                }
                _ => {}
            }
        }
        Ok(raw_value)
    }

    /// Render the UI based on the protocol and state.
    pub fn render<'a>(
        &self,
        protocol: &'a Protocol,
        state: &'a Value,
        options: &RenderOptions<'a>,
        writer: &'a mut dyn ResponseWriter,
    ) -> Result<()> {
        protocol.ensure_style_metadata()?;
        let document = protocol.protocol();
        let entry = document
            .fragments
            .get(options.entry_id)
            .ok_or_else(|| HandlerError::MissingFragment(options.entry_id.to_string()))?;
        let entry_owns_css_tree = Self::component_owns_css_tree(options.entry_id, document);
        let has_document_head_boundary = !entry_owns_css_tree
            && !document.style_closures.is_empty()
            && entry.fragments.iter().any(|fragment| {
                matches!(
                    fragment.fragment.as_ref(),
                    Some(Fragment::Signal(signal))
                        if structural_signal_value(signal) == Some("head_end")
                )
            });
        let doctype_split = (!entry_owns_css_tree
            && !has_document_head_boundary
            && !document.style_closures.is_empty())
        .then(|| {
            let Some(Fragment::Raw(raw)) = entry
                .fragments
                .first()
                .and_then(|fragment| fragment.fragment.as_ref())
            else {
                return None;
            };
            doctype_prefix_end(&raw.value).map(|end| (raw.value.as_str(), end))
        })
        .flatten();
        let mut context = WebUIProcessContext {
            protocol: document,
            state,
            writer,
            local_vars: HashMap::new(),
            component_attrs: HashMap::new(),
            request_path: options.request_path,
            route_base: Cow::Borrowed("/"),
            rendered_components: HashSet::new(),
            plugin: self.plugin_factory.map(|f| f()),
            route_children: Vec::new(),
            entry_id: options.entry_id,
            // Same defensive normalisation as `handle()`. See the
            // doc-comment there for the CSP-outage rationale.
            nonce: options.nonce.filter(|s| !s.is_empty()),
            head_inject: options.head_inject.filter(|s| !s.is_empty()),
            body_inject: options.body_inject.filter(|s| !s.is_empty()),
            state_inject: StateInject::resolve(state),
            head_end_emitted: false,
            body_start_emitted: false,
            component_index: protocol.component_index(),
            style_resource_index: protocol.style_resource_index(),
            style_chunk_index: protocol.protocol().style_chunk_index(),
            css_strategy: protocol.css_strategy(),
            body_end_emitted: false,
            route_index: protocol.route_index(),
            route_chain_index: 0,
            route_chain: None,
            route_document_style_targets: Vec::new(),
            reachable_components: None,
            streaming: None,
            json_scratch: Vec::new(),
            scope_pool: Vec::new(),
            document_style_resources: HashSet::new(),
            shadow_style_roots: Vec::new(),
        };

        if entry_owns_css_tree {
            Self::push_shadow_style_root(options.entry_id, &mut context)?;
        }

        let render_result = if let Some((first_raw, split)) = doctype_split {
            context.writer.write(&first_raw[..split])?;
            self.emit_component_style_closure(
                options.entry_id,
                StyleClosureInstall::Static,
                &mut context,
            )?;
            self.emit_active_route_styles(&mut Vec::new(), &mut context)?;
            context.writer.write(&first_raw[split..])?;
            self.process_fragment_from(&entry.fragments, 1, &mut context)
        } else {
            if !entry_owns_css_tree
                && !has_document_head_boundary
                && !document.style_closures.is_empty()
            {
                self.emit_component_style_closure(
                    options.entry_id,
                    StyleClosureInstall::Static,
                    &mut context,
                )?;
                self.emit_active_route_styles(&mut Vec::new(), &mut context)?;
            }
            self.process_fragment_id(options.entry_id, &mut context)
        };

        if entry_owns_css_tree {
            Self::pop_shadow_style_root(options.entry_id, &mut context)?;
        }
        render_result?;
        writer.end()?;

        Ok(())
    }
}

impl Default for WebUIHandler {
    fn default() -> Self {
        Self::new()
    }
}

/// Write ` name="value"` to the writer without allocating a format string.
fn write_attr(writer: &mut dyn ResponseWriter, name: &str, value: &str) -> Result<()> {
    writer.write(" ")?;
    writer.write(name)?;
    writer.write("=\"")?;
    writer.write(value)?;
    writer.write("\"")
}

#[cfg(test)]
fn handle(
    protocol: &WebUIProtocol,
    state: &Value,
    options: &RenderOptions<'_>,
    writer: &mut dyn ResponseWriter,
) -> Result<()> {
    let handler = WebUIHandler::new();
    handler.handle(protocol, state, options, writer)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::streaming::STREAMING_MARKER;
    use std::cell::RefCell;
    use std::sync::Arc;
    use webui_parser::{ComponentRegistration, DomStrategy, HtmlParser};
    use webui_protocol::{
        web_ui_fragment, ComparisonOperator, ConditionExpr, FragmentList, LogicalOperator,
        WebUIFragmentAttribute, WebUiFragmentRoute,
    };
    use webui_test_utils::test_json;

    fn structural_fragment(value: impl AsRef<str>) -> WebUIFragment {
        WebUIFragment::signal(
            format!("{STRUCTURAL_SIGNAL_PREFIX}{}", value.as_ref()),
            true,
        )
    }

    // A simple test writer implementation
    struct TestWriter {
        content: RefCell<String>,
        ended: RefCell<bool>,
    }

    impl TestWriter {
        fn new() -> Self {
            Self {
                content: RefCell::new(String::new()),
                ended: RefCell::new(false),
            }
        }

        fn get_content(&self) -> String {
            self.content.borrow().clone()
        }

        fn is_ended(&self) -> bool {
            *self.ended.borrow()
        }
    }

    impl ResponseWriter for TestWriter {
        fn write(&mut self, content: &str) -> Result<()> {
            self.content.borrow_mut().push_str(content);
            Ok(())
        }

        fn end(&mut self) -> Result<()> {
            *self.ended.borrow_mut() = true;
            Ok(())
        }
    }

    #[test]
    fn test_handle_raw() {
        // Create a simple protocol
        let mut fragments = HashMap::new();
        fragments.insert(
            "index.html".to_string(),
            FragmentList {
                fragments: vec![WebUIFragment::raw("Hello, WebUI!")],
            },
        );

        let protocol = WebUIProtocol::new(fragments);
        let state = test_json!({});

        // Create a test writer
        let mut writer = TestWriter::new();

        // Handle the protocol
        assert!(
            handle(
                &protocol,
                &state,
                &RenderOptions::new("index.html", "/"),
                &mut writer
            )
            .is_ok(),
            "Failed to handle raw protocol"
        );

        // Check the output
        assert_eq!(writer.get_content(), "Hello, WebUI!");
        assert!(writer.is_ended());
    }

    #[test]
    fn test_handle_signal() {
        // Create a protocol with a signal
        let mut fragments = HashMap::new();
        fragments.insert(
            "index.html".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::raw("Hello, "),
                    WebUIFragment::signal("name", false),
                    WebUIFragment::raw("!"),
                ],
            },
        );

        let protocol = WebUIProtocol::new(fragments);
        let state = test_json!({"name": "WebUI"});

        // Create a test writer
        let mut writer = TestWriter::new();

        // Handle the protocol
        assert!(
            handle(
                &protocol,
                &state,
                &RenderOptions::new("index.html", "/"),
                &mut writer
            )
            .is_ok(),
            "Failed to handle signal protocol"
        );

        // Check the output
        assert_eq!(writer.get_content(), "Hello, WebUI!");
        assert!(writer.is_ended());
    }

    #[test]
    fn test_handle_for_loop() {
        // Create a protocol with a for loop
        let mut fragments = HashMap::new();
        fragments.insert(
            "index.html".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::raw("People: "),
                    WebUIFragment::for_loop("person", "people", "person-item"),
                ],
            },
        );

        fragments.insert(
            "person-item".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::signal("person.name", false),
                    WebUIFragment::raw(", "),
                ],
            },
        );

        let protocol = WebUIProtocol::new(fragments);
        let state = test_json!({
            "people": [
                {"name": "Alice"},
                {"name": "Bob"},
                {"name": "Charlie"}
            ]
        });

        // Create a test writer
        let mut writer = TestWriter::new();

        // Handle the protocol
        assert!(
            handle(
                &protocol,
                &state,
                &RenderOptions::new("index.html", "/"),
                &mut writer
            )
            .is_ok(),
            "Failed to handle for loop protocol"
        );

        // Check the output
        assert_eq!(writer.get_content(), "People: Alice, Bob, Charlie, ");
        assert!(writer.is_ended());
    }

    #[test]
    fn test_handle_if_condition() {
        // Create a protocol with an if condition
        let mut fragments = HashMap::new();
        fragments.insert(
            "index.html".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::raw("Status: "),
                    WebUIFragment::if_cond(
                        webui_protocol::ConditionExpr::identifier("isActive"),
                        "active-content",
                    ),
                    WebUIFragment::raw("End"),
                ],
            },
        );

        fragments.insert(
            "active-content".to_string(),
            FragmentList {
                fragments: vec![WebUIFragment::raw("Active")],
            },
        );

        let protocol = WebUIProtocol::new(fragments);

        // Test with isActive = true
        let state_true = test_json!({"isActive": true});
        let mut writer_true = TestWriter::new();
        assert!(
            handle(
                &protocol,
                &state_true,
                &RenderOptions::new("index.html", "/"),
                &mut writer_true
            )
            .is_ok(),
            "Failed to handle if condition (true case)"
        );
        assert_eq!(writer_true.get_content(), "Status: ActiveEnd");
        assert!(writer_true.is_ended());

        // Test with isActive = false
        let state_false = test_json!({"isActive": false});
        let mut writer_false = TestWriter::new();
        assert!(
            handle(
                &protocol,
                &state_false,
                &RenderOptions::new("index.html", "/"),
                &mut writer_false
            )
            .is_ok(),
            "Failed to handle if condition (false case)"
        );
        assert_eq!(writer_false.get_content(), "Status: End");
        assert!(writer_false.is_ended());
    }

    #[test]
    fn test_handle_component() {
        // Create a protocol with a component
        let mut fragments = HashMap::new();
        fragments.insert(
            "index.html".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::raw("Component: "),
                    WebUIFragment::component("my-component"),
                ],
            },
        );

        fragments.insert(
            "my-component".to_string(),
            FragmentList {
                fragments: vec![WebUIFragment::raw("<div>Component Content</div>")],
            },
        );

        let protocol = WebUIProtocol::new(fragments);
        let state = test_json!({});

        // Create a test writer
        let mut writer = TestWriter::new();

        // Handle the protocol
        assert!(
            handle(
                &protocol,
                &state,
                &RenderOptions::new("index.html", "/"),
                &mut writer
            )
            .is_ok(),
            "Failed to handle component protocol"
        );

        // Check the output
        assert_eq!(
            writer.get_content(),
            "Component: <div>Component Content</div>"
        );
        assert!(writer.is_ended());
    }

    #[test]
    fn test_missing_fragment() {
        // Create a protocol with a missing fragment reference
        let mut fragments = HashMap::new();
        fragments.insert(
            "index.html".to_string(),
            FragmentList {
                fragments: vec![WebUIFragment::component("missing-component")],
            },
        );

        let protocol = WebUIProtocol::new(fragments);
        let state = test_json!({});

        // Create a test writer
        let mut writer = TestWriter::new();

        // Handle the protocol
        let result = handle(
            &protocol,
            &state,
            &RenderOptions::new("index.html", "/"),
            &mut writer,
        );

        // Expect an error
        assert!(result.is_err());
        if let Err(HandlerError::MissingFragment(fragment_id)) = result {
            assert_eq!(fragment_id, "missing-component");
        } else {
            panic!("Expected MissingFragment error");
        }
    }

    #[test]
    fn test_missing_signal_renders_empty() {
        // A signal referencing a field absent from state should render as empty
        let mut fragments = HashMap::new();
        fragments.insert(
            "index.html".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::raw("Hello, "),
                    WebUIFragment::signal("missing_field", false),
                    WebUIFragment::raw("!"),
                ],
            },
        );

        let protocol = WebUIProtocol::new(fragments);
        let state = test_json!({});

        let mut writer = TestWriter::new();

        assert!(
            handle(
                &protocol,
                &state,
                &RenderOptions::new("index.html", "/"),
                &mut writer
            )
            .is_ok(),
            "Missing signal should not produce an error"
        );

        assert_eq!(writer.get_content(), "Hello, !");
        assert!(writer.is_ended());
    }

    // ── Boolean attribute rendering tests ─────────────────────────────

    #[test]
    fn test_boolean_attr_true() {
        let mut fragments = HashMap::new();
        fragments.insert(
            "index.html".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::raw("<button"),
                    WebUIFragment::attribute_boolean(
                        "disabled",
                        ConditionExpr::identifier("isDisabled"),
                    ),
                    WebUIFragment::raw(">Click</button>"),
                ],
            },
        );
        let protocol = WebUIProtocol::new(fragments);
        let state = test_json!({"isDisabled": true});
        let mut writer = TestWriter::new();
        handle(
            &protocol,
            &state,
            &RenderOptions::new("index.html", "/"),
            &mut writer,
        )
        .unwrap();
        assert_eq!(writer.get_content(), "<button disabled>Click</button>");
    }

    #[test]
    fn test_boolean_attr_false() {
        let mut fragments = HashMap::new();
        fragments.insert(
            "index.html".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::raw("<button"),
                    WebUIFragment::attribute_boolean(
                        "disabled",
                        ConditionExpr::identifier("isDisabled"),
                    ),
                    WebUIFragment::raw(">Click</button>"),
                ],
            },
        );
        let protocol = WebUIProtocol::new(fragments);
        let state = test_json!({"isDisabled": false});
        let mut writer = TestWriter::new();
        handle(
            &protocol,
            &state,
            &RenderOptions::new("index.html", "/"),
            &mut writer,
        )
        .unwrap();
        assert_eq!(writer.get_content(), "<button>Click</button>");
    }

    #[test]
    fn test_boolean_attr_missing() {
        let mut fragments = HashMap::new();
        fragments.insert(
            "index.html".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::raw("<input type=\"checkbox\""),
                    WebUIFragment::attribute_boolean(
                        "checked",
                        ConditionExpr::identifier("checked"),
                    ),
                    WebUIFragment::raw(">"),
                ],
            },
        );
        let protocol = WebUIProtocol::new(fragments);
        let state = test_json!({});
        let mut writer = TestWriter::new();
        handle(
            &protocol,
            &state,
            &RenderOptions::new("index.html", "/"),
            &mut writer,
        )
        .unwrap();
        assert_eq!(writer.get_content(), "<input type=\"checkbox\">");
    }

    #[test]
    fn test_boolean_attr_multiple() {
        let mut fragments = HashMap::new();
        fragments.insert(
            "index.html".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::raw("<input type=\"checkbox\""),
                    WebUIFragment::attribute_boolean(
                        "checked",
                        ConditionExpr::identifier("checked"),
                    ),
                    WebUIFragment::attribute_boolean(
                        "disabled",
                        ConditionExpr::identifier("disabled"),
                    ),
                    WebUIFragment::raw(">"),
                ],
            },
        );
        let protocol = WebUIProtocol::new(fragments);
        let state = test_json!({"checked": true, "disabled": false});
        let mut writer = TestWriter::new();
        handle(
            &protocol,
            &state,
            &RenderOptions::new("index.html", "/"),
            &mut writer,
        )
        .unwrap();
        assert_eq!(writer.get_content(), "<input type=\"checkbox\" checked>");
    }

    // ── Simple attribute rendering tests ──────────────────────────────

    #[test]
    fn test_attribute_with_value() {
        let mut fragments = HashMap::new();
        fragments.insert(
            "index.html".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::raw("<input"),
                    WebUIFragment::attribute("value", "inputValue"),
                    WebUIFragment::raw(">"),
                ],
            },
        );
        let protocol = WebUIProtocol::new(fragments);
        let state = test_json!({"inputValue": "Hello"});
        let mut writer = TestWriter::new();
        handle(
            &protocol,
            &state,
            &RenderOptions::new("index.html", "/"),
            &mut writer,
        )
        .unwrap();
        assert_eq!(writer.get_content(), "<input value=\"Hello\">");
    }

    #[test]
    fn test_attribute_with_falsy_numeric() {
        let mut fragments = HashMap::new();
        fragments.insert(
            "index.html".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::raw("<div name=\"test\""),
                    WebUIFragment::attribute("handle", "number"),
                    WebUIFragment::raw("></div>"),
                ],
            },
        );
        let protocol = WebUIProtocol::new(fragments);
        let state = test_json!({"number": 0});
        let mut writer = TestWriter::new();
        handle(
            &protocol,
            &state,
            &RenderOptions::new("index.html", "/"),
            &mut writer,
        )
        .unwrap();
        assert_eq!(
            writer.get_content(),
            "<div name=\"test\" handle=\"0\"></div>"
        );
    }

    // ── Dynamic attribute escaping for non-string JSON types ─────────

    #[test]
    fn test_attribute_array_value_is_escaped() {
        let mut fragments = HashMap::new();
        fragments.insert(
            "index.html".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::raw("<a"),
                    WebUIFragment::attribute("href", "value"),
                    WebUIFragment::raw(">demo</a>"),
                ],
            },
        );
        let protocol = WebUIProtocol::new(fragments);
        let state = test_json!({"value": ["\" autofocus onfocus=alert(1) x=\""]});
        let mut writer = TestWriter::new();
        handle(
            &protocol,
            &state,
            &RenderOptions::new("index.html", "/"),
            &mut writer,
        )
        .unwrap();
        let content = writer.get_content();
        // All inner double quotes must be entity-escaped so that the
        // browser never sees a second attribute boundary.
        assert!(
            content.contains("&quot;"),
            "Double quotes inside attribute value must be escaped: {content}"
        );
        // The href attribute value must be a single contiguous quoted
        // string — no extra attributes should appear.
        assert_eq!(
            content.matches("=\"").count(),
            1,
            "Only one attribute assignment expected: {content}"
        );
    }

    #[test]
    fn test_attribute_object_value_is_escaped() {
        let mut fragments = HashMap::new();
        fragments.insert(
            "index.html".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::raw("<div"),
                    WebUIFragment::attribute("data-cfg", "cfg"),
                    WebUIFragment::raw("></div>"),
                ],
            },
        );
        let protocol = WebUIProtocol::new(fragments);
        let state = test_json!({"cfg": {"key": "\" onfocus=alert(1) x=\""}});
        let mut writer = TestWriter::new();
        handle(
            &protocol,
            &state,
            &RenderOptions::new("index.html", "/"),
            &mut writer,
        )
        .unwrap();
        let content = writer.get_content();
        assert!(
            content.contains("&quot;"),
            "Double quotes inside attribute value must be escaped: {content}"
        );
        assert_eq!(
            content.matches("=\"").count(),
            1,
            "Only one attribute assignment expected: {content}"
        );
    }

    // ── Template attribute rendering tests ────────────────────────────

    #[test]
    fn test_mixed_attribute_template() {
        let mut fragments = HashMap::new();
        fragments.insert(
            "index.html".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::raw("<input"),
                    WebUIFragment::attribute_template("value", "attr-1"),
                    WebUIFragment::raw(">"),
                ],
            },
        );
        fragments.insert(
            "attr-1".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::raw("hello "),
                    WebUIFragment::signal("item", false),
                ],
            },
        );
        let protocol = WebUIProtocol::new(fragments);
        let state = test_json!({"item": "world"});
        let mut writer = TestWriter::new();
        handle(
            &protocol,
            &state,
            &RenderOptions::new("index.html", "/"),
            &mut writer,
        )
        .unwrap();
        assert_eq!(writer.get_content(), "<input value=\"hello world\">");
    }

    // ── Raw signal rendering test ─────────────────────────────────────

    #[test]
    fn test_raw_signal_not_escaped() {
        let mut fragments = HashMap::new();
        fragments.insert(
            "index.html".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::signal("html", false),
                    WebUIFragment::signal("html", true),
                ],
            },
        );
        let protocol = WebUIProtocol::new(fragments);
        let state = test_json!({"html": "<strong>hi</strong>"});
        let mut writer = TestWriter::new();
        handle(
            &protocol,
            &state,
            &RenderOptions::new("index.html", "/"),
            &mut writer,
        )
        .unwrap();
        assert_eq!(
            writer.get_content(),
            "&lt;strong&gt;hi&lt;&#x2F;strong&gt;<strong>hi</strong>"
        );
    }

    // ── Nested for loop tests ─────────────────────────────────────────

    #[test]
    fn test_nested_for_loop() {
        let mut fragments = HashMap::new();
        fragments.insert(
            "index.html".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::raw("<div>"),
                    WebUIFragment::for_loop("outerItem", "outerItems", "outer"),
                    WebUIFragment::raw("</div>"),
                ],
            },
        );
        fragments.insert(
            "outer".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::raw("<div>"),
                    WebUIFragment::for_loop("innerItem", "outerItem.innerItems", "inner"),
                    WebUIFragment::raw("</div>"),
                ],
            },
        );
        fragments.insert(
            "inner".to_string(),
            FragmentList {
                fragments: vec![WebUIFragment::raw("<span>Inner</span>")],
            },
        );
        let protocol = WebUIProtocol::new(fragments);
        let state = test_json!({
            "outerItems": [
                {"innerItems": [{"name": "A"}, {"name": "B"}]},
                {"innerItems": [{"name": "C"}]}
            ]
        });
        let mut writer = TestWriter::new();
        handle(
            &protocol,
            &state,
            &RenderOptions::new("index.html", "/"),
            &mut writer,
        )
        .unwrap();
        assert_eq!(
            writer.get_content(),
            "<div><div><span>Inner</span><span>Inner</span></div><div><span>Inner</span></div></div>"
        );
    }

    #[test]
    fn test_nested_for_with_signals() {
        let mut fragments = HashMap::new();
        fragments.insert(
            "index.html".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::raw("<div>"),
                    WebUIFragment::for_loop("outerItem", "outerItems", "outerTemplate"),
                    WebUIFragment::raw("</div>"),
                ],
            },
        );
        fragments.insert(
            "outerTemplate".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::raw("<div>"),
                    WebUIFragment::for_loop("innerItem", "outerItem.innerItems", "innerTemplate"),
                    WebUIFragment::raw("</div>"),
                ],
            },
        );
        fragments.insert(
            "innerTemplate".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::raw("<span>"),
                    WebUIFragment::signal("innerItem.name", false),
                    WebUIFragment::raw("</span>"),
                ],
            },
        );
        let protocol = WebUIProtocol::new(fragments);
        let state = test_json!({
            "outerItems": [
                {"innerItems": [{"name": "Item1"}, {"name": "Item2"}]},
                {"innerItems": [{"name": "Item3"}, {"name": "Item4"}]}
            ]
        });
        let mut writer = TestWriter::new();
        handle(
            &protocol,
            &state,
            &RenderOptions::new("index.html", "/"),
            &mut writer,
        )
        .unwrap();
        assert_eq!(
            writer.get_content(),
            "<div><div><span>Item1</span><span>Item2</span></div><div><span>Item3</span><span>Item4</span></div></div>"
        );
    }

    #[test]
    fn test_nested_for_with_global_state() {
        let mut fragments = HashMap::new();
        fragments.insert(
            "index.html".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::raw("<div>"),
                    WebUIFragment::for_loop("outerItem", "outerItems", "outerTemplate"),
                    WebUIFragment::raw("</div>"),
                ],
            },
        );
        fragments.insert(
            "outerTemplate".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::raw("<div>"),
                    WebUIFragment::signal("globalOuter", false),
                    WebUIFragment::for_loop("innerItem", "outerItem.innerItems", "innerTemplate"),
                    WebUIFragment::raw("</div>"),
                ],
            },
        );
        fragments.insert(
            "innerTemplate".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::raw("<span>"),
                    WebUIFragment::signal("innerItem.name", false),
                    WebUIFragment::signal("globalInner", false),
                    WebUIFragment::raw("</span>"),
                ],
            },
        );
        let protocol = WebUIProtocol::new(fragments);
        let state = test_json!({
            "globalOuter": "GO",
            "globalInner": "GI",
            "outerItems": [
                {"innerItems": [{"name": "Item1"}, {"name": "Item2"}]},
                {"innerItems": [{"name": "Item3"}, {"name": "Item4"}]}
            ]
        });
        let mut writer = TestWriter::new();
        handle(
            &protocol,
            &state,
            &RenderOptions::new("index.html", "/"),
            &mut writer,
        )
        .unwrap();
        assert_eq!(
            writer.get_content(),
            "<div><div>GO<span>Item1GI</span><span>Item2GI</span></div><div>GO<span>Item3GI</span><span>Item4GI</span></div></div>"
        );
    }

    // ── For + If state scoping tests ──────────────────────────────────

    #[test]
    fn test_if_in_for_uses_local_state() {
        let mut fragments = HashMap::new();
        fragments.insert(
            "index.html".to_string(),
            FragmentList {
                fragments: vec![WebUIFragment::for_loop("item", "items", "item-tpl")],
            },
        );
        fragments.insert(
            "item-tpl".to_string(),
            FragmentList {
                fragments: vec![WebUIFragment::if_cond(
                    ConditionExpr::identifier("item.visible"),
                    "visible-tpl",
                )],
            },
        );
        fragments.insert(
            "visible-tpl".to_string(),
            FragmentList {
                fragments: vec![WebUIFragment::signal("item.name", false)],
            },
        );
        let protocol = WebUIProtocol::new(fragments);
        let state = test_json!({"items": [{"name": "Show", "visible": true}, {"name": "Hide", "visible": false}]});
        let mut writer = TestWriter::new();
        handle(
            &protocol,
            &state,
            &RenderOptions::new("index.html", "/"),
            &mut writer,
        )
        .unwrap();
        assert_eq!(writer.get_content(), "Show");
    }

    #[test]
    fn test_for_if_local_overrides_global() {
        let mut fragments = HashMap::new();
        fragments.insert(
            "index.html".to_string(),
            FragmentList {
                fragments: vec![WebUIFragment::for_loop("item", "items", "item-tpl")],
            },
        );
        fragments.insert(
            "item-tpl".to_string(),
            FragmentList {
                fragments: vec![WebUIFragment::if_cond(
                    ConditionExpr::identifier("item.flag"),
                    "show-tpl",
                )],
            },
        );
        fragments.insert(
            "show-tpl".to_string(),
            FragmentList {
                fragments: vec![WebUIFragment::raw("yes")],
            },
        );
        let protocol = WebUIProtocol::new(fragments);
        // Global flag is true, but local item.flag is false for second item
        let state = test_json!({"flag": true, "items": [{"flag": true}, {"flag": false}]});
        let mut writer = TestWriter::new();
        handle(
            &protocol,
            &state,
            &RenderOptions::new("index.html", "/"),
            &mut writer,
        )
        .unwrap();
        assert_eq!(writer.get_content(), "yes");
    }

    // ── Component attribute state tests ───────────────────────────────

    #[test]
    fn test_component_attr_state_simple() {
        let mut fragments = HashMap::new();
        fragments.insert(
            "index.html".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::raw("<my-comp"),
                    WebUIFragment {
                        fragment: Some(web_ui_fragment::Fragment::Attribute(
                            WebUIFragmentAttribute {
                                name: "title".into(),
                                value: "Attribute Title".into(),
                                attr_start: true,
                                raw_value: true,
                                ..Default::default()
                            },
                        )),
                    },
                    WebUIFragment::raw(">"),
                    WebUIFragment::component("my-comp"),
                    WebUIFragment::raw("</my-comp>"),
                ],
            },
        );
        fragments.insert(
            "my-comp".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::raw("<span>"),
                    WebUIFragment::signal("title", false),
                    WebUIFragment::raw("</span>"),
                ],
            },
        );
        let protocol = WebUIProtocol::new(fragments);
        let state = test_json!({"title": "Global Title"});
        let mut writer = TestWriter::new();
        handle(
            &protocol,
            &state,
            &RenderOptions::new("index.html", "/"),
            &mut writer,
        )
        .unwrap();
        assert_eq!(
            writer.get_content(),
            "<my-comp title=\"Attribute Title\"><span>Attribute Title</span></my-comp>"
        );
    }

    #[test]
    fn test_component_attr_state_template() {
        let mut fragments = HashMap::new();
        fragments.insert(
            "index.html".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::raw("<my-comp"),
                    WebUIFragment {
                        fragment: Some(web_ui_fragment::Fragment::Attribute(
                            WebUIFragmentAttribute {
                                name: "title".into(),
                                template: "title-attr".into(),
                                attr_start: true,
                                ..Default::default()
                            },
                        )),
                    },
                    WebUIFragment::raw(">"),
                    WebUIFragment::component("my-comp"),
                    WebUIFragment::raw("</my-comp>"),
                ],
            },
        );
        fragments.insert(
            "title-attr".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::raw("hello "),
                    WebUIFragment::signal("item", false),
                ],
            },
        );
        fragments.insert(
            "my-comp".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::raw("<span>"),
                    WebUIFragment::signal("title", false),
                    WebUIFragment::raw("</span>"),
                ],
            },
        );
        let protocol = WebUIProtocol::new(fragments);
        let state = test_json!({"item": "<world>"});
        let mut writer = TestWriter::new();
        handle(
            &protocol,
            &state,
            &RenderOptions::new("index.html", "/"),
            &mut writer,
        )
        .unwrap();
        assert_eq!(
            writer.get_content(),
            "<my-comp title=\"hello &lt;world&gt;\"><span>hello &lt;world&gt;</span></my-comp>"
        );
    }

    #[test]
    fn test_component_attr_camel_case() {
        let mut fragments = HashMap::new();
        fragments.insert(
            "index.html".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::raw("<my-comp"),
                    WebUIFragment {
                        fragment: Some(web_ui_fragment::Fragment::Attribute(
                            WebUIFragmentAttribute {
                                name: "data-title".into(),
                                template: "dt-attr".into(),
                                attr_start: true,
                                ..Default::default()
                            },
                        )),
                    },
                    WebUIFragment::raw(">"),
                    WebUIFragment::component("my-comp"),
                    WebUIFragment::raw("</my-comp>"),
                ],
            },
        );
        fragments.insert(
            "dt-attr".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::raw("prefix "),
                    WebUIFragment::signal("item", false),
                ],
            },
        );
        fragments.insert(
            "my-comp".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::raw("<span>"),
                    WebUIFragment::signal("dataTitle", false),
                    WebUIFragment::raw("</span>"),
                ],
            },
        );
        let protocol = WebUIProtocol::new(fragments);
        let state = test_json!({"item": "a&b"});
        let mut writer = TestWriter::new();
        handle(
            &protocol,
            &state,
            &RenderOptions::new("index.html", "/"),
            &mut writer,
        )
        .unwrap();
        assert_eq!(
            writer.get_content(),
            "<my-comp data-title=\"prefix a&amp;b\"><span>prefix a&amp;b</span></my-comp>"
        );
    }

    #[test]
    fn test_component_complex_attr() {
        let mut fragments = HashMap::new();
        fragments.insert(
            "index.html".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::raw("<my-comp"),
                    WebUIFragment {
                        fragment: Some(web_ui_fragment::Fragment::Attribute(
                            WebUIFragmentAttribute {
                                name: ":item".into(),
                                value: "complexItem".into(),
                                attr_start: true,
                                complex: true,
                                ..Default::default()
                            },
                        )),
                    },
                    WebUIFragment::raw(">"),
                    WebUIFragment::component("my-comp"),
                    WebUIFragment::raw("</my-comp>"),
                ],
            },
        );
        fragments.insert(
            "my-comp".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::raw("<span>"),
                    WebUIFragment::signal("item.foo", false),
                    WebUIFragment::raw("</span><p>"),
                    WebUIFragment::signal("item.bar", false),
                    WebUIFragment::raw("</p>"),
                ],
            },
        );
        let protocol = WebUIProtocol::new(fragments);
        let state = test_json!({"complexItem": {"foo": 1, "bar": "true"}});
        let mut writer = TestWriter::new();
        handle(
            &protocol,
            &state,
            &RenderOptions::new("index.html", "/"),
            &mut writer,
        )
        .unwrap();
        assert_eq!(
            writer.get_content(),
            "<my-comp><span>1</span><p>true</p></my-comp>"
        );
    }

    #[test]
    fn test_component_no_parent_pollution() {
        let mut fragments = HashMap::new();
        fragments.insert(
            "index.html".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::raw("<parent"),
                    WebUIFragment {
                        fragment: Some(web_ui_fragment::Fragment::Attribute(
                            WebUIFragmentAttribute {
                                name: "var".into(),
                                value: "var".into(),
                                attr_start: true,
                                ..Default::default()
                            },
                        )),
                    },
                    WebUIFragment::raw(">"),
                    WebUIFragment::component("parent"),
                    WebUIFragment::raw("</parent>"),
                ],
            },
        );
        fragments.insert(
            "parent".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::raw("Before: "),
                    WebUIFragment::signal("var", false),
                    WebUIFragment::raw("<child foo"),
                    WebUIFragment {
                        fragment: Some(web_ui_fragment::Fragment::Attribute(
                            WebUIFragmentAttribute {
                                name: "var".into(),
                                value: "replaced".into(),
                                raw_value: true,
                                ..Default::default()
                            },
                        )),
                    },
                    WebUIFragment::raw(">"),
                    WebUIFragment::component("child"),
                    WebUIFragment::raw("Label</child>After: "),
                    WebUIFragment::signal("var", false),
                ],
            },
        );
        fragments.insert("child".to_string(), FragmentList { fragments: vec![] });
        let protocol = WebUIProtocol::new(fragments);
        let state = test_json!({"var": "original"});
        let mut writer = TestWriter::new();
        handle(
            &protocol,
            &state,
            &RenderOptions::new("index.html", "/"),
            &mut writer,
        )
        .unwrap();
        assert_eq!(
            writer.get_content(),
            "<parent var=\"original\">Before: original<child foo var=\"replaced\">Label</child>After: original</parent>"
        );
    }

    #[test]
    fn test_component_boolean_attr_state() {
        let mut fragments = HashMap::new();
        fragments.insert(
            "index.html".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::raw("<my-comp"),
                    WebUIFragment {
                        fragment: Some(web_ui_fragment::Fragment::Attribute(
                            WebUIFragmentAttribute {
                                name: "disabled".into(),
                                attr_start: true,
                                condition_tree: Some(ConditionExpr::identifier("isDisabled")),
                                ..Default::default()
                            },
                        )),
                    },
                    WebUIFragment::raw(">"),
                    WebUIFragment::component("my-comp"),
                    WebUIFragment::raw("</my-comp>"),
                ],
            },
        );
        fragments.insert(
            "my-comp".to_string(),
            FragmentList {
                fragments: vec![WebUIFragment::if_cond(
                    ConditionExpr::identifier("disabled"),
                    "show",
                )],
            },
        );
        fragments.insert(
            "show".to_string(),
            FragmentList {
                fragments: vec![WebUIFragment::raw("disabled!")],
            },
        );
        let protocol = WebUIProtocol::new(fragments);
        let state = test_json!({"isDisabled": true});
        let mut writer = TestWriter::new();
        handle(
            &protocol,
            &state,
            &RenderOptions::new("index.html", "/"),
            &mut writer,
        )
        .unwrap();
        assert_eq!(
            writer.get_content(),
            "<my-comp disabled>disabled!</my-comp>"
        );
    }

    // ===== HTML Escape Tests (ported from utils.test.js escapeHtml) =====

    /// Helper: render a signal value through the handler and return the escaped output.
    fn render_signal(value: &str) -> String {
        let mut fragments = HashMap::new();
        fragments.insert(
            "index.html".to_string(),
            FragmentList {
                fragments: vec![WebUIFragment::signal("v", false)],
            },
        );
        let protocol = WebUIProtocol::new(fragments);
        let state = test_json!({"v": value});
        let mut writer = TestWriter::new();
        handle(
            &protocol,
            &state,
            &RenderOptions::new("index.html", "/"),
            &mut writer,
        )
        .unwrap();
        writer.get_content()
    }

    #[test]
    fn test_escape_ampersand() {
        assert_eq!(render_signal("&"), "&amp;");
    }

    #[test]
    fn test_escape_less_than() {
        assert_eq!(render_signal("<"), "&lt;");
    }

    #[test]
    fn test_escape_greater_than() {
        assert_eq!(render_signal(">"), "&gt;");
    }

    #[test]
    fn test_escape_double_quote() {
        assert_eq!(render_signal("\""), "&quot;");
    }

    #[test]
    fn test_escape_single_quote() {
        // encode_safe escapes ' as &#x27;
        let result = render_signal("'");
        assert_eq!(
            result, "&#x27;",
            "Expected &#x27; for single quote, got: {result}"
        );
    }

    #[test]
    fn test_escape_multiple_special_chars() {
        let result = render_signal("<script>alert('xss');</script>");
        assert!(
            result.contains("&lt;") && result.contains("&gt;"),
            "Expected escaped HTML, got: {}",
            result
        );
        assert!(
            !result.contains("<script>"),
            "Should not contain raw <script> tag"
        );
    }

    #[test]
    fn test_escape_no_special_chars() {
        assert_eq!(render_signal("Hello World"), "Hello World");
    }

    #[test]
    fn test_escape_empty_string() {
        assert_eq!(render_signal(""), "");
    }

    #[test]
    fn test_escape_special_at_beginning() {
        let result = render_signal("<Hello");
        assert!(
            result.starts_with("&lt;"),
            "Expected &lt; at start, got: {}",
            result
        );
    }

    #[test]
    fn test_escape_special_at_end() {
        let result = render_signal("Hello>");
        assert!(
            result.ends_with("&gt;"),
            "Expected &gt; at end, got: {}",
            result
        );
    }

    #[test]
    fn test_escape_special_in_middle() {
        let result = render_signal("Hel&lo");
        assert!(
            result.contains("&amp;"),
            "Expected &amp; in middle, got: {}",
            result
        );
    }

    // ── GROUP 5: Boolean Attribute Edge Cases ─────────────────────────

    #[test]
    fn test_boolean_attr_truthy_values() {
        // checked: 1
        {
            let mut fragments = HashMap::new();
            fragments.insert(
                "index.html".to_string(),
                FragmentList {
                    fragments: vec![
                        WebUIFragment::raw("<input"),
                        WebUIFragment::attribute_boolean(
                            "checked",
                            ConditionExpr::identifier("checked"),
                        ),
                        WebUIFragment::raw(">"),
                    ],
                },
            );
            let protocol = WebUIProtocol::new(fragments);
            let state = test_json!({"checked": 1});
            let mut writer = TestWriter::new();
            handle(
                &protocol,
                &state,
                &RenderOptions::new("index.html", "/"),
                &mut writer,
            )
            .unwrap();
            assert_eq!(writer.get_content(), "<input checked>");
        }
        // checked: "yes"
        {
            let mut fragments = HashMap::new();
            fragments.insert(
                "index.html".to_string(),
                FragmentList {
                    fragments: vec![
                        WebUIFragment::raw("<input"),
                        WebUIFragment::attribute_boolean(
                            "checked",
                            ConditionExpr::identifier("checked"),
                        ),
                        WebUIFragment::raw(">"),
                    ],
                },
            );
            let protocol = WebUIProtocol::new(fragments);
            let state = test_json!({"checked": "yes"});
            let mut writer = TestWriter::new();
            handle(
                &protocol,
                &state,
                &RenderOptions::new("index.html", "/"),
                &mut writer,
            )
            .unwrap();
            assert_eq!(writer.get_content(), "<input checked>");
        }
        // checked: {} (empty object is truthy)
        {
            let mut fragments = HashMap::new();
            fragments.insert(
                "index.html".to_string(),
                FragmentList {
                    fragments: vec![
                        WebUIFragment::raw("<input"),
                        WebUIFragment::attribute_boolean(
                            "checked",
                            ConditionExpr::identifier("checked"),
                        ),
                        WebUIFragment::raw(">"),
                    ],
                },
            );
            let protocol = WebUIProtocol::new(fragments);
            let state = test_json!({"checked": {}});
            let mut writer = TestWriter::new();
            handle(
                &protocol,
                &state,
                &RenderOptions::new("index.html", "/"),
                &mut writer,
            )
            .unwrap();
            // Empty object is falsy in this expression evaluator
            assert_eq!(writer.get_content(), "<input>");
        }
        // checked: "false" (string "false" is truthy)
        {
            let mut fragments = HashMap::new();
            fragments.insert(
                "index.html".to_string(),
                FragmentList {
                    fragments: vec![
                        WebUIFragment::raw("<input"),
                        WebUIFragment::attribute_boolean(
                            "checked",
                            ConditionExpr::identifier("checked"),
                        ),
                        WebUIFragment::raw(">"),
                    ],
                },
            );
            let protocol = WebUIProtocol::new(fragments);
            let state = test_json!({"checked": "false"});
            let mut writer = TestWriter::new();
            handle(
                &protocol,
                &state,
                &RenderOptions::new("index.html", "/"),
                &mut writer,
            )
            .unwrap();
            assert_eq!(writer.get_content(), "<input checked>");
        }
    }

    #[test]
    fn test_boolean_attr_falsy_values() {
        // checked: 0
        {
            let mut fragments = HashMap::new();
            fragments.insert(
                "index.html".to_string(),
                FragmentList {
                    fragments: vec![
                        WebUIFragment::raw("<input"),
                        WebUIFragment::attribute_boolean(
                            "checked",
                            ConditionExpr::identifier("checked"),
                        ),
                        WebUIFragment::raw(">"),
                    ],
                },
            );
            let protocol = WebUIProtocol::new(fragments);
            let state = test_json!({"checked": 0});
            let mut writer = TestWriter::new();
            handle(
                &protocol,
                &state,
                &RenderOptions::new("index.html", "/"),
                &mut writer,
            )
            .unwrap();
            assert_eq!(writer.get_content(), "<input>");
        }
        // checked: ""
        {
            let mut fragments = HashMap::new();
            fragments.insert(
                "index.html".to_string(),
                FragmentList {
                    fragments: vec![
                        WebUIFragment::raw("<input"),
                        WebUIFragment::attribute_boolean(
                            "checked",
                            ConditionExpr::identifier("checked"),
                        ),
                        WebUIFragment::raw(">"),
                    ],
                },
            );
            let protocol = WebUIProtocol::new(fragments);
            let state = test_json!({"checked": ""});
            let mut writer = TestWriter::new();
            handle(
                &protocol,
                &state,
                &RenderOptions::new("index.html", "/"),
                &mut writer,
            )
            .unwrap();
            assert_eq!(writer.get_content(), "<input>");
        }
        // checked: false
        {
            let mut fragments = HashMap::new();
            fragments.insert(
                "index.html".to_string(),
                FragmentList {
                    fragments: vec![
                        WebUIFragment::raw("<input"),
                        WebUIFragment::attribute_boolean(
                            "checked",
                            ConditionExpr::identifier("checked"),
                        ),
                        WebUIFragment::raw(">"),
                    ],
                },
            );
            let protocol = WebUIProtocol::new(fragments);
            let state = test_json!({"checked": false});
            let mut writer = TestWriter::new();
            handle(
                &protocol,
                &state,
                &RenderOptions::new("index.html", "/"),
                &mut writer,
            )
            .unwrap();
            assert_eq!(writer.get_content(), "<input>");
        }
        // no checked key at all
        {
            let mut fragments = HashMap::new();
            fragments.insert(
                "index.html".to_string(),
                FragmentList {
                    fragments: vec![
                        WebUIFragment::raw("<input"),
                        WebUIFragment::attribute_boolean(
                            "checked",
                            ConditionExpr::identifier("checked"),
                        ),
                        WebUIFragment::raw(">"),
                    ],
                },
            );
            let protocol = WebUIProtocol::new(fragments);
            let state = test_json!({});
            let mut writer = TestWriter::new();
            handle(
                &protocol,
                &state,
                &RenderOptions::new("index.html", "/"),
                &mut writer,
            )
            .unwrap();
            assert_eq!(writer.get_content(), "<input>");
        }
    }

    #[test]
    fn test_boolean_attr_expression_true() {
        let mut fragments = HashMap::new();
        fragments.insert(
            "index.html".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::raw("<button"),
                    WebUIFragment::attribute_boolean(
                        "disabled",
                        ConditionExpr::predicate("itemCount", ComparisonOperator::Equal, "5"),
                    ),
                    WebUIFragment::raw(">Click</button>"),
                ],
            },
        );
        let protocol = WebUIProtocol::new(fragments);
        let state = test_json!({"itemCount": 5});
        let mut writer = TestWriter::new();
        handle(
            &protocol,
            &state,
            &RenderOptions::new("index.html", "/"),
            &mut writer,
        )
        .unwrap();
        assert_eq!(writer.get_content(), "<button disabled>Click</button>");
    }

    #[test]
    fn test_boolean_attr_expression_false() {
        let mut fragments = HashMap::new();
        fragments.insert(
            "index.html".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::raw("<button"),
                    WebUIFragment::attribute_boolean(
                        "disabled",
                        ConditionExpr::predicate("itemCount", ComparisonOperator::Equal, "5"),
                    ),
                    WebUIFragment::raw(">Click</button>"),
                ],
            },
        );
        let protocol = WebUIProtocol::new(fragments);
        let state = test_json!({"itemCount": 3});
        let mut writer = TestWriter::new();
        handle(
            &protocol,
            &state,
            &RenderOptions::new("index.html", "/"),
            &mut writer,
        )
        .unwrap();
        assert_eq!(writer.get_content(), "<button>Click</button>");
    }

    // ── GROUP 6: Mixed Attributes ─────────────────────────────────────

    #[test]
    fn test_nested_component_attr_capture() {
        let mut fragments = HashMap::new();
        fragments.insert(
            "index.html".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::raw("<parent-component"),
                    WebUIFragment {
                        fragment: Some(web_ui_fragment::Fragment::Attribute(
                            WebUIFragmentAttribute {
                                name: "title".into(),
                                template: "parent-title".into(),
                                attr_start: true,
                                ..Default::default()
                            },
                        )),
                    },
                    WebUIFragment::raw(">"),
                    WebUIFragment::component("parent-component"),
                    WebUIFragment::raw("</parent-component>"),
                ],
            },
        );
        fragments.insert(
            "parent-title".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::raw("Hello "),
                    WebUIFragment::signal("who", false),
                ],
            },
        );
        fragments.insert(
            "parent-component".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::raw("<child-component"),
                    WebUIFragment {
                        fragment: Some(web_ui_fragment::Fragment::Attribute(
                            WebUIFragmentAttribute {
                                name: "title".into(),
                                template: "child-title".into(),
                                attr_start: true,
                                ..Default::default()
                            },
                        )),
                    },
                    WebUIFragment::raw(">"),
                    WebUIFragment::component("child-component"),
                    WebUIFragment::raw("</child-component>"),
                ],
            },
        );
        fragments.insert(
            "child-title".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::raw("Child of "),
                    WebUIFragment::signal("title", false),
                ],
            },
        );
        fragments.insert(
            "child-component".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::raw("<span>"),
                    WebUIFragment::signal("title", false),
                    WebUIFragment::raw("</span>"),
                ],
            },
        );
        let protocol = WebUIProtocol::new(fragments);
        let state = test_json!({"who": "<world>"});
        let mut writer = TestWriter::new();
        handle(
            &protocol,
            &state,
            &RenderOptions::new("index.html", "/"),
            &mut writer,
        )
        .unwrap();
        assert_eq!(
            writer.get_content(),
            "<parent-component title=\"Hello &lt;world&gt;\"><child-component title=\"Child of Hello &lt;world&gt;\"><span>Child of Hello &lt;world&gt;</span></child-component></parent-component>"
        );
    }

    #[test]
    fn test_grandchild_attr_propagation() {
        let mut fragments = HashMap::new();
        fragments.insert(
            "index.html".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::raw("<parent-component"),
                    WebUIFragment {
                        fragment: Some(web_ui_fragment::Fragment::Attribute(
                            WebUIFragmentAttribute {
                                name: "title".into(),
                                template: "p-title".into(),
                                attr_start: true,
                                ..Default::default()
                            },
                        )),
                    },
                    WebUIFragment::raw(">"),
                    WebUIFragment::component("parent-component"),
                    WebUIFragment::raw("</parent-component>"),
                ],
            },
        );
        fragments.insert(
            "p-title".to_string(),
            FragmentList {
                fragments: vec![WebUIFragment::raw("P:"), WebUIFragment::signal("p", false)],
            },
        );
        fragments.insert(
            "parent-component".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::raw("<child-component"),
                    WebUIFragment {
                        fragment: Some(web_ui_fragment::Fragment::Attribute(
                            WebUIFragmentAttribute {
                                name: "title".into(),
                                template: "c-title".into(),
                                attr_start: true,
                                ..Default::default()
                            },
                        )),
                    },
                    WebUIFragment::raw(">"),
                    WebUIFragment::component("child-component"),
                    WebUIFragment::raw("</child-component>"),
                ],
            },
        );
        fragments.insert(
            "c-title".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::raw("C("),
                    WebUIFragment::signal("title", false),
                    WebUIFragment::raw(")-"),
                    WebUIFragment::signal("cExtra", false),
                ],
            },
        );
        fragments.insert(
            "child-component".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::raw("<grandchild-component"),
                    WebUIFragment {
                        fragment: Some(web_ui_fragment::Fragment::Attribute(
                            WebUIFragmentAttribute {
                                name: "title".into(),
                                value: "title".into(),
                                attr_start: true,
                                ..Default::default()
                            },
                        )),
                    },
                    WebUIFragment::raw(">"),
                    WebUIFragment::component("grandchild-component"),
                    WebUIFragment::raw("</grandchild-component>"),
                ],
            },
        );
        fragments.insert(
            "grandchild-component".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::raw("<span>"),
                    WebUIFragment::signal("title", false),
                    WebUIFragment::raw("</span>"),
                ],
            },
        );
        let protocol = WebUIProtocol::new(fragments);
        let state = test_json!({"p": "<p>", "cExtra": "x&y"});
        let mut writer = TestWriter::new();
        handle(
            &protocol,
            &state,
            &RenderOptions::new("index.html", "/"),
            &mut writer,
        )
        .unwrap();
        assert_eq!(
            writer.get_content(),
            "<parent-component title=\"P:&lt;p&gt;\"><child-component title=\"C(P:&lt;p&gt;)-x&amp;y\"><grandchild-component title=\"C(P:&lt;p&gt;)-x&amp;y\"><span>C(P:&lt;p&gt;)-x&amp;y</span></grandchild-component></child-component></parent-component>"
        );
    }

    #[test]
    fn test_for_loop_component_attr() {
        let mut fragments = HashMap::new();
        fragments.insert(
            "index.html".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::raw("<parent-component"),
                    WebUIFragment {
                        fragment: Some(web_ui_fragment::Fragment::Attribute(
                            WebUIFragmentAttribute {
                                name: "title".into(),
                                template: "parent-title-loop".into(),
                                attr_start: true,
                                ..Default::default()
                            },
                        )),
                    },
                    WebUIFragment::raw(">"),
                    WebUIFragment::component("parent-component"),
                    WebUIFragment::raw("</parent-component>"),
                ],
            },
        );
        fragments.insert(
            "parent-title-loop".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::raw("Parent:"),
                    WebUIFragment::signal("who", false),
                ],
            },
        );
        fragments.insert(
            "parent-component".to_string(),
            FragmentList {
                fragments: vec![WebUIFragment::for_loop("item", "items", "child-loop")],
            },
        );
        fragments.insert(
            "child-loop".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::raw("<child-component"),
                    WebUIFragment {
                        fragment: Some(web_ui_fragment::Fragment::Attribute(
                            WebUIFragmentAttribute {
                                name: "title".into(),
                                template: "child-title-loop".into(),
                                attr_start: true,
                                ..Default::default()
                            },
                        )),
                    },
                    WebUIFragment::raw(">"),
                    WebUIFragment::component("child-component"),
                    WebUIFragment::raw("</child-component>"),
                ],
            },
        );
        fragments.insert(
            "child-title-loop".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::raw("Hi "),
                    WebUIFragment::signal("item.name", false),
                    WebUIFragment::raw(" / "),
                    WebUIFragment::signal("title", false),
                ],
            },
        );
        fragments.insert(
            "child-component".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::raw("<span>"),
                    WebUIFragment::signal("title", false),
                    WebUIFragment::raw("</span>"),
                ],
            },
        );
        let protocol = WebUIProtocol::new(fragments);
        let state = test_json!({"who": "Bob", "items": [{"name": "A<1>"}, {"name": "B&2"}]});
        let mut writer = TestWriter::new();
        handle(
            &protocol,
            &state,
            &RenderOptions::new("index.html", "/"),
            &mut writer,
        )
        .unwrap();
        assert_eq!(
            writer.get_content(),
            "<parent-component title=\"Parent:Bob\"><child-component title=\"Hi A&lt;1&gt; &#x2F; Parent:Bob\"><span>Hi A&lt;1&gt; &#x2F; Parent:Bob</span></child-component><child-component title=\"Hi B&amp;2 &#x2F; Parent:Bob\"><span>Hi B&amp;2 &#x2F; Parent:Bob</span></child-component></parent-component>"
        );
    }

    #[test]
    fn test_multiple_template_attrs() {
        let mut fragments = HashMap::new();
        fragments.insert(
            "index.html".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::raw("<my-component"),
                    WebUIFragment {
                        fragment: Some(web_ui_fragment::Fragment::Attribute(
                            WebUIFragmentAttribute {
                                name: "title".into(),
                                template: "attr-title".into(),
                                attr_start: true,
                                ..Default::default()
                            },
                        )),
                    },
                    WebUIFragment {
                        fragment: Some(web_ui_fragment::Fragment::Attribute(
                            WebUIFragmentAttribute {
                                name: "data-title".into(),
                                template: "attr-data-title".into(),
                                attr_start: false,
                                ..Default::default()
                            },
                        )),
                    },
                    WebUIFragment {
                        fragment: Some(web_ui_fragment::Fragment::Attribute(
                            WebUIFragmentAttribute {
                                name: "aria-label".into(),
                                template: "attr-aria-label".into(),
                                attr_start: false,
                                ..Default::default()
                            },
                        )),
                    },
                    WebUIFragment::raw(">"),
                    WebUIFragment::component("my-component"),
                    WebUIFragment::raw("</my-component>"),
                ],
            },
        );
        fragments.insert(
            "attr-title".to_string(),
            FragmentList {
                fragments: vec![WebUIFragment::raw("T:"), WebUIFragment::signal("t", false)],
            },
        );
        fragments.insert(
            "attr-data-title".to_string(),
            FragmentList {
                fragments: vec![WebUIFragment::raw("D:"), WebUIFragment::signal("d", false)],
            },
        );
        fragments.insert(
            "attr-aria-label".to_string(),
            FragmentList {
                fragments: vec![WebUIFragment::raw("A:"), WebUIFragment::signal("a", false)],
            },
        );
        fragments.insert(
            "my-component".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::raw("<span>"),
                    WebUIFragment::signal("title", false),
                    WebUIFragment::raw("|"),
                    WebUIFragment::signal("dataTitle", false),
                    WebUIFragment::raw("|"),
                    WebUIFragment::signal("ariaLabel", false),
                    WebUIFragment::raw("</span>"),
                ],
            },
        );
        let protocol = WebUIProtocol::new(fragments);
        let state = test_json!({"t": "<t&1>", "d": "d<2>", "a": "a&3"});
        let mut writer = TestWriter::new();
        handle(
            &protocol,
            &state,
            &RenderOptions::new("index.html", "/"),
            &mut writer,
        )
        .unwrap();
        assert_eq!(
            writer.get_content(),
            "<my-component title=\"T:&lt;t&amp;1&gt;\" data-title=\"D:d&lt;2&gt;\" aria-label=\"A:a&amp;3\"><span>T:&lt;t&amp;1&gt;|D:d&lt;2&gt;|A:a&amp;3</span></my-component>"
        );
    }

    #[test]
    fn test_attr_priority_over_global() {
        let mut fragments = HashMap::new();
        fragments.insert(
            "index.html".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::raw("<my-component"),
                    WebUIFragment {
                        fragment: Some(web_ui_fragment::Fragment::Attribute(
                            WebUIFragmentAttribute {
                                name: "title".into(),
                                value: "Attribute Title".into(),
                                raw_value: true,
                                attr_start: true,
                                ..Default::default()
                            },
                        )),
                    },
                    WebUIFragment::raw(">"),
                    WebUIFragment::component("my-component"),
                    WebUIFragment::raw("</my-component>"),
                ],
            },
        );
        fragments.insert(
            "my-component".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::raw("<span>"),
                    WebUIFragment::signal("title", false),
                    WebUIFragment::raw("</span>"),
                ],
            },
        );
        let protocol = WebUIProtocol::new(fragments);
        let state = test_json!({"title": "Global Title"});
        let mut writer = TestWriter::new();
        handle(
            &protocol,
            &state,
            &RenderOptions::new("index.html", "/"),
            &mut writer,
        )
        .unwrap();
        assert_eq!(
            writer.get_content(),
            "<my-component title=\"Attribute Title\"><span>Attribute Title</span></my-component>"
        );
    }

    #[test]
    fn test_attr_priority_over_local_and_global() {
        let mut fragments = HashMap::new();
        fragments.insert(
            "index.html".to_string(),
            FragmentList {
                fragments: vec![WebUIFragment::for_loop("item", "items", "loop")],
            },
        );
        fragments.insert(
            "loop".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::raw("<my-component"),
                    WebUIFragment {
                        fragment: Some(web_ui_fragment::Fragment::Attribute(
                            WebUIFragmentAttribute {
                                name: "title".into(),
                                value: "Attribute Title".into(),
                                raw_value: true,
                                attr_start: true,
                                ..Default::default()
                            },
                        )),
                    },
                    WebUIFragment::raw(">"),
                    WebUIFragment::component("my-component"),
                    WebUIFragment::raw("</my-component>"),
                ],
            },
        );
        fragments.insert(
            "my-component".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::raw("<span>"),
                    WebUIFragment::signal("title", false),
                    WebUIFragment::raw("</span>"),
                ],
            },
        );
        let protocol = WebUIProtocol::new(fragments);
        let state = test_json!({"title": "Global Title", "items": [{"title": "Local Title"}]});
        let mut writer = TestWriter::new();
        handle(
            &protocol,
            &state,
            &RenderOptions::new("index.html", "/"),
            &mut writer,
        )
        .unwrap();
        assert_eq!(
            writer.get_content(),
            "<my-component title=\"Attribute Title\"><span>Attribute Title</span></my-component>"
        );
    }

    #[test]
    fn test_boolean_attr_first_component_attr() {
        let mut fragments = HashMap::new();
        fragments.insert(
            "index.html".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::raw("<my-component"),
                    WebUIFragment {
                        fragment: Some(web_ui_fragment::Fragment::Attribute(
                            WebUIFragmentAttribute {
                                name: "disabled".into(),
                                attr_start: true,
                                condition_tree: Some(ConditionExpr::identifier("isDisabled")),
                                ..Default::default()
                            },
                        )),
                    },
                    WebUIFragment {
                        fragment: Some(web_ui_fragment::Fragment::Attribute(
                            WebUIFragmentAttribute {
                                name: "label".into(),
                                value: "Component Label".into(),
                                raw_value: true,
                                attr_start: false,
                                ..Default::default()
                            },
                        )),
                    },
                    WebUIFragment::raw(">"),
                    WebUIFragment::component("my-component"),
                    WebUIFragment::raw("</my-component>"),
                ],
            },
        );
        fragments.insert(
            "my-component".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::if_cond(
                        ConditionExpr::identifier("disabled"),
                        "disabledTemplate",
                    ),
                    WebUIFragment::raw("<span>"),
                    WebUIFragment::signal("label", false),
                    WebUIFragment::raw("</span>"),
                ],
            },
        );
        fragments.insert(
            "disabledTemplate".to_string(),
            FragmentList {
                fragments: vec![WebUIFragment::raw("<div>Disabled</div>")],
            },
        );
        let protocol = WebUIProtocol::new(fragments);
        let state = test_json!({"isDisabled": true});
        let mut writer = TestWriter::new();
        handle(
            &protocol,
            &state,
            &RenderOptions::new("index.html", "/"),
            &mut writer,
        )
        .unwrap();
        assert_eq!(
            writer.get_content(),
            "<my-component disabled label=\"Component Label\"><div>Disabled</div><span>Component Label</span></my-component>"
        );
    }

    #[test]
    fn test_hyphenated_attr_camelcase() {
        let mut fragments = HashMap::new();
        fragments.insert(
            "index.html".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::raw("<my-component"),
                    WebUIFragment {
                        fragment: Some(web_ui_fragment::Fragment::Attribute(
                            WebUIFragmentAttribute {
                                name: "key-hyphen".into(),
                                value: "Local Value".into(),
                                raw_value: true,
                                attr_start: true,
                                ..Default::default()
                            },
                        )),
                    },
                    WebUIFragment::raw(">"),
                    WebUIFragment::component("my-component"),
                    WebUIFragment::raw("</my-component>"),
                ],
            },
        );
        fragments.insert(
            "my-component".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::raw("<span>"),
                    WebUIFragment::signal("keyHyphen", false),
                    WebUIFragment::raw("</span>"),
                ],
            },
        );
        let protocol = WebUIProtocol::new(fragments);
        let state = test_json!({"keyHyphen": "Global Value"});
        let mut writer = TestWriter::new();
        handle(
            &protocol,
            &state,
            &RenderOptions::new("index.html", "/"),
            &mut writer,
        )
        .unwrap();
        assert_eq!(
            writer.get_content(),
            "<my-component key-hyphen=\"Local Value\"><span>Local Value</span></my-component>"
        );
    }

    #[test]
    fn test_skipped_component_attrs() {
        // Skipped attributes: class, style, role, data-*, aria-*
        // Plus framework-specific prefixes/names that the parser marks with attr_skip.
        // These render on the HTML element but are NOT passed into component attribute state.
        let mut fragments = HashMap::new();
        fragments.insert(
            "index.html".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::raw("<test-component"),
                    // Skipped: class
                    WebUIFragment {
                        fragment: Some(web_ui_fragment::Fragment::Attribute(
                            WebUIFragmentAttribute {
                                name: "class".into(),
                                value: "skippedClass".into(),
                                attr_start: true,
                                attr_skip: true,
                                ..Default::default()
                            },
                        )),
                    },
                    // Skipped: style
                    WebUIFragment {
                        fragment: Some(web_ui_fragment::Fragment::Attribute(
                            WebUIFragmentAttribute {
                                name: "style".into(),
                                value: "skippedStyle".into(),
                                attr_skip: true,
                                ..Default::default()
                            },
                        )),
                    },
                    // Skipped: role
                    WebUIFragment {
                        fragment: Some(web_ui_fragment::Fragment::Attribute(
                            WebUIFragmentAttribute {
                                name: "role".into(),
                                value: "skippedRole".into(),
                                attr_skip: true,
                                ..Default::default()
                            },
                        )),
                    },
                    // Skipped: data-testid (data-* prefix)
                    WebUIFragment {
                        fragment: Some(web_ui_fragment::Fragment::Attribute(
                            WebUIFragmentAttribute {
                                name: "data-testid".into(),
                                value: "skippedDataTestid".into(),
                                attr_skip: true,
                                ..Default::default()
                            },
                        )),
                    },
                    // Skipped: aria-label (aria-* prefix)
                    WebUIFragment {
                        fragment: Some(web_ui_fragment::Fragment::Attribute(
                            WebUIFragmentAttribute {
                                name: "aria-label".into(),
                                value: "skippedAriaLabel".into(),
                                attr_skip: true,
                                ..Default::default()
                            },
                        )),
                    },
                    // NOT skipped: title
                    WebUIFragment {
                        fragment: Some(web_ui_fragment::Fragment::Attribute(
                            WebUIFragmentAttribute {
                                name: "title".into(),
                                value: "title".into(),
                                ..Default::default()
                            },
                        )),
                    },
                    WebUIFragment::raw(">"),
                    WebUIFragment::component("test-component"),
                    WebUIFragment::raw("</test-component>"),
                ],
            },
        );
        fragments.insert(
            "test-component".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::raw("<span>"),
                    WebUIFragment::signal("title", false),
                    WebUIFragment::raw("-"),
                    WebUIFragment::signal("class", false),
                    WebUIFragment::raw("-"),
                    WebUIFragment::signal("style", false),
                    WebUIFragment::raw("-"),
                    WebUIFragment::signal("role", false),
                    WebUIFragment::raw("-"),
                    WebUIFragment::signal("dataTestid", false),
                    WebUIFragment::raw("-"),
                    WebUIFragment::signal("ariaLabel", false),
                    WebUIFragment::raw("</span>"),
                ],
            },
        );
        let protocol = WebUIProtocol::new(fragments);
        let state = test_json!({
            "title": "Hello",
            "skippedClass": "my-class",
            "skippedStyle": "color:red",
            "skippedRole": "button",
            "skippedDataTestid": "test-id",
            "skippedAriaLabel": "label-text"
        });
        let mut writer = TestWriter::new();
        handle(
            &protocol,
            &state,
            &RenderOptions::new("index.html", "/"),
            &mut writer,
        )
        .unwrap();
        // Skipped attrs render on the element but their values are NOT accessible inside the component.
        // The component's signals for skipped attrs resolve to empty strings.
        // Only "title" (non-skipped) is accessible.
        assert_eq!(
            writer.get_content(),
            "<test-component class=\"my-class\" style=\"color:red\" role=\"button\" data-testid=\"test-id\" aria-label=\"label-text\" title=\"Hello\"><span>Hello-----</span></test-component>"
        );
    }

    // ── GROUP 7: Attribute Inheritance ─────────────────────────────────

    #[test]
    fn test_attr_inherit_parent_to_child() {
        let mut fragments = HashMap::new();
        fragments.insert(
            "index.html".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::raw("<parent-component"),
                    WebUIFragment {
                        fragment: Some(web_ui_fragment::Fragment::Attribute(
                            WebUIFragmentAttribute {
                                name: "title".into(),
                                value: "Parent Title".into(),
                                raw_value: true,
                                attr_start: true,
                                ..Default::default()
                            },
                        )),
                    },
                    WebUIFragment::raw(">"),
                    WebUIFragment::component("parent-component"),
                    WebUIFragment::raw("</parent-component>"),
                ],
            },
        );
        fragments.insert(
            "parent-component".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::raw("<h1>"),
                    WebUIFragment::signal("title", false),
                    WebUIFragment::raw("</h1><child-component"),
                    WebUIFragment {
                        fragment: Some(web_ui_fragment::Fragment::Attribute(
                            WebUIFragmentAttribute {
                                name: "title".into(),
                                value: "title".into(),
                                attr_start: true,
                                ..Default::default()
                            },
                        )),
                    },
                    WebUIFragment::raw(">"),
                    WebUIFragment::component("child-component"),
                    WebUIFragment::raw("</child-component>"),
                ],
            },
        );
        fragments.insert(
            "child-component".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::raw("<h2>"),
                    WebUIFragment::signal("title", false),
                    WebUIFragment::raw("</h2>"),
                ],
            },
        );
        let protocol = WebUIProtocol::new(fragments);
        let state = test_json!({});
        let mut writer = TestWriter::new();
        handle(
            &protocol,
            &state,
            &RenderOptions::new("index.html", "/"),
            &mut writer,
        )
        .unwrap();
        assert_eq!(
            writer.get_content(),
            "<parent-component title=\"Parent Title\"><h1>Parent Title</h1><child-component title=\"Parent Title\"><h2>Parent Title</h2></child-component></parent-component>"
        );
    }

    #[test]
    fn test_attr_inherit_deep() {
        let mut fragments = HashMap::new();
        fragments.insert(
            "index.html".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::raw("<parent-component"),
                    WebUIFragment {
                        fragment: Some(web_ui_fragment::Fragment::Attribute(
                            WebUIFragmentAttribute {
                                name: "title".into(),
                                value: "Parent Title".into(),
                                raw_value: true,
                                attr_start: true,
                                ..Default::default()
                            },
                        )),
                    },
                    WebUIFragment::raw(">"),
                    WebUIFragment::component("parent-component"),
                    WebUIFragment::raw("</parent-component>"),
                ],
            },
        );
        fragments.insert(
            "parent-component".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::raw("<child-component"),
                    WebUIFragment {
                        fragment: Some(web_ui_fragment::Fragment::Attribute(
                            WebUIFragmentAttribute {
                                name: "title".into(),
                                value: "Child Title".into(),
                                raw_value: true,
                                attr_start: true,
                                ..Default::default()
                            },
                        )),
                    },
                    WebUIFragment::raw(">"),
                    WebUIFragment::component("child-component"),
                    WebUIFragment::raw("</child-component>"),
                ],
            },
        );
        fragments.insert(
            "child-component".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::raw("<grandchild-component"),
                    WebUIFragment {
                        fragment: Some(web_ui_fragment::Fragment::Attribute(
                            WebUIFragmentAttribute {
                                name: "title".into(),
                                value: "title".into(),
                                attr_start: true,
                                ..Default::default()
                            },
                        )),
                    },
                    WebUIFragment::raw(">"),
                    WebUIFragment::component("grandchild-component"),
                    WebUIFragment::raw("</grandchild-component>"),
                ],
            },
        );
        fragments.insert(
            "grandchild-component".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::raw("<h3>"),
                    WebUIFragment::signal("title", false),
                    WebUIFragment::raw("</h3>"),
                ],
            },
        );
        let protocol = WebUIProtocol::new(fragments);
        let state = test_json!({});
        let mut writer = TestWriter::new();
        handle(
            &protocol,
            &state,
            &RenderOptions::new("index.html", "/"),
            &mut writer,
        )
        .unwrap();
        assert_eq!(
            writer.get_content(),
            "<parent-component title=\"Parent Title\"><child-component title=\"Child Title\"><grandchild-component title=\"Child Title\"><h3>Child Title</h3></grandchild-component></child-component></parent-component>"
        );
    }

    #[test]
    fn test_complex_attr_access() {
        let mut fragments = HashMap::new();
        fragments.insert(
            "index.html".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::raw("<my-component"),
                    WebUIFragment {
                        fragment: Some(web_ui_fragment::Fragment::Attribute(
                            WebUIFragmentAttribute {
                                name: ":item".into(),
                                value: "complexItem".into(),
                                attr_start: true,
                                complex: true,
                                ..Default::default()
                            },
                        )),
                    },
                    WebUIFragment::raw(">"),
                    WebUIFragment::component("my-component"),
                    WebUIFragment::raw("</my-component>"),
                ],
            },
        );
        fragments.insert(
            "my-component".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::raw("<span>"),
                    WebUIFragment::signal("item.foo", false),
                    WebUIFragment::raw("</span><p>"),
                    WebUIFragment::signal("item.bar", false),
                    WebUIFragment::raw("</p>"),
                ],
            },
        );
        let protocol = WebUIProtocol::new(fragments);
        let state = test_json!({"complexItem": {"foo": 1, "bar": "true"}});
        let mut writer = TestWriter::new();
        handle(
            &protocol,
            &state,
            &RenderOptions::new("index.html", "/"),
            &mut writer,
        )
        .unwrap();
        assert_eq!(
            writer.get_content(),
            "<my-component><span>1</span><p>true</p></my-component>"
        );
    }

    #[test]
    fn test_complex_attr_for_loop() {
        let mut fragments = HashMap::new();
        fragments.insert(
            "index.html".to_string(),
            FragmentList {
                fragments: vec![WebUIFragment::for_loop(
                    "item",
                    "list.items",
                    "listTemplate",
                )],
            },
        );
        fragments.insert(
            "listTemplate".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment {
                        fragment: Some(web_ui_fragment::Fragment::Attribute(
                            WebUIFragmentAttribute {
                                name: ":item".into(),
                                value: "item".into(),
                                attr_start: true,
                                complex: true,
                                ..Default::default()
                            },
                        )),
                    },
                    WebUIFragment::component("item_component"),
                ],
            },
        );
        fragments.insert(
            "item_component".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::raw("<span>"),
                    WebUIFragment::signal("item.name", false),
                    WebUIFragment::raw("</span>"),
                ],
            },
        );
        let protocol = WebUIProtocol::new(fragments);
        let state = test_json!({"list": {"items": [{"name": "Alice"}, {"name": "Bob"}]}});
        let mut writer = TestWriter::new();
        handle(
            &protocol,
            &state,
            &RenderOptions::new("index.html", "/"),
            &mut writer,
        )
        .unwrap();
        assert_eq!(writer.get_content(), "<span>Alice</span><span>Bob</span>");
    }

    #[test]
    fn test_complex_attr_nested_for() {
        let mut fragments = HashMap::new();
        fragments.insert(
            "index.html".to_string(),
            FragmentList {
                fragments: vec![WebUIFragment::for_loop(
                    "outer",
                    "data.outer",
                    "outerTemplate",
                )],
            },
        );
        fragments.insert(
            "outerTemplate".to_string(),
            FragmentList {
                fragments: vec![WebUIFragment::for_loop(
                    "middle",
                    "outer.middle",
                    "middleTemplate",
                )],
            },
        );
        fragments.insert(
            "middleTemplate".to_string(),
            FragmentList {
                fragments: vec![WebUIFragment::for_loop(
                    "inner",
                    "middle.inner",
                    "innerTemplate",
                )],
            },
        );
        fragments.insert(
            "innerTemplate".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::raw("<card"),
                    WebUIFragment {
                        fragment: Some(web_ui_fragment::Fragment::Attribute(
                            WebUIFragmentAttribute {
                                name: ":outer".into(),
                                value: "outer".into(),
                                attr_start: true,
                                complex: true,
                                ..Default::default()
                            },
                        )),
                    },
                    WebUIFragment {
                        fragment: Some(web_ui_fragment::Fragment::Attribute(
                            WebUIFragmentAttribute {
                                name: ":middle".into(),
                                value: "middle".into(),
                                attr_start: false,
                                complex: true,
                                ..Default::default()
                            },
                        )),
                    },
                    WebUIFragment {
                        fragment: Some(web_ui_fragment::Fragment::Attribute(
                            WebUIFragmentAttribute {
                                name: ":inner".into(),
                                value: "inner".into(),
                                attr_start: false,
                                complex: true,
                                ..Default::default()
                            },
                        )),
                    },
                    WebUIFragment::raw(">"),
                    WebUIFragment::component("card_component"),
                    WebUIFragment::raw("</card>"),
                ],
            },
        );
        fragments.insert(
            "card_component".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::raw("<p>"),
                    WebUIFragment::signal("outer.label", false),
                    WebUIFragment::raw(" / "),
                    WebUIFragment::signal("middle.label", false),
                    WebUIFragment::raw(" / "),
                    WebUIFragment::signal("inner.label", false),
                    WebUIFragment::raw("</p>"),
                ],
            },
        );
        let protocol = WebUIProtocol::new(fragments);
        let state = test_json!({"data": {"outer": [
            {"label": "Outer1", "middle": [{"label": "Middle1", "inner": [{"label": "Inner1A"}, {"label": "Inner1B"}]}]},
            {"label": "Outer2", "middle": [{"label": "Middle2", "inner": [{"label": "Inner2A"}]}]}
        ]}});
        let mut writer = TestWriter::new();
        handle(
            &protocol,
            &state,
            &RenderOptions::new("index.html", "/"),
            &mut writer,
        )
        .unwrap();
        assert_eq!(
            writer.get_content(),
            "<card><p>Outer1 / Middle1 / Inner1A</p></card><card><p>Outer1 / Middle1 / Inner1B</p></card><card><p>Outer2 / Middle2 / Inner2A</p></card>"
        );
    }

    // ── GROUP 8: Boolean Component State ──────────────────────────────

    #[test]
    fn test_bool_component_state_true() {
        let mut fragments = HashMap::new();
        fragments.insert(
            "index.html".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::raw("<my-component"),
                    WebUIFragment {
                        fragment: Some(web_ui_fragment::Fragment::Attribute(
                            WebUIFragmentAttribute {
                                name: "disabled".into(),
                                attr_start: true,
                                condition_tree: Some(ConditionExpr::identifier("isDisabled")),
                                ..Default::default()
                            },
                        )),
                    },
                    WebUIFragment::raw(">"),
                    WebUIFragment::component("my-component"),
                    WebUIFragment::raw("</my-component>"),
                ],
            },
        );
        fragments.insert(
            "my-component".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::if_cond(
                        ConditionExpr::identifier("disabled"),
                        "disabledTemplate",
                    ),
                    WebUIFragment::if_cond(
                        ConditionExpr::negated(ConditionExpr::identifier("disabled")),
                        "enabledTemplate",
                    ),
                ],
            },
        );
        fragments.insert(
            "disabledTemplate".to_string(),
            FragmentList {
                fragments: vec![WebUIFragment::raw("<span>Disabled</span>")],
            },
        );
        fragments.insert(
            "enabledTemplate".to_string(),
            FragmentList {
                fragments: vec![WebUIFragment::raw("<span>Enabled</span>")],
            },
        );
        let protocol = WebUIProtocol::new(fragments);
        let state = test_json!({"isDisabled": true});
        let mut writer = TestWriter::new();
        handle(
            &protocol,
            &state,
            &RenderOptions::new("index.html", "/"),
            &mut writer,
        )
        .unwrap();
        assert_eq!(
            writer.get_content(),
            "<my-component disabled><span>Disabled</span></my-component>"
        );
    }

    #[test]
    fn test_bool_component_state_false() {
        let mut fragments = HashMap::new();
        fragments.insert(
            "index.html".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::raw("<my-component"),
                    WebUIFragment {
                        fragment: Some(web_ui_fragment::Fragment::Attribute(
                            WebUIFragmentAttribute {
                                name: "disabled".into(),
                                attr_start: true,
                                condition_tree: Some(ConditionExpr::identifier("isDisabled")),
                                ..Default::default()
                            },
                        )),
                    },
                    WebUIFragment::raw(">"),
                    WebUIFragment::component("my-component"),
                    WebUIFragment::raw("</my-component>"),
                ],
            },
        );
        fragments.insert(
            "my-component".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::if_cond(
                        ConditionExpr::identifier("disabled"),
                        "disabledTemplate",
                    ),
                    WebUIFragment::if_cond(
                        ConditionExpr::negated(ConditionExpr::identifier("disabled")),
                        "enabledTemplate",
                    ),
                ],
            },
        );
        fragments.insert(
            "disabledTemplate".to_string(),
            FragmentList {
                fragments: vec![WebUIFragment::raw("<span>Disabled</span>")],
            },
        );
        fragments.insert(
            "enabledTemplate".to_string(),
            FragmentList {
                fragments: vec![WebUIFragment::raw("<span>Enabled</span>")],
            },
        );
        let protocol = WebUIProtocol::new(fragments);
        let state = test_json!({"isDisabled": false});
        let mut writer = TestWriter::new();
        handle(
            &protocol,
            &state,
            &RenderOptions::new("index.html", "/"),
            &mut writer,
        )
        .unwrap();
        assert_eq!(
            writer.get_content(),
            "<my-component><span>Enabled</span></my-component>"
        );
    }

    #[test]
    fn test_bool_component_state_forward() {
        let mut fragments = HashMap::new();
        fragments.insert(
            "index.html".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::raw("<parent-component"),
                    WebUIFragment {
                        fragment: Some(web_ui_fragment::Fragment::Attribute(
                            WebUIFragmentAttribute {
                                name: "disabled".into(),
                                attr_start: true,
                                condition_tree: Some(ConditionExpr::identifier("isDisabled")),
                                ..Default::default()
                            },
                        )),
                    },
                    WebUIFragment::raw(">"),
                    WebUIFragment::component("parent-component"),
                    WebUIFragment::raw("</parent-component>"),
                ],
            },
        );
        fragments.insert(
            "parent-component".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::if_cond(
                        ConditionExpr::identifier("disabled"),
                        "parentDisabledTemplate",
                    ),
                    WebUIFragment::raw("<child-component"),
                    WebUIFragment {
                        fragment: Some(web_ui_fragment::Fragment::Attribute(
                            WebUIFragmentAttribute {
                                name: "disabled".into(),
                                attr_start: true,
                                condition_tree: Some(ConditionExpr::identifier("disabled")),
                                ..Default::default()
                            },
                        )),
                    },
                    WebUIFragment::raw(">"),
                    WebUIFragment::component("child-component"),
                    WebUIFragment::raw("</child-component>"),
                ],
            },
        );
        fragments.insert(
            "parentDisabledTemplate".to_string(),
            FragmentList {
                fragments: vec![WebUIFragment::raw("<div>Parent Disabled</div>")],
            },
        );
        fragments.insert(
            "child-component".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::if_cond(
                        ConditionExpr::identifier("disabled"),
                        "childDisabledTemplate",
                    ),
                    WebUIFragment::if_cond(
                        ConditionExpr::negated(ConditionExpr::identifier("disabled")),
                        "childEnabledTemplate",
                    ),
                ],
            },
        );
        fragments.insert(
            "childDisabledTemplate".to_string(),
            FragmentList {
                fragments: vec![WebUIFragment::raw("<div>Child Disabled</div>")],
            },
        );
        fragments.insert(
            "childEnabledTemplate".to_string(),
            FragmentList {
                fragments: vec![WebUIFragment::raw("<div>Child Enabled</div>")],
            },
        );

        // Test case 1: isDisabled = true
        {
            let protocol = WebUIProtocol::new(fragments.clone());
            let state = test_json!({"isDisabled": true});
            let mut writer = TestWriter::new();
            handle(
                &protocol,
                &state,
                &RenderOptions::new("index.html", "/"),
                &mut writer,
            )
            .unwrap();
            assert_eq!(
                writer.get_content(),
                "<parent-component disabled><div>Parent Disabled</div><child-component disabled><div>Child Disabled</div></child-component></parent-component>"
            );
        }

        // Test case 2: isDisabled = false
        {
            let protocol = WebUIProtocol::new(fragments.clone());
            let state = test_json!({"isDisabled": false});
            let mut writer = TestWriter::new();
            handle(
                &protocol,
                &state,
                &RenderOptions::new("index.html", "/"),
                &mut writer,
            )
            .unwrap();
            assert_eq!(
                writer.get_content(),
                "<parent-component><child-component><div>Child Enabled</div></child-component></parent-component>"
            );
        }
    }

    // ── GROUP 9: Hydration (SKIP) ─────────────────────────────────────

    // TODO: test_hydration - requires FAST handler plugin integration; see plugin/fast.rs

    // ── Component tests ──────────────────────────────────────────────

    #[test]
    fn test_component_with_template() {
        let mut fragments = HashMap::new();
        fragments.insert(
            "index.html".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::raw("<custom-element>"),
                    WebUIFragment::component("custom-element"),
                    WebUIFragment::raw("</custom-element>"),
                ],
            },
        );
        fragments.insert(
            "custom-element".to_string(),
            FragmentList {
                fragments: vec![WebUIFragment::raw("<div>Custom Element</div>")],
            },
        );
        let protocol = WebUIProtocol::new(fragments);
        let state = test_json!({});
        let mut writer = TestWriter::new();
        handle(
            &protocol,
            &state,
            &RenderOptions::new("index.html", "/"),
            &mut writer,
        )
        .unwrap();
        assert_eq!(
            writer.get_content(),
            "<custom-element><div>Custom Element</div></custom-element>"
        );
        assert!(writer.is_ended());
    }

    #[test]
    fn test_component_with_slots() {
        let mut fragments = HashMap::new();
        fragments.insert(
            "index.html".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::raw("<custom-element appearance=\"subtle\">"),
                    WebUIFragment::component("custom-element"),
                    WebUIFragment::raw("Hello World</custom-element>"),
                ],
            },
        );
        fragments.insert(
            "custom-element".to_string(),
            FragmentList {
                fragments: vec![WebUIFragment::raw("<slot></slot>")],
            },
        );
        let protocol = WebUIProtocol::new(fragments);
        let state = test_json!({});
        let mut writer = TestWriter::new();
        handle(
            &protocol,
            &state,
            &RenderOptions::new("index.html", "/"),
            &mut writer,
        )
        .unwrap();
        assert_eq!(
            writer.get_content(),
            "<custom-element appearance=\"subtle\"><slot></slot>Hello World</custom-element>"
        );
        assert!(writer.is_ended());
    }

    #[test]
    fn test_multiple_nested_components() {
        let mut fragments = HashMap::new();
        fragments.insert(
            "index.html".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::raw("<div>"),
                    WebUIFragment::for_loop("item", "items", "templateRepeat"),
                    WebUIFragment::raw("</div>"),
                ],
            },
        );
        fragments.insert(
            "custom-button".to_string(),
            FragmentList {
                fragments: vec![WebUIFragment::raw("<slot></slot>")],
            },
        );
        fragments.insert(
            "custom-element".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::raw("<custom-child>"),
                    WebUIFragment::component("custom-child"),
                    WebUIFragment::raw("</custom-child><slot></slot>"),
                ],
            },
        );
        fragments.insert(
            "custom-child".to_string(),
            FragmentList {
                fragments: vec![WebUIFragment::raw("<h1>Hello World!</h1>")],
            },
        );
        fragments.insert(
            "templateRepeat".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::raw("<custom-element>"),
                    WebUIFragment::component("custom-element"),
                    WebUIFragment::raw("<custom-button>"),
                    WebUIFragment::component("custom-button"),
                    WebUIFragment::raw("Ok</custom-button></custom-element>"),
                ],
            },
        );
        let protocol = WebUIProtocol::new(fragments);
        let state = test_json!({"items": [{"name": "Item1"}]});
        let mut writer = TestWriter::new();
        handle(
            &protocol,
            &state,
            &RenderOptions::new("index.html", "/"),
            &mut writer,
        )
        .unwrap();
        assert_eq!(
            writer.get_content(),
            "<div><custom-element><custom-child><h1>Hello World!</h1></custom-child><slot></slot><custom-button><slot></slot>Ok</custom-button></custom-element></div>"
        );
        assert!(writer.is_ended());
    }

    // ── Conditional tests ────────────────────────────────────────────

    #[test]
    fn test_if_with_binary_expression() {
        let mut fragments = HashMap::new();
        fragments.insert(
            "index.html".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::raw("<div>"),
                    WebUIFragment::if_cond(
                        ConditionExpr::predicate("x", ComparisonOperator::GreaterThan, "5"),
                        "if-1",
                    ),
                    WebUIFragment::raw("</div>"),
                ],
            },
        );
        fragments.insert(
            "if-1".to_string(),
            FragmentList {
                fragments: vec![WebUIFragment::raw("<span>If 1</span>")],
            },
        );
        let protocol = WebUIProtocol::new(fragments);

        // True case: x = 10 > 5
        let state_true = test_json!({"x": 10});
        let mut writer_true = TestWriter::new();
        handle(
            &protocol,
            &state_true,
            &RenderOptions::new("index.html", "/"),
            &mut writer_true,
        )
        .unwrap();
        assert_eq!(writer_true.get_content(), "<div><span>If 1</span></div>");

        // False case: x = 1 <= 5
        let state_false = test_json!({"x": 1});
        let mut writer_false = TestWriter::new();
        handle(
            &protocol,
            &state_false,
            &RenderOptions::new("index.html", "/"),
            &mut writer_false,
        )
        .unwrap();
        assert_eq!(writer_false.get_content(), "<div></div>");
    }

    #[test]
    fn test_for_if_overlapping_local_state() {
        let mut fragments = HashMap::new();
        fragments.insert(
            "index.html".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::raw("<div>"),
                    WebUIFragment::for_loop("item", "items", "template1"),
                    WebUIFragment::raw("</div>"),
                ],
            },
        );
        fragments.insert(
            "template1".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::raw("<div>"),
                    WebUIFragment::if_cond(ConditionExpr::identifier("item.flag"), "ifBlock"),
                    WebUIFragment::raw("</div>"),
                ],
            },
        );
        fragments.insert(
            "ifBlock".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::raw("<span>"),
                    WebUIFragment::signal("item.label", false),
                    WebUIFragment::raw("</span>"),
                ],
            },
        );
        let protocol = WebUIProtocol::new(fragments);
        let state = test_json!({
            "flag": false,
            "items": [
                {"label": "A", "flag": true},
                {"label": "B", "flag": false},
                {"label": "C", "flag": true}
            ]
        });
        let mut writer = TestWriter::new();
        handle(
            &protocol,
            &state,
            &RenderOptions::new("index.html", "/"),
            &mut writer,
        )
        .unwrap();
        assert_eq!(
            writer.get_content(),
            "<div><div><span>A</span></div><div></div><div><span>C</span></div></div>"
        );
    }

    #[test]
    fn test_for_if_global_flag_no_effect() {
        let mut fragments = HashMap::new();
        fragments.insert(
            "index.html".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::raw("<div>"),
                    WebUIFragment::for_loop("item", "items", "template1"),
                    WebUIFragment::raw("</div>"),
                ],
            },
        );
        fragments.insert(
            "template1".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::raw("<div>"),
                    WebUIFragment::if_cond(ConditionExpr::identifier("item.flag"), "ifBlock"),
                    WebUIFragment::raw("</div>"),
                ],
            },
        );
        fragments.insert(
            "ifBlock".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::raw("<span>"),
                    WebUIFragment::signal("item.label", false),
                    WebUIFragment::raw("</span>"),
                ],
            },
        );
        let protocol = WebUIProtocol::new(fragments);
        let state = test_json!({
            "item": {"flag": true},
            "items": [
                {"label": "A", "flag": false},
                {"label": "B", "flag": true},
                {"label": "C", "flag": false}
            ]
        });
        let mut writer = TestWriter::new();
        handle(
            &protocol,
            &state,
            &RenderOptions::new("index.html", "/"),
            &mut writer,
        )
        .unwrap();
        assert_eq!(
            writer.get_content(),
            "<div><div></div><div><span>B</span></div><div></div></div>"
        );
    }

    // ── Recursive template test ──────────────────────────────────────

    #[test]
    fn test_recursive_template_refs() {
        let mut fragments = HashMap::new();
        fragments.insert(
            "index.html".to_string(),
            FragmentList {
                fragments: vec![WebUIFragment::for_loop("item", "items", "static")],
            },
        );
        fragments.insert(
            "static".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::raw("<div expanded=\""),
                    WebUIFragment::signal("item.expanded", false),
                    WebUIFragment::raw("\" class=\""),
                    WebUIFragment::signal("testScenario", false),
                    WebUIFragment::raw("\"><span>"),
                    WebUIFragment::signal("item.name", false),
                    WebUIFragment::raw("</span>"),
                    WebUIFragment::for_loop("item", "item.children", "static"),
                    WebUIFragment::raw("</div>"),
                ],
            },
        );
        let protocol = WebUIProtocol::new(fragments);
        let state = test_json!({
            "testScenario": "RecursiveTemplatesWithGlobalState",
            "items": [
                {"name": "A", "expanded": "false", "children": []},
                {"name": "B", "expanded": "true", "children": [
                    {"name": "C", "expanded": "false"},
                    {"name": "D", "expanded": "false"}
                ]},
                {"name": "E", "expanded": "false"}
            ]
        });
        let mut writer = TestWriter::new();
        handle(
            &protocol,
            &state,
            &RenderOptions::new("index.html", "/"),
            &mut writer,
        )
        .unwrap();
        assert_eq!(
            writer.get_content(),
            "<div expanded=\"false\" class=\"RecursiveTemplatesWithGlobalState\"><span>A</span></div><div expanded=\"true\" class=\"RecursiveTemplatesWithGlobalState\"><span>B</span><div expanded=\"false\" class=\"RecursiveTemplatesWithGlobalState\"><span>C</span></div><div expanded=\"false\" class=\"RecursiveTemplatesWithGlobalState\"><span>D</span></div></div><div expanded=\"false\" class=\"RecursiveTemplatesWithGlobalState\"><span>E</span></div>"
        );
    }

    // ── Advanced state management tests ──────────────────────────────

    #[test]
    fn test_component_in_for_no_local_access() {
        let mut fragments = HashMap::new();
        fragments.insert(
            "index.html".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::raw("<div>"),
                    WebUIFragment::for_loop("item", "items", "templateComponent"),
                    WebUIFragment::raw("</div>"),
                ],
            },
        );
        fragments.insert(
            "templateComponent".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::raw("<component-tag>"),
                    WebUIFragment::component("my-component"),
                    WebUIFragment::raw("</component-tag>"),
                ],
            },
        );
        fragments.insert(
            "my-component".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::raw("<span>"),
                    WebUIFragment::signal("name", false),
                    WebUIFragment::raw("</span>"),
                ],
            },
        );
        let protocol = WebUIProtocol::new(fragments);
        let state = test_json!({"items": [{"name": "Item1"}, {"name": "Item2"}]});
        let mut writer = TestWriter::new();
        handle(
            &protocol,
            &state,
            &RenderOptions::new("index.html", "/"),
            &mut writer,
        )
        .unwrap();
        assert_eq!(
            writer.get_content(),
            "<div><component-tag><span></span></component-tag><component-tag><span></span></component-tag></div>"
        );
    }

    #[test]
    fn test_nested_for_hierarchical_state() {
        let mut fragments = HashMap::new();
        fragments.insert(
            "index.html".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::raw("<div>"),
                    WebUIFragment::for_loop("outerItem", "outerItems", "outerTemplate"),
                    WebUIFragment::raw("</div>"),
                ],
            },
        );
        fragments.insert(
            "outerTemplate".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::raw("<section>"),
                    WebUIFragment::signal("globalPrefix", false),
                    WebUIFragment::signal("outerItem.outerLabel", false),
                    WebUIFragment::for_loop("innerItem", "outerItem.innerItems", "innerTemplate"),
                    WebUIFragment::raw("</section>"),
                ],
            },
        );
        fragments.insert(
            "innerTemplate".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::raw("<p>"),
                    WebUIFragment::signal("globalPrefix", false),
                    WebUIFragment::signal("outerItem.outerLabel", false),
                    WebUIFragment::raw(": "),
                    WebUIFragment::signal("innerItem.innerLabel", false),
                    WebUIFragment::raw("</p>"),
                ],
            },
        );
        let protocol = WebUIProtocol::new(fragments);
        let state = test_json!({
            "globalPrefix": "Prefix: ",
            "outerItems": [
                {"outerLabel": "O1", "innerItems": [{"innerLabel": "I1"}, {"innerLabel": "I2"}]},
                {"outerLabel": "O2", "innerItems": [{"innerLabel": "I3"}]}
            ]
        });
        let mut writer = TestWriter::new();
        handle(
            &protocol,
            &state,
            &RenderOptions::new("index.html", "/"),
            &mut writer,
        )
        .unwrap();
        assert_eq!(
            writer.get_content(),
            "<div><section>Prefix: O1<p>Prefix: O1: I1</p><p>Prefix: O1: I2</p></section><section>Prefix: O2<p>Prefix: O2: I3</p></section></div>"
        );
    }

    #[test]
    fn test_component_in_for_global_only() {
        let mut fragments = HashMap::new();
        fragments.insert(
            "index.html".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::raw("<div>"),
                    WebUIFragment::for_loop("item", "items", "templateComponent"),
                    WebUIFragment::raw("</div>"),
                ],
            },
        );
        fragments.insert(
            "templateComponent".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::raw("<component-tag>"),
                    WebUIFragment::component("my-component"),
                    WebUIFragment::raw("</component-tag>"),
                ],
            },
        );
        fragments.insert(
            "my-component".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::raw("<span>"),
                    WebUIFragment::signal("name", false),
                    WebUIFragment::raw("-"),
                    WebUIFragment::signal("globalSuffix", false),
                    WebUIFragment::raw("</span>"),
                ],
            },
        );
        let protocol = WebUIProtocol::new(fragments);
        let state =
            test_json!({"globalSuffix": "Global", "items": [{"name": "Item1"}, {"name": "Item2"}]});
        let mut writer = TestWriter::new();
        handle(
            &protocol,
            &state,
            &RenderOptions::new("index.html", "/"),
            &mut writer,
        )
        .unwrap();
        assert_eq!(
            writer.get_content(),
            "<div><component-tag><span>-Global</span></component-tag><component-tag><span>-Global</span></component-tag></div>"
        );
    }

    #[test]
    fn test_component_no_item_moniker() {
        let mut fragments = HashMap::new();
        fragments.insert(
            "index.html".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::raw("<div>"),
                    WebUIFragment::for_loop("item", "items", "templateComponent"),
                    WebUIFragment::raw("</div>"),
                ],
            },
        );
        fragments.insert(
            "templateComponent".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::raw("<component-tag>"),
                    WebUIFragment::component("my-component"),
                    WebUIFragment::raw("</component-tag>"),
                ],
            },
        );
        fragments.insert(
            "my-component".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::raw("<span>"),
                    WebUIFragment::signal("item.name", false),
                    WebUIFragment::raw("-"),
                    WebUIFragment::signal("globalSuffix", false),
                    WebUIFragment::raw("</span>"),
                ],
            },
        );
        let protocol = WebUIProtocol::new(fragments);
        let state =
            test_json!({"globalSuffix": "Global", "items": [{"name": "Item1"}, {"name": "Item2"}]});
        let mut writer = TestWriter::new();
        handle(
            &protocol,
            &state,
            &RenderOptions::new("index.html", "/"),
            &mut writer,
        )
        .unwrap();
        assert_eq!(
            writer.get_content(),
            "<div><component-tag><span>-Global</span></component-tag><component-tag><span>-Global</span></component-tag></div>"
        );
    }

    #[test]
    fn test_for_nonqualified_uses_global() {
        let mut fragments = HashMap::new();
        fragments.insert(
            "index.html".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::raw("<div>"),
                    WebUIFragment::for_loop("item", "items", "template1"),
                    WebUIFragment::raw("</div>"),
                ],
            },
        );
        fragments.insert(
            "template1".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::raw("<span>"),
                    WebUIFragment::signal("name", false),
                    WebUIFragment::raw("</span>"),
                ],
            },
        );
        let protocol = WebUIProtocol::new(fragments);
        let state = test_json!({"name": "GlobalName", "items": [{"name": "LocalName1"}, {"name": "LocalName2"}]});
        let mut writer = TestWriter::new();
        handle(
            &protocol,
            &state,
            &RenderOptions::new("index.html", "/"),
            &mut writer,
        )
        .unwrap();
        assert_eq!(
            writer.get_content(),
            "<div><span>GlobalName</span><span>GlobalName</span></div>"
        );
    }

    #[test]
    fn test_nested_for_if_interleaved() {
        let mut fragments = HashMap::new();
        fragments.insert(
            "index.html".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::raw("<div>"),
                    WebUIFragment::for_loop("outerItem", "outerItems", "outerTemplate"),
                    WebUIFragment::raw("</div>"),
                ],
            },
        );
        fragments.insert(
            "outerTemplate".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::raw("<section>"),
                    WebUIFragment::signal("globalPrefix", false),
                    WebUIFragment::signal("outerItem.outerLabel", false),
                    WebUIFragment::if_cond(
                        ConditionExpr::identifier("outerItem.include"),
                        "ifTemplate",
                    ),
                    WebUIFragment::raw("</section>"),
                ],
            },
        );
        fragments.insert(
            "ifTemplate".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::raw("<div>"),
                    WebUIFragment::for_loop("innerItem", "outerItem.innerItems", "innerTemplate"),
                    WebUIFragment::raw("</div>"),
                ],
            },
        );
        fragments.insert(
            "innerTemplate".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::raw("<p>"),
                    WebUIFragment::signal("globalSuffix", false),
                    WebUIFragment::raw(": "),
                    WebUIFragment::signal("innerItem.innerLabel", false),
                    WebUIFragment::raw("</p>"),
                ],
            },
        );
        let protocol = WebUIProtocol::new(fragments);
        let state = test_json!({
            "globalPrefix": "Prefix: ",
            "globalSuffix": "Suffix",
            "outerItems": [
                {"outerLabel": "O1", "include": true, "innerItems": [{"innerLabel": "I1"}, {"innerLabel": "I2"}]},
                {"outerLabel": "O2", "include": false, "innerItems": [{"innerLabel": "Iignored"}]},
                {"outerLabel": "O3", "include": true, "innerItems": [{"innerLabel": "I3"}]}
            ]
        });
        let mut writer = TestWriter::new();
        handle(
            &protocol,
            &state,
            &RenderOptions::new("index.html", "/"),
            &mut writer,
        )
        .unwrap();
        assert_eq!(
            writer.get_content(),
            "<div><section>Prefix: O1<div><p>Suffix: I1</p><p>Suffix: I2</p></div></section><section>Prefix: O2</section><section>Prefix: O3<div><p>Suffix: I3</p></div></section></div>"
        );
    }

    #[test]
    fn test_nested_for_if_outer_state() {
        let mut fragments = HashMap::new();
        fragments.insert(
            "index.html".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::raw("<div>"),
                    WebUIFragment::for_loop("outerItem", "outerItems", "outerTemplate"),
                    WebUIFragment::raw("</div>"),
                ],
            },
        );
        fragments.insert(
            "outerTemplate".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::raw("<section>"),
                    WebUIFragment::signal("globalPrefix", false),
                    WebUIFragment::signal("outerItem.label", false),
                    WebUIFragment::for_loop(
                        "middleItem",
                        "outerItem.middleItems",
                        "middleTemplate",
                    ),
                    WebUIFragment::raw("</section>"),
                ],
            },
        );
        fragments.insert(
            "middleTemplate".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::raw("<div>"),
                    WebUIFragment::if_cond(
                        ConditionExpr::identifier("outerItem.active"),
                        "ifTemplate",
                    ),
                    WebUIFragment::raw("</div>"),
                ],
            },
        );
        fragments.insert(
            "ifTemplate".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::raw("<p>"),
                    WebUIFragment::signal("middleItem.value", false),
                    WebUIFragment::raw("</p>"),
                ],
            },
        );
        let protocol = WebUIProtocol::new(fragments);
        let state = test_json!({
            "globalPrefix": "GP-",
            "outerItems": [
                {"label": "O1", "active": true, "middleItems": [{"value": "M1"}, {"value": "M2"}]},
                {"label": "O2", "active": false, "middleItems": [{"value": "M3"}]},
                {"label": "O3", "active": true, "middleItems": [{"value": "M4"}]}
            ]
        });
        let mut writer = TestWriter::new();
        handle(
            &protocol,
            &state,
            &RenderOptions::new("index.html", "/"),
            &mut writer,
        )
        .unwrap();
        assert_eq!(
            writer.get_content(),
            "<div><section>GP-O1<div><p>M1</p></div><div><p>M2</p></div></section><section>GP-O2<div></div></section><section>GP-O3<div><p>M4</p></div></section></div>"
        );
    }

    #[test]
    fn test_nested_for_if_inner_state() {
        let mut fragments = HashMap::new();
        fragments.insert(
            "index.html".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::raw("<div>"),
                    WebUIFragment::for_loop("outerItem", "outerItems", "outerTemplate"),
                    WebUIFragment::raw("</div>"),
                ],
            },
        );
        fragments.insert(
            "outerTemplate".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::raw("<section>"),
                    WebUIFragment::signal("outerItem.label", false),
                    WebUIFragment::for_loop("innerItem", "outerItem.innerItems", "innerTemplate"),
                    WebUIFragment::raw("</section>"),
                ],
            },
        );
        fragments.insert(
            "innerTemplate".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::raw("<article>"),
                    WebUIFragment::if_cond(
                        ConditionExpr::identifier("innerItem.show"),
                        "ifTemplate",
                    ),
                    WebUIFragment::raw("</article>"),
                ],
            },
        );
        fragments.insert(
            "ifTemplate".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::raw("<p>"),
                    WebUIFragment::signal("innerItem.detail", false),
                    WebUIFragment::raw("</p>"),
                ],
            },
        );
        let protocol = WebUIProtocol::new(fragments);
        let state = test_json!({
            "outerItems": [
                {"label": "Outer1", "innerItems": [{"detail": "Detail1", "show": true}, {"detail": "Detail2", "show": false}]},
                {"label": "Outer2", "innerItems": [{"detail": "Detail3", "show": true}]}
            ]
        });
        let mut writer = TestWriter::new();
        handle(
            &protocol,
            &state,
            &RenderOptions::new("index.html", "/"),
            &mut writer,
        )
        .unwrap();
        assert_eq!(
            writer.get_content(),
            "<div><section>Outer1<article><p>Detail1</p></article><article></article></section><section>Outer2<article><p>Detail3</p></article></section></div>"
        );
    }

    #[test]
    fn test_for_merge_local_global_monikers() {
        let mut fragments = HashMap::new();
        fragments.insert(
            "index.html".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::raw("<div>"),
                    WebUIFragment::for_loop("item", "items", "template1"),
                    WebUIFragment::raw("</div>"),
                ],
            },
        );
        fragments.insert(
            "template1".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::raw("<span>"),
                    WebUIFragment::signal("item.name", false),
                    WebUIFragment::raw("-"),
                    WebUIFragment::signal("item.globalValue", false),
                    WebUIFragment::raw("-"),
                    WebUIFragment::signal("item.localOnly", false),
                    WebUIFragment::raw("-"),
                    WebUIFragment::signal("item.otherVal", false),
                    WebUIFragment::raw("</span>"),
                ],
            },
        );
        let protocol = WebUIProtocol::new(fragments);
        let state = test_json!({
            "item": {"globalValue": "GLOBAL", "otherVal": "other"},
            "items": [
                {"name": "Local1", "globalValue": "LOCAL", "localOnly": "Only1"},
                {"name": "Local2", "localOnly": "Only2"}
            ]
        });
        let mut writer = TestWriter::new();
        handle(
            &protocol,
            &state,
            &RenderOptions::new("index.html", "/"),
            &mut writer,
        )
        .unwrap();
        assert_eq!(
            writer.get_content(),
            "<div><span>Local1-LOCAL-Only1-other</span><span>Local2-GLOBAL-Only2-other</span></div>"
        );
    }

    #[test]
    fn test_component_in_for_global_moniker_shadow() {
        let mut fragments = HashMap::new();
        fragments.insert(
            "index.html".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::raw("<div>"),
                    WebUIFragment::for_loop("item", "items", "templateComponent"),
                    WebUIFragment::raw("</div>"),
                ],
            },
        );
        fragments.insert(
            "templateComponent".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::raw("<component-tag>"),
                    WebUIFragment::component("my-component"),
                    WebUIFragment::raw("</component-tag>"),
                ],
            },
        );
        fragments.insert(
            "my-component".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::raw("<span>"),
                    WebUIFragment::signal("name", false),
                    WebUIFragment::raw("-"),
                    WebUIFragment::signal("item.globalValue", false),
                    WebUIFragment::raw("-"),
                    WebUIFragment::signal("localOnly", false),
                    WebUIFragment::raw("-"),
                    WebUIFragment::signal("item.otherVal", false),
                    WebUIFragment::raw("</span>"),
                ],
            },
        );
        let protocol = WebUIProtocol::new(fragments);
        let state = test_json!({
            "item": {"globalValue": "GLOBAL", "otherVal": "other"},
            "items": [
                {"name": "Local1", "globalValue": "LOCAL", "localOnly": "Only1"},
                {"name": "Local2", "localOnly": "Only2"}
            ]
        });
        let mut writer = TestWriter::new();
        handle(
            &protocol,
            &state,
            &RenderOptions::new("index.html", "/"),
            &mut writer,
        )
        .unwrap();
        assert_eq!(
            writer.get_content(),
            "<div><component-tag><span>-GLOBAL--other</span></component-tag><component-tag><span>-GLOBAL--other</span></component-tag></div>"
        );
    }

    #[test]
    fn test_if_in_nested_for_local_flag() {
        let mut fragments = HashMap::new();
        fragments.insert(
            "index.html".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::raw("<div>"),
                    WebUIFragment::for_loop("outer", "list.outer_items", "outerTemplate"),
                    WebUIFragment::raw("</div>"),
                ],
            },
        );
        fragments.insert(
            "outerTemplate".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::raw("<section>"),
                    WebUIFragment::for_loop("inner_item", "outer.inner_items", "innerTemplate"),
                    WebUIFragment::raw("</section>"),
                ],
            },
        );
        fragments.insert(
            "innerTemplate".to_string(),
            FragmentList {
                fragments: vec![WebUIFragment::if_cond(
                    ConditionExpr::identifier("inner_item.flag"),
                    "ifInner",
                )],
            },
        );
        fragments.insert(
            "ifInner".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::raw("<p>"),
                    WebUIFragment::signal("inner_item.value", false),
                    WebUIFragment::raw("</p>"),
                ],
            },
        );
        let protocol = WebUIProtocol::new(fragments);
        let state = test_json!({
            "list": {"outer_items": [{"inner_items": [{"flag": true, "value": "X"}, {"flag": false, "value": "Y"}]}]},
            "inner_item": {"flag": false}
        });
        let mut writer = TestWriter::new();
        handle(
            &protocol,
            &state,
            &RenderOptions::new("index.html", "/"),
            &mut writer,
        )
        .unwrap();
        assert_eq!(
            writer.get_content(),
            "<div><section><p>X</p></section></div>"
        );
    }

    #[test]
    fn test_if_in_nested_for_global_fallback() {
        let mut fragments = HashMap::new();
        fragments.insert(
            "index.html".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::raw("<div>"),
                    WebUIFragment::for_loop("outer", "list.outer_items", "outerTemplate"),
                    WebUIFragment::raw("</div>"),
                ],
            },
        );
        fragments.insert(
            "outerTemplate".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::raw("<section>"),
                    WebUIFragment::for_loop("inner_item", "outer.inner_items", "innerTemplate"),
                    WebUIFragment::raw("</section>"),
                ],
            },
        );
        fragments.insert(
            "innerTemplate".to_string(),
            FragmentList {
                fragments: vec![WebUIFragment::if_cond(
                    ConditionExpr::identifier("inner_item.flag"),
                    "ifInner",
                )],
            },
        );
        fragments.insert(
            "ifInner".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::raw("<p>"),
                    WebUIFragment::signal("inner_item.value", false),
                    WebUIFragment::raw("</p>"),
                ],
            },
        );
        let protocol = WebUIProtocol::new(fragments);
        let state = test_json!({
            "list": {"outer_items": [{"inner_items": [{"value": "X"}, {"value": "Y"}]}]},
            "inner_item": {"flag": true}
        });
        let mut writer = TestWriter::new();
        handle(
            &protocol,
            &state,
            &RenderOptions::new("index.html", "/"),
            &mut writer,
        )
        .unwrap();
        assert_eq!(
            writer.get_content(),
            "<div><section><p>X</p><p>Y</p></section></div>"
        );
    }

    #[test]
    fn test_if_mixed_for_monikers() {
        let mut fragments = HashMap::new();
        fragments.insert(
            "index.html".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::raw("<div>"),
                    WebUIFragment::for_loop("outer", "list.outerItems", "outerTemplate"),
                    WebUIFragment::raw("</div>"),
                ],
            },
        );
        fragments.insert(
            "outerTemplate".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::raw("<section>"),
                    WebUIFragment::signal("outer.outerLabel", false),
                    WebUIFragment::for_loop("inner", "outer.innerItems", "innerTemplate"),
                    WebUIFragment::raw("</section>"),
                ],
            },
        );
        fragments.insert(
            "innerTemplate".to_string(),
            FragmentList {
                fragments: vec![WebUIFragment::if_cond(
                    ConditionExpr::compound(
                        ConditionExpr::identifier("outer.active"),
                        LogicalOperator::And,
                        ConditionExpr::predicate(
                            "inner.value",
                            ComparisonOperator::GreaterThan,
                            "globalLimit",
                        ),
                    ),
                    "ifInner",
                )],
            },
        );
        fragments.insert(
            "ifInner".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::raw("<p>"),
                    WebUIFragment::signal("inner.value", false),
                    WebUIFragment::raw("</p>"),
                ],
            },
        );
        let protocol = WebUIProtocol::new(fragments);
        let state = test_json!({
            "globalLimit": 10,
            "list": {"outerItems": [
                {"outerLabel": "O1", "active": true, "innerItems": [{"value": 15}, {"value": 8}]},
                {"outerLabel": "O2", "active": false, "innerItems": [{"value": 20}]},
                {"outerLabel": "O3", "active": true, "innerItems": [{"value": 5}]}
            ]}
        });
        let mut writer = TestWriter::new();
        handle(
            &protocol,
            &state,
            &RenderOptions::new("index.html", "/"),
            &mut writer,
        )
        .unwrap();
        assert_eq!(
            writer.get_content(),
            "<div><section>O1<p>15</p></section><section>O2</section><section>O3</section></div>"
        );
    }

    // ── Route-aware rendering tests ─────────────────────────────────────

    fn make_route_protocol() -> WebUIProtocol {
        use webui_protocol::WebUiFragmentRoute;

        let mut fragments = HashMap::new();

        // Entry page with two routes
        fragments.insert(
            "index.html".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::raw("<h1>Shell</h1>"),
                    WebUIFragment::route_from(WebUiFragmentRoute {
                        path: "/".into(),
                        fragment_id: "dash-page".into(),
                        exact: true,
                        keep_alive: false,
                        ..Default::default()
                    }),
                    WebUIFragment::route_from(WebUiFragmentRoute {
                        path: "/contacts/:id".into(),
                        fragment_id: "detail-page".into(),
                        exact: true,
                        keep_alive: true,
                        ..Default::default()
                    }),
                ],
            },
        );

        // Dashboard page component
        fragments.insert(
            "dash-page".to_string(),
            FragmentList {
                fragments: vec![WebUIFragment::raw("<p>Dashboard</p>")],
            },
        );

        // Detail page component
        fragments.insert(
            "detail-page".to_string(),
            FragmentList {
                fragments: vec![WebUIFragment::raw("<p>Detail</p>")],
            },
        );

        WebUIProtocol::new(fragments)
    }

    fn make_nested_route_protocol() -> WebUIProtocol {
        use webui_protocol::WebUiFragmentRoute;

        let mut fragments = HashMap::new();

        fragments.insert(
            "index.html".to_string(),
            FragmentList {
                fragments: vec![WebUIFragment::route_from(WebUiFragmentRoute {
                    path: "/".into(),
                    fragment_id: "app-shell".into(),
                    exact: false,
                    children: vec![WebUiFragmentRoute {
                        path: "sections/:id".into(),
                        fragment_id: "section-comp".into(),
                        exact: false,
                        children: vec![WebUiFragmentRoute {
                            path: "topics/:topicId".into(),
                            fragment_id: "topic-comp".into(),
                            exact: true,
                            children: vec![],
                            keep_alive: false,
                            ..Default::default()
                        }],
                        keep_alive: false,
                        ..Default::default()
                    }],
                    keep_alive: false,
                    ..Default::default()
                })],
            },
        );

        fragments.insert(
            "app-shell".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::raw("<h1>Shell</h1>"),
                    WebUIFragment::outlet(),
                ],
            },
        );

        fragments.insert(
            "section-comp".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::raw("<h2>Section</h2>"),
                    WebUIFragment::outlet(),
                ],
            },
        );

        fragments.insert(
            "topic-comp".to_string(),
            FragmentList {
                fragments: vec![WebUIFragment::raw("<p>Topic content</p>")],
            },
        );

        WebUIProtocol::new(fragments)
    }

    #[test]
    fn test_route_renders_shell_always() {
        let protocol = make_route_protocol();
        let state = test_json!({});
        let mut writer = TestWriter::new();
        handle(
            &protocol,
            &state,
            &RenderOptions::new("index.html", "/"),
            &mut writer,
        )
        .unwrap();
        let html = writer.get_content();

        // Shell content always renders regardless of route matching
        assert!(html.contains("<h1>Shell</h1>"), "shell should render");
        // Dashboard matches "/" so it should be active
        assert!(html.contains(" active>"), "matched route should be active");
        // Detail should be hidden and empty
        assert!(
            html.contains("style=\"display:none\""),
            "non-matched routes should be hidden"
        );
    }

    #[test]
    fn test_route_matched_renders_visible() {
        let protocol = make_route_protocol();
        let state = test_json!({});
        let mut writer = TestWriter::new();
        handle(
            &protocol,
            &state,
            &RenderOptions::new("index.html", "/"),
            &mut writer,
        )
        .unwrap();
        let html = writer.get_content();

        // Dashboard route should be visible (active, no display:none)
        assert!(
            html.contains("<webui-route path=\"/\""),
            "dashboard route should exist"
        );
        assert!(
            html.contains("active>") && html.contains("<dash-page"),
            "matched route should be active with component tag: {html}"
        );
        assert!(
            html.contains("<p>Dashboard</p>"),
            "matched route should have content"
        );
    }

    #[test]
    fn test_route_non_matched_renders_hidden_empty() {
        let protocol = make_route_protocol();
        let state = test_json!({});
        let mut writer = TestWriter::new();
        handle(
            &protocol,
            &state,
            &RenderOptions::new("index.html", "/"),
            &mut writer,
        )
        .unwrap();
        let html = writer.get_content();

        // Detail route should be hidden and empty (no content rendered)
        assert!(
            html.contains("<webui-route path=\"/contacts/:id\""),
            "detail route element should exist"
        );
        // The non-matched route should have display:none and no inner content
        let detail_start = html.find("path=\"/contacts/:id\"").expect("detail route");
        let after_detail = &html[detail_start..];
        assert!(
            after_detail.contains("style=\"display:none\">")
                && !after_detail.starts_with(&format!("path=\"/contacts/:id\"{}detail-page>", "")),
            "non-matched route should be hidden: {after_detail}"
        );
        // Should NOT contain the component's rendered content
        let detail_end = after_detail.find("</webui-route>").expect("closing tag");
        let detail_body = &after_detail[..detail_end];
        assert!(
            !detail_body.contains("<detail-page"),
            "non-matched route should not render component content: {detail_body}"
        );
    }

    #[test]
    fn test_route_parameterized_match() {
        let protocol = make_route_protocol();
        let state = test_json!({});
        let mut writer = TestWriter::new();
        handle(
            &protocol,
            &state,
            &RenderOptions::new("index.html", "/contacts/42"),
            &mut writer,
        )
        .unwrap();
        let html = writer.get_content();

        // Detail route matches /contacts/42
        assert!(
            html.contains("active>") && html.contains("<detail-page"),
            "detail route should be active: {html}"
        );
        assert!(html.contains("<p>Detail</p>"), "detail should have content");
        // Dashboard should be hidden + empty
        let dash_start = html
            .find("component=\"dash-page\"")
            .expect("dashboard route");
        let after_dash = &html[dash_start..];
        assert!(
            after_dash.contains("style=\"display:none\">"),
            "dashboard should be hidden when detail matches: {after_dash}"
        );
        let dash_end = after_dash.find("</webui-route>").expect("closing tag");
        let dash_body = &after_dash[..dash_end];
        assert!(
            !dash_body.contains("<dash-page"),
            "dashboard should not render component content: {dash_body}"
        );
    }

    #[test]
    fn test_route_no_match_all_hidden_empty() {
        let protocol = make_route_protocol();
        let state = test_json!({});
        let mut writer = TestWriter::new();
        handle(
            &protocol,
            &state,
            &RenderOptions::new("index.html", "/nonexistent"),
            &mut writer,
        )
        .unwrap();
        let html = writer.get_content();

        // Shell content should still render
        assert!(html.contains("<h1>Shell</h1>"));
        // All routes should be hidden + empty (nothing matched)
        assert!(
            !html.contains("<p>Dashboard</p>"),
            "no route content when nothing matches"
        );
        assert!(
            !html.contains("<p>Detail</p>"),
            "no route content when nothing matches"
        );
    }

    #[test]
    fn test_route_component_attr_emitted() {
        let protocol = make_route_protocol();
        let state = test_json!({});
        let mut writer = TestWriter::new();
        handle(
            &protocol,
            &state,
            &RenderOptions::new("index.html", "/"),
            &mut writer,
        )
        .unwrap();
        let html = writer.get_content();
        // component attribute should be emitted on webui-route
        assert!(
            html.contains("component=\"dash-page\""),
            "component attr should be on webui-route: {html}"
        );
        assert!(
            html.contains("component=\"detail-page\""),
            "component attr should be on webui-route: {html}"
        );
        assert!(
            html.contains(r#"component="detail-page" exact keep-alive style="display:none">"#),
            "keep-alive should be emitted on an unmatched destination placeholder: {html}"
        );
    }

    #[test]
    fn test_no_plugin_no_state_attributes() {
        let protocol = make_route_protocol();
        let state = test_json!({
            "title": "Fish & Chips",
            "cartOpen": true,
            "items": [{"name": "A&B"}]
        });
        let mut writer = TestWriter::new();

        handle(
            &protocol,
            &state,
            &RenderOptions::new("index.html", "/"),
            &mut writer,
        )
        .unwrap();

        let html = writer.get_content();
        // Without a plugin, no state attributes at all
        assert!(
            !html.contains("data-state"),
            "no data-state without plugin: {html}"
        );
        assert!(
            !html.contains(r#"title="Fish"#),
            "no scalar attrs without plugin: {html}"
        );
    }

    #[test]
    fn test_nested_routes_render_webui_route_as_light_dom() {
        let protocol = make_nested_route_protocol();
        let state = test_json!({"title": "Test"});
        let handler = WebUIHandler::new();
        let mut writer = TestWriter::new();

        handler
            .handle(
                &protocol,
                &state,
                &RenderOptions::new("index.html", "/sections/frontend"),
                &mut writer,
            )
            .expect("render failed");

        let html = writer.get_content();

        assert!(
            html.contains("component=\"app-shell\"") && html.contains("active>"),
            "root route should be active: {html}"
        );
        // webui-route should NOT have shadow DOM — it's a light DOM structural element
        assert!(
            !html.contains("<template shadowrootmode"),
            "webui-route should be light DOM (no shadow template): {html}"
        );
    }

    #[test]
    fn test_nested_routes_render_outlet_as_light_dom() {
        let protocol = make_nested_route_protocol();
        let state = test_json!({"title": "Test"});
        let handler = WebUIHandler::new();
        let mut writer = TestWriter::new();

        handler
            .handle(
                &protocol,
                &state,
                &RenderOptions::new("index.html", "/sections/frontend"),
                &mut writer,
            )
            .expect("render failed");

        let html = writer.get_content();

        // No <webui-outlet> wrapper — routes render directly at outlet position
        assert!(
            !html.contains("<webui-outlet>"),
            "should not contain webui-outlet wrapper: {html}"
        );
        // Route elements should be in the output directly
        assert!(
            html.contains("<webui-route"),
            "should contain webui-route elements: {html}"
        );
    }

    #[test]
    fn test_nested_routes_match_child_at_outlet() {
        let protocol = make_nested_route_protocol();
        let state = test_json!({"title": "Test"});
        let handler = WebUIHandler::new();
        let mut writer = TestWriter::new();

        handler
            .handle(
                &protocol,
                &state,
                &RenderOptions::new("index.html", "/sections/frontend"),
                &mut writer,
            )
            .expect("render failed");

        let html = writer.get_content();

        assert!(
            html.contains("component=\"section-comp\"") && html.contains("active>"),
            "section route should be active: {html}"
        );
        assert!(
            html.contains("<h2>Section</h2>"),
            "section content should be present: {html}"
        );
    }

    #[test]
    fn test_nested_routes_three_levels_deep() {
        let protocol = make_nested_route_protocol();
        let state = test_json!({"title": "Test"});
        let handler = WebUIHandler::new();
        let mut writer = TestWriter::new();

        handler
            .handle(
                &protocol,
                &state,
                &RenderOptions::new("index.html", "/sections/frontend/topics/react"),
                &mut writer,
            )
            .expect("render failed");

        let html = writer.get_content();

        assert!(
            html.contains("component=\"app-shell\"") && html.contains("active>"),
            "root active: {html}"
        );
        assert!(
            html.contains("component=\"section-comp\"") && html.contains("active>"),
            "section active: {html}"
        );
        assert!(
            html.contains("component=\"topic-comp\"")
                && html.contains("exact")
                && html.contains("active>"),
            "topic active: {html}"
        );
        assert!(
            html.contains("<p>Topic content</p>"),
            "leaf content present: {html}"
        );
    }

    #[test]
    fn test_nested_routes_nonmatched_siblings_hidden() {
        let protocol = make_nested_route_protocol();
        let state = test_json!({"title": "Test"});
        let handler = WebUIHandler::new();
        let mut writer = TestWriter::new();

        handler
            .handle(
                &protocol,
                &state,
                &RenderOptions::new("index.html", "/sections/frontend"),
                &mut writer,
            )
            .expect("render failed");

        let html = writer.get_content();

        assert!(
            html.contains(r#"component="topic-comp" exact style="display:none">"#),
            "topic should be hidden: {html}"
        );
    }

    #[test]
    fn test_nested_routes_root_only() {
        let protocol = make_nested_route_protocol();
        let state = test_json!({"title": "Test"});
        let handler = WebUIHandler::new();
        let mut writer = TestWriter::new();

        handler
            .handle(
                &protocol,
                &state,
                &RenderOptions::new("index.html", "/"),
                &mut writer,
            )
            .expect("render failed");

        let html = writer.get_content();

        assert!(
            html.contains("component=\"app-shell\"") && html.contains("active>"),
            "root active at /: {html}"
        );
        assert!(
            html.contains("<h1>Shell</h1>"),
            "shell renders at /: {html}"
        );
        assert!(
            html.contains(r#"component="section-comp" style="display:none">"#),
            "section hidden at /: {html}"
        );
    }

    // ── CSS Module dedup tests ───────────────────────────────────────

    #[test]
    fn test_css_module_emitted_once_inline_in_component() {
        // CSS module definition emitted once in the component's light DOM
        // on first render, not in <head> and not on second instance.
        let template = r#"<p><slot></slot></p>"#;

        let mut fragments = HashMap::new();
        fragments.insert(
            "index.html".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::raw("<html><head>".to_string()),
                    structural_fragment("head_end"),
                    WebUIFragment::raw("</head><body><div>".to_string()),
                    WebUIFragment::component("my-card"),
                    WebUIFragment::raw("A".to_string()),
                    WebUIFragment::component("my-card"),
                    WebUIFragment::raw("B</div>".to_string()),
                    structural_fragment("body_end"),
                    WebUIFragment::raw("</body></html>".to_string()),
                ],
            },
        );
        fragments.insert(
            "my-card".to_string(),
            FragmentList {
                fragments: vec![WebUIFragment::raw(template.to_string())],
            },
        );

        let mut protocol = WebUIProtocol::new(fragments);
        protocol.set_css_strategy(webui_protocol::CssStrategy::Module);
        protocol
            .components
            .entry("my-card".to_string())
            .or_default()
            .css = "p{color:red}".to_string();
        protocol.populate_style_closures(&["index.html"]);
        let state = test_json!({});
        let mut writer = TestWriter::new();

        handle(
            &protocol,
            &state,
            &RenderOptions::new("index.html", "/"),
            &mut writer,
        )
        .unwrap();

        let html = writer.get_content();

        // CSS module importmap should appear exactly once
        let count = html.matches(r#"<script type="importmap""#).count();
        assert_eq!(
            count, 1,
            "CSS module importmap should be emitted once, got {count} in: {html}"
        );
        assert!(
            html.contains(r#""my-card":"data:text/css,"#),
            "Importmap must register my-card under a data: URI: {html}"
        );

        // Template content should appear twice (once per component instance)
        let tmpl_count = html.matches(r#"<p><slot></slot></p>"#).count();
        assert_eq!(
            tmpl_count, 2,
            "Template should render twice, got {tmpl_count} in: {html}"
        );

        // CSS module should be in <body> (inline), not in <head>
        let css_pos = html
            .find(r#"<script type="importmap""#)
            .expect("CSS module importmap missing");
        let body_pos = html.find("<body>").expect("<body> missing");
        assert!(
            css_pos > body_pos,
            "CSS module should be inline in component, not in <head>: {html}"
        );
    }

    #[test]
    fn test_component_without_css_renders_normally() {
        // Components without CSS module prefix pass through unchanged
        let mut fragments = HashMap::new();
        fragments.insert(
            "index.html".to_string(),
            FragmentList {
                fragments: vec![WebUIFragment::component("my-card")],
            },
        );
        fragments.insert(
            "my-card".to_string(),
            FragmentList {
                fragments: vec![WebUIFragment::raw(r#"<p>hello</p>"#.to_string())],
            },
        );

        let protocol = WebUIProtocol::new(fragments);
        let state = test_json!({});
        let mut writer = TestWriter::new();

        handle(
            &protocol,
            &state,
            &RenderOptions::new("index.html", "/"),
            &mut writer,
        )
        .unwrap();

        let html = writer.get_content();
        assert!(
            html.contains("<p>hello</p>"),
            "Non-module component should render normally: {html}"
        );
    }

    #[test]
    fn test_non_module_strategy_no_css_in_head() {
        // When component_css is empty (Link/Style strategies), no
        // CSS module importmap tags should appear in <head>.
        let template = r#"<p>hello</p>"#;

        let mut fragments = HashMap::new();
        fragments.insert(
            "index.html".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::raw("<html><head>".to_string()),
                    structural_fragment("head_end"),
                    WebUIFragment::raw("</head><body>".to_string()),
                    WebUIFragment::component("my-card"),
                    WebUIFragment::raw("</body></html>".to_string()),
                ],
            },
        );
        fragments.insert(
            "my-card".to_string(),
            FragmentList {
                fragments: vec![WebUIFragment::raw(template.to_string())],
            },
        );

        // No component css populated — simulates Link/Style strategy
        let protocol = WebUIProtocol::new(fragments);
        let state = test_json!({});
        let mut writer = TestWriter::new();

        handle(
            &protocol,
            &state,
            &RenderOptions::new("index.html", "/"),
            &mut writer,
        )
        .unwrap();

        let html = writer.get_content();

        assert!(
            !html.contains(r#"<style type="module""#),
            "Non-module strategy should not emit CSS module tags in <head>: {html}"
        );
        assert!(
            !html.contains(r#"<script type="importmap""#),
            "Non-module strategy should not emit CSS module importmaps in <head>: {html}"
        );
        assert!(
            html.contains("<p>hello</p>"),
            "Component should still render: {html}"
        );
    }

    #[test]
    fn test_style_strategy_embeds_inline_style_in_shadow_template() {
        let mut fragments = HashMap::new();
        fragments.insert(
            "index.html".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::raw("<html><head>".to_string()),
                    structural_fragment("head_end"),
                    WebUIFragment::raw("</head><body><my-card>".to_string()),
                    WebUIFragment::component("my-card"),
                    WebUIFragment::raw("</my-card></body></html>".to_string()),
                ],
            },
        );
        fragments.insert(
            "my-card".to_string(),
            FragmentList {
                fragments: vec![WebUIFragment::raw(
                    "<template shadowrootmode=\"open\"><style>.card{color:red}</style><div>card</div></template>"
                        .to_string(),
                )],
            },
        );

        let protocol = WebUIProtocol::new(fragments);
        let state = test_json!({});
        let mut writer = TestWriter::new();

        handle(
            &protocol,
            &state,
            &RenderOptions::new("index.html", "/"),
            &mut writer,
        )
        .unwrap();

        let html = writer.get_content();

        assert!(
            html.contains("<style>.card{color:red}</style>"),
            "Style strategy should embed inline CSS in shadow template: {html}"
        );
        assert!(
            !html.contains(r#"<style type="module""#),
            "Style strategy should not emit legacy module CSS in <head>: {html}"
        );
        assert!(
            !html.contains(r#"<script type="importmap""#),
            "Style strategy should not emit CSS module importmaps in <head>: {html}"
        );
    }

    #[test]
    fn test_link_strategy_light_dom_emits_stylesheet_in_head() {
        // Light DOM + Link strategy: handler emits <link rel="stylesheet">
        // in <head>. No preload tag — the stylesheet itself fetches.
        let mut fragments = HashMap::new();
        fragments.insert(
            "index.html".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::raw("<html><head>".to_string()),
                    structural_fragment("head_end"),
                    WebUIFragment::raw("</head><body><my-card>".to_string()),
                    WebUIFragment::component("my-card"),
                    WebUIFragment::raw("</my-card>".to_string()),
                    structural_fragment("body_end"),
                    WebUIFragment::raw("</body></html>".to_string()),
                ],
            },
        );
        fragments.insert(
            "my-card".to_string(),
            FragmentList {
                fragments: vec![WebUIFragment::raw("<div>card</div>".to_string())],
            },
        );

        let mut protocol = WebUIProtocol::new(fragments);
        protocol.set_css_strategy(webui_protocol::CssStrategy::Link);

        let comp = protocol
            .components
            .entry("my-card".to_string())
            .or_default();
        comp.css_href = "my-card.css".to_string();
        comp.template_json = r#"{"h":"<div>card</div>"}"#.to_string();
        protocol.populate_style_closures(&["index.html"]);

        let state = test_json!({});
        let mut writer = TestWriter::new();

        handle(
            &protocol,
            &state,
            &RenderOptions::new("index.html", "/"),
            &mut writer,
        )
        .unwrap();

        let html = writer.get_content();

        let head_end = html.find("</head>").expect("</head> missing");
        let link_pos = html.find(r#"<link rel="stylesheet" href="my-card.css""#);
        assert!(
            link_pos.is_some_and(|p| p < head_end),
            "Light DOM Link strategy should emit <link rel=stylesheet> in <head>: {html}"
        );
        assert!(
            !html.contains(r#"<link rel="preload""#),
            "Light DOM Link strategy should NOT emit preload (stylesheet already fetches): {html}"
        );
        assert!(
            !html.contains(r#"<style type="module""#),
            "Link strategy should not emit legacy module CSS: {html}"
        );
        assert!(
            !html.contains(r#"<script type="importmap""#),
            "Link strategy should not emit CSS module importmaps: {html}"
        );
    }

    #[test]
    fn link_strategy_preloads_static_shadow_css_in_head() {
        let mut fragments = HashMap::new();
        fragments.insert(
            "index.html".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::raw("<html><head>".to_string()),
                    structural_fragment("head_end"),
                    WebUIFragment::raw("</head><body><my-card>".to_string()),
                    WebUIFragment::component("my-card"),
                    WebUIFragment::raw("</my-card></body></html>".to_string()),
                ],
            },
        );
        fragments.insert(
            "my-card".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::raw("<template shadowrootmode=\"open\">".to_string()),
                    structural_fragment("shadow_styles:my-card"),
                    WebUIFragment::raw("<div>card</div></template>".to_string()),
                ],
            },
        );

        let mut protocol = WebUIProtocol::new(fragments);
        protocol.set_css_strategy(webui_protocol::CssStrategy::Link);
        let component = protocol
            .components
            .entry("my-card".to_string())
            .or_default();
        component.css_href = "my-card.css".to_string();
        component.template_json = r#"{"h":"<div>card</div>"}"#.to_string();
        component.uses_shadow_dom = true;
        protocol.populate_style_closures(&["index.html"]);

        let mut writer = TestWriter::new();
        handle(
            &protocol,
            &test_json!({}),
            &RenderOptions::new("index.html", "/"),
            &mut writer,
        )
        .unwrap();

        let html = writer.get_content();
        let head_end = html.find("</head>").expect("</head> missing");
        let preload = html
            .find(r#"<link rel="preload" as="style" href="my-card.css">"#)
            .expect("Shadow preload missing");
        let stylesheet = html
            .find(r#"<link rel="stylesheet" href="my-card.css""#)
            .expect("Shadow stylesheet missing");
        assert!(preload < head_end);
        assert!(stylesheet > head_end);
        assert_eq!(html.matches(r#"rel="preload" as="style""#).count(), 1);
    }

    #[test]
    fn tree_local_styles_deduplicate_per_root_and_resume_caller_frame() {
        let fragments = HashMap::from([
            (
                "index.html".to_string(),
                FragmentList {
                    fragments: vec![
                        WebUIFragment::raw("<html><head>".to_string()),
                        structural_fragment("head_end"),
                        WebUIFragment::raw("</head><body><outer-box>".to_string()),
                        WebUIFragment::component("outer-box"),
                        WebUIFragment::raw("</outer-box></body></html>".to_string()),
                    ],
                },
            ),
            (
                "outer-box".to_string(),
                FragmentList {
                    fragments: vec![
                        WebUIFragment::raw("<template shadowrootmode=\"open\">".to_string()),
                        structural_fragment("shadow_styles:outer-box"),
                        WebUIFragment::raw("<light-card>".to_string()),
                        WebUIFragment::component("light-card"),
                        WebUIFragment::raw("</light-card><light-card>".to_string()),
                        WebUIFragment::component("light-card"),
                        WebUIFragment::raw("</light-card><inner-box>".to_string()),
                        WebUIFragment::component("inner-box"),
                        WebUIFragment::raw("</inner-box><light-card>".to_string()),
                        WebUIFragment::component("light-card"),
                        WebUIFragment::raw("</light-card></template>".to_string()),
                    ],
                },
            ),
            (
                "inner-box".to_string(),
                FragmentList {
                    fragments: vec![
                        WebUIFragment::raw("<template shadowrootmode=\"open\">".to_string()),
                        structural_fragment("shadow_styles:inner-box"),
                        WebUIFragment::raw("<light-card>".to_string()),
                        WebUIFragment::component("light-card"),
                        WebUIFragment::raw("</light-card></template>".to_string()),
                    ],
                },
            ),
            (
                "light-card".to_string(),
                FragmentList {
                    fragments: vec![WebUIFragment::raw("<p>card</p>".to_string())],
                },
            ),
        ]);
        let mut protocol = WebUIProtocol::new(fragments);
        protocol.set_css_strategy(webui_protocol::CssStrategy::Style);
        for (tag, css, uses_shadow_dom) in [
            ("outer-box", ".outer{}", true),
            ("inner-box", ".inner{}", true),
            ("light-card", ".card{}", false),
        ] {
            let component = protocol.components.entry(tag.to_string()).or_default();
            component.css = css.to_string();
            component.uses_shadow_dom = uses_shadow_dom;
        }
        protocol.populate_style_closures(&["index.html"]);

        let mut writer = TestWriter::new();
        handle(
            &protocol,
            &test_json!({}),
            &RenderOptions::new("index.html", "/").with_nonce("css-nonce"),
            &mut writer,
        )
        .unwrap();
        let html = writer.get_content();

        assert_eq!(html.matches("data-webui-resource=\"outer-box\"").count(), 1);
        assert_eq!(html.matches("data-webui-resource=\"inner-box\"").count(), 1);
        assert_eq!(
            html.matches("data-webui-resource=\"light-card\"").count(),
            2,
            "the Light resource installs once in each Shadow tree: {html}"
        );
        assert_eq!(
            html.matches("data-webui-resource=").count(),
            4,
            "outer-box + light-card and inner-box + light-card are the four tree-local installs"
        );
        assert_eq!(
            html.matches("nonce=\"css-nonce\"").count(),
            4,
            "each inline style resource carries the nonce"
        );
        assert!(html.contains(r#"<meta name="webui-nonce" content="css-nonce">"#));
        let inner_end = html.find("</inner-box>").expect("inner host close");
        let caller_continuation = &html[inner_end..];
        let trailing_card = caller_continuation
            .find("<p>card</p>")
            .expect("caller continues after nested Shadow root");
        assert!(trailing_card > 0);
        assert!(
            !caller_continuation.contains("data-webui-resource=\"light-card\""),
            "returning from the nested Shadow root must restore the outer frame's delivered set"
        );
    }

    #[test]
    fn routed_document_styles_are_hoisted_for_the_active_chain() {
        let route = WebUiFragmentRoute {
            path: "/".to_string(),
            fragment_id: "app-shell".to_string(),
            children: vec![
                WebUiFragmentRoute {
                    path: String::new(),
                    fragment_id: "dashboard-page".to_string(),
                    exact: true,
                    ..Default::default()
                },
                WebUiFragmentRoute {
                    path: "details".to_string(),
                    fragment_id: "detail-page".to_string(),
                    exact: true,
                    ..Default::default()
                },
            ],
            ..Default::default()
        };
        let fragments = HashMap::from([
            (
                "index.html".to_string(),
                FragmentList {
                    fragments: vec![
                        WebUIFragment::raw("<html><head>"),
                        structural_fragment("head_start"),
                        structural_fragment("head_end"),
                        WebUIFragment::raw("</head><body>"),
                        structural_fragment("body_start"),
                        structural_fragment("boundary_start:0"),
                        WebUIFragment::route_from(route),
                        structural_fragment("boundary_end:0"),
                        structural_fragment("body_end"),
                        WebUIFragment::raw("</body></html>"),
                    ],
                },
            ),
            (
                "app-shell".to_string(),
                FragmentList {
                    fragments: vec![
                        WebUIFragment::raw("<header>App</header>"),
                        WebUIFragment::outlet(),
                    ],
                },
            ),
            (
                "dashboard-page".to_string(),
                FragmentList {
                    fragments: vec![WebUIFragment::raw("<main>Dashboard</main>")],
                },
            ),
            (
                "detail-page".to_string(),
                FragmentList {
                    fragments: vec![WebUIFragment::raw("<main>Detail</main>")],
                },
            ),
        ]);
        let mut protocol = WebUIProtocol::new(fragments);
        protocol.set_css_strategy(webui_protocol::CssStrategy::Link);
        for tag in ["app-shell", "dashboard-page", "detail-page"] {
            let component = protocol.components.entry(tag.to_string()).or_default();
            component.css_href = format!("/{tag}.css");
            component.template_json = r#"{"h":""}"#.to_string();
        }
        protocol.populate_style_closures(&["index.html"]);

        let mut writer = TestWriter::new();
        handle(
            &protocol,
            &test_json!({}),
            &RenderOptions::new("index.html", "/"),
            &mut writer,
        )
        .unwrap();
        let html = writer.get_content();

        assert_eq!(
            html.matches(r#"data-webui-resource="app-shell""#).count(),
            1,
            "the matched route shell must install once: {html}"
        );
        assert_eq!(
            html.matches(r#"data-webui-resource="dashboard-page""#)
                .count(),
            1,
            "the matched child route must install once: {html}"
        );
        assert!(
            !html.contains(r#"data-webui-resource="detail-page""#),
            "inactive route CSS must not be delivered: {html}"
        );
        let head_end = html.find("</head>").expect("head close");
        let head = &html[..head_end];
        for (resource, href) in [
            ("app-shell", "/app-shell.css"),
            ("dashboard-page", "/dashboard-page.css"),
        ] {
            let encoded_href = crate::html_encode::encode_safe(href);
            assert!(
                head.contains(&format!(
                    r#"<link rel="stylesheet" href="{encoded_href}" data-webui-resource="{resource}" data-webui-strategy="link">"#
                )),
                "Document-targeted route CSS must be render-blocking in head: {html}"
            );
            assert!(
                !head.contains(&format!(
                    r#"<link rel="preload" as="style" href="{encoded_href}">"#
                )),
                "a hoisted stylesheet must not also be preloaded: {html}"
            );
        }
        assert!(
            !head.contains("detail-page.css"),
            "inactive route CSS must not be preloaded: {html}"
        );
        let app_style = html
            .find(r#"data-webui-resource="app-shell""#)
            .expect("app style");
        let dashboard_style = html
            .find(r#"data-webui-resource="dashboard-page""#)
            .expect("dashboard style");
        let app_host = html.find("<app-shell data-wl").expect("app route host");
        let dashboard_host = html
            .find("<dashboard-page data-wl")
            .expect("dashboard route host");
        assert!(
            app_style < dashboard_style
                && dashboard_style < head_end
                && head_end < app_host
                && app_host < dashboard_host,
            "matched Document route closures must be hoisted in chain order: {html}"
        );

        let protocol = Protocol::new(protocol);
        let mut streamed = FlushTestWriter::default();
        WebUIHandler::new()
            .render_streaming(
                &protocol,
                &test_json!({}),
                &RenderOptions::new("index.html", "/"),
                &mut streamed,
            )
            .unwrap();
        let streamed_head_end = streamed
            .output
            .find("</head>")
            .expect("streamed head close");
        for resource in ["app-shell", "dashboard-page"] {
            let marker = format!(r#"data-webui-resource="{resource}""#);
            let position = streamed.output.find(&marker).expect("streamed route style");
            assert!(
                position < streamed_head_end,
                "streaming must hoist {resource} into head: {}",
                streamed.output
            );
            assert_eq!(
                streamed.output.matches(&marker).count(),
                1,
                "streaming must not emit {resource} twice: {}",
                streamed.output
            );
        }
    }

    #[test]
    fn bundled_routed_document_stylesheet_is_hoisted_once() {
        let route = WebUiFragmentRoute {
            path: "/".to_string(),
            fragment_id: "dashboard-page".to_string(),
            exact: true,
            ..Default::default()
        };
        let mut protocol = WebUIProtocol::new(HashMap::from([
            (
                "index.html".to_string(),
                FragmentList {
                    fragments: vec![
                        WebUIFragment::raw("<html><head>"),
                        structural_fragment("head_start"),
                        structural_fragment("head_end"),
                        WebUIFragment::raw("</head><body>"),
                        WebUIFragment::route_from(route),
                        structural_fragment("body_end"),
                        WebUIFragment::raw("</body></html>"),
                    ],
                },
            ),
            (
                "dashboard-page".to_string(),
                FragmentList {
                    fragments: vec![WebUIFragment::raw("<main>Dashboard</main>")],
                },
            ),
        ]));
        protocol.set_css_strategy(webui_protocol::CssStrategy::Link);
        for tag in ["dashboard-page", "summary-card", "activity-list"] {
            protocol
                .components
                .entry(tag.to_string())
                .or_default()
                .css_href = format!("/{tag}.css");
        }
        protocol.style_closures.insert(
            "index.html".to_string(),
            webui_protocol::ComponentStyleClosure::default(),
        );
        protocol.style_closures.insert(
            "dashboard-page".to_string(),
            webui_protocol::ComponentStyleClosure {
                component_tags: vec![
                    "dashboard-page".to_string(),
                    "summary-card".to_string(),
                    "activity-list".to_string(),
                ],
                style_chunks: vec![0],
            },
        );
        protocol.style_chunks.push(webui_protocol::StyleChunk {
            name: "_chunk-dashboard-page-3".to_string(),
            css: String::new(),
            css_href: "/_chunk-dashboard-page-3.css".to_string(),
            component_tags: vec![
                "dashboard-page".to_string(),
                "summary-card".to_string(),
                "activity-list".to_string(),
            ],
        });

        let mut writer = TestWriter::new();
        handle(
            &protocol,
            &test_json!({}),
            &RenderOptions::new("index.html", "/"),
            &mut writer,
        )
        .unwrap();
        let html = writer.get_content();
        let head_end = html.find("</head>").expect("head close");
        let marker = r#"data-webui-resource="_chunk-dashboard-page-3""#;
        let style = html.find(marker).expect("bundled route stylesheet");

        assert!(
            style < head_end,
            "bundled route CSS must be in head: {html}"
        );
        assert_eq!(
            html.matches(marker).count(),
            1,
            "the route host must not emit the hoisted chunk again: {html}"
        );
        let bundled_href = crate::html_encode::encode_safe("/_chunk-dashboard-page-3.css");
        assert!(
            !html.contains(&format!(
                r#"<link rel="preload" as="style" href="{bundled_href}">"#
            )),
            "the applied bundled stylesheet must not also be preloaded: {html}"
        );
    }

    #[test]
    fn routed_document_inline_styles_are_hoisted() {
        for strategy in [
            webui_protocol::CssStrategy::Style,
            webui_protocol::CssStrategy::Module,
        ] {
            let route = WebUiFragmentRoute {
                path: "/".to_string(),
                fragment_id: "dashboard-page".to_string(),
                exact: true,
                ..Default::default()
            };
            let mut protocol = WebUIProtocol::new(HashMap::from([
                (
                    "index.html".to_string(),
                    FragmentList {
                        fragments: vec![
                            WebUIFragment::raw("<html><head>"),
                            structural_fragment("head_start"),
                            structural_fragment("head_end"),
                            WebUIFragment::raw("</head><body>"),
                            WebUIFragment::route_from(route),
                            structural_fragment("body_end"),
                            WebUIFragment::raw("</body></html>"),
                        ],
                    },
                ),
                (
                    "dashboard-page".to_string(),
                    FragmentList {
                        fragments: vec![WebUIFragment::raw("<main>Dashboard</main>")],
                    },
                ),
            ]));
            protocol.set_css_strategy(strategy);
            protocol
                .components
                .entry("dashboard-page".to_string())
                .or_default()
                .css = ".dashboard{display:grid}".to_string();
            protocol.populate_style_closures(&["index.html"]);

            let mut writer = TestWriter::new();
            handle(
                &protocol,
                &test_json!({}),
                &RenderOptions::new("index.html", "/").with_nonce("css-nonce"),
                &mut writer,
            )
            .unwrap();
            let html = writer.get_content();
            let head_end = html.find("</head>").expect("head close");
            let marker = r#"data-webui-resource="dashboard-page""#;
            let style = html.find(marker).expect("route style fallback");
            let host = html
                .find("<dashboard-page data-wl")
                .expect("dashboard route host");
            let strategy_name = if strategy == webui_protocol::CssStrategy::Module {
                "module"
            } else {
                "style"
            };
            let style_marker = format!(
                r#"data-webui-resource="dashboard-page" data-webui-strategy="{strategy_name}""#
            );

            assert!(
                style < head_end && head_end < host,
                "{strategy:?} route CSS must be applied in head before its host: {html}"
            );
            assert_eq!(
                html.matches(&style_marker).count(),
                1,
                "{strategy:?} route CSS must not be emitted twice: {html}"
            );
        }
    }

    #[test]
    fn light_route_inside_static_shadow_root_is_only_preloaded_in_head() {
        let route = WebUiFragmentRoute {
            path: "/".to_string(),
            fragment_id: "dashboard-page".to_string(),
            exact: true,
            ..Default::default()
        };
        let mut protocol = WebUIProtocol::new(HashMap::from([
            (
                "index.html".to_string(),
                FragmentList {
                    fragments: vec![
                        WebUIFragment::raw("<html><head>"),
                        structural_fragment("head_start"),
                        structural_fragment("head_end"),
                        WebUIFragment::raw("</head><body><outer-box>"),
                        WebUIFragment::component("outer-box"),
                        WebUIFragment::raw("</outer-box></body></html>"),
                    ],
                },
            ),
            (
                "outer-box".to_string(),
                FragmentList {
                    fragments: vec![
                        WebUIFragment::raw("<template shadowrootmode=\"open\">"),
                        structural_fragment("shadow_styles:outer-box"),
                        WebUIFragment::route_from(route),
                        WebUIFragment::raw("</template>"),
                    ],
                },
            ),
            (
                "dashboard-page".to_string(),
                FragmentList {
                    fragments: vec![WebUIFragment::raw("<main>Dashboard</main>")],
                },
            ),
        ]));
        protocol.set_css_strategy(webui_protocol::CssStrategy::Link);
        for tag in ["outer-box", "dashboard-page"] {
            protocol
                .components
                .entry(tag.to_string())
                .or_default()
                .css_href = format!("/{tag}.css");
        }
        protocol
            .components
            .get_mut("outer-box")
            .expect("outer-box component")
            .uses_shadow_dom = true;
        protocol.populate_style_closures(&["index.html"]);

        let mut writer = TestWriter::new();
        handle(
            &protocol,
            &test_json!({}),
            &RenderOptions::new("index.html", "/"),
            &mut writer,
        )
        .unwrap();
        let html = writer.get_content();
        let head_end = html.find("</head>").expect("head close");
        let head = &html[..head_end];
        let dashboard_style = html
            .find(r#"data-webui-resource="dashboard-page""#)
            .expect("tree-local dashboard stylesheet");
        let shadow_start = html
            .find("<template shadowrootmode=\"open\">")
            .expect("Shadow template");
        let dashboard_host = html
            .find("<dashboard-page data-wl")
            .expect("dashboard route host");
        let dashboard_href = crate::html_encode::encode_safe("/dashboard-page.css");

        assert!(
            head.contains(&format!(
                r#"<link rel="preload" as="style" href="{dashboard_href}">"#
            )),
            "Shadow-targeted route CSS must start loading from head: {html}"
        );
        assert!(
            !head.contains(r#"data-webui-resource="dashboard-page""#),
            "Shadow-targeted route CSS must not be applied to Document: {html}"
        );
        assert!(
            shadow_start < dashboard_style && dashboard_style < dashboard_host,
            "the applying stylesheet must remain inside the owning ShadowRoot: {html}"
        );
    }

    #[test]
    fn isolated_shadow_entry_does_not_emit_document_preloads() {
        let route = WebUiFragmentRoute {
            path: "/".to_string(),
            fragment_id: "dashboard-page".to_string(),
            exact: true,
            ..Default::default()
        };
        let mut protocol = WebUIProtocol::new(HashMap::from([
            (
                "outer-box".to_string(),
                FragmentList {
                    fragments: vec![
                        WebUIFragment::raw("<template shadowrootmode=\"open\">"),
                        structural_fragment("shadow_styles:outer-box"),
                        WebUIFragment::route_from(route),
                        WebUIFragment::raw("</template>"),
                    ],
                },
            ),
            (
                "dashboard-page".to_string(),
                FragmentList {
                    fragments: vec![WebUIFragment::raw("<main>Dashboard</main>")],
                },
            ),
        ]));
        protocol.set_css_strategy(webui_protocol::CssStrategy::Link);
        for tag in ["outer-box", "dashboard-page"] {
            protocol
                .components
                .entry(tag.to_string())
                .or_default()
                .css_href = format!("/{tag}.css");
        }
        protocol
            .components
            .get_mut("outer-box")
            .expect("outer-box component")
            .uses_shadow_dom = true;
        protocol.populate_style_closures(&["outer-box"]);

        let mut writer = TestWriter::new();
        handle(
            &protocol,
            &test_json!({}),
            &RenderOptions::new("outer-box", "/"),
            &mut writer,
        )
        .unwrap();
        let html = writer.get_content();

        assert!(
            html.starts_with("<template shadowrootmode=\"open\">"),
            "isolated component output must not be prefixed with document metadata: {html}"
        );
        assert!(
            !html.contains(r#"<link rel="preload""#),
            "an isolated Shadow entry has no Document head for preloads: {html}"
        );
        assert!(
            html.contains(r#"data-webui-resource="dashboard-page""#),
            "the route stylesheet must still install inside the ShadowRoot: {html}"
        );
    }

    #[test]
    fn routed_shadow_styles_stay_tree_local_and_deduplicate_static_resources() {
        let route = WebUiFragmentRoute {
            path: "/".to_string(),
            fragment_id: "app-shell".to_string(),
            children: vec![
                WebUiFragmentRoute {
                    path: String::new(),
                    fragment_id: "dashboard-page".to_string(),
                    exact: true,
                    ..Default::default()
                },
                WebUiFragmentRoute {
                    path: "details".to_string(),
                    fragment_id: "detail-page".to_string(),
                    exact: true,
                    ..Default::default()
                },
            ],
            ..Default::default()
        };
        let fragments = HashMap::from([
            (
                "index.html".to_string(),
                FragmentList {
                    fragments: vec![
                        WebUIFragment::raw("<html><head>"),
                        structural_fragment("head_start"),
                        structural_fragment("head_end"),
                        WebUIFragment::raw("</head><body>"),
                        structural_fragment("body_start"),
                        structural_fragment("boundary_start:0"),
                        WebUIFragment::route_from(route),
                        structural_fragment("boundary_end:0"),
                        structural_fragment("body_end"),
                        WebUIFragment::raw("</body></html>"),
                    ],
                },
            ),
            (
                "app-shell".to_string(),
                FragmentList {
                    fragments: vec![
                        WebUIFragment::raw("<template shadowrootmode=\"open\">"),
                        structural_fragment("shadow_styles:app-shell"),
                        WebUIFragment::raw("<shared-card data-wl"),
                        structural_fragment("streaming_root:shared-card"),
                        WebUIFragment::raw(">"),
                        WebUIFragment::component("shared-card"),
                        WebUIFragment::raw("</shared-card>"),
                        WebUIFragment::outlet(),
                        WebUIFragment::raw("</template>"),
                    ],
                },
            ),
            (
                "dashboard-page".to_string(),
                FragmentList {
                    fragments: vec![
                        WebUIFragment::raw("<shared-card data-wl"),
                        structural_fragment("streaming_root:shared-card"),
                        WebUIFragment::raw(">"),
                        WebUIFragment::component("shared-card"),
                        WebUIFragment::raw("</shared-card><main>Dashboard</main>"),
                    ],
                },
            ),
            (
                "detail-page".to_string(),
                FragmentList {
                    fragments: vec![WebUIFragment::raw("<main>Detail</main>")],
                },
            ),
            (
                "shared-card".to_string(),
                FragmentList {
                    fragments: vec![WebUIFragment::raw("<p>Shared</p>")],
                },
            ),
        ]);
        let mut protocol = WebUIProtocol::new(fragments);
        protocol.set_css_strategy(webui_protocol::CssStrategy::Style);
        for tag in ["app-shell", "dashboard-page", "detail-page", "shared-card"] {
            let component = protocol.components.entry(tag.to_string()).or_default();
            component.css = format!(".{tag}{{display:block}}");
            component.template_json = r#"{"h":""}"#.to_string();
        }
        protocol
            .components
            .get_mut("app-shell")
            .expect("app shell component")
            .uses_shadow_dom = true;
        protocol.populate_style_closures(&["index.html"]);

        let mut writer = TestWriter::new();
        handle(
            &protocol,
            &test_json!({}),
            &RenderOptions::new("index.html", "/"),
            &mut writer,
        )
        .unwrap();
        let html = writer.get_content();

        for resource in ["app-shell", "shared-card", "dashboard-page"] {
            assert_eq!(
                html.matches(&format!(r#"data-webui-resource="{resource}""#))
                    .count(),
                1,
                "{resource} must install once in the active ShadowRoot: {html}"
            );
        }
        assert!(
            !html.contains(r#"data-webui-resource="detail-page""#),
            "inactive route CSS must not enter the ShadowRoot: {html}"
        );
        let shadow_start = html
            .find("<template shadowrootmode=\"open\">")
            .expect("Shadow template");
        let dashboard_style = html
            .find(r#"data-webui-resource="dashboard-page""#)
            .expect("dashboard style");
        let dashboard_host = html
            .find("<dashboard-page data-wl")
            .expect("dashboard route host");
        let shadow_end = html.find("</template>").expect("Shadow template close");
        assert!(
            shadow_start < dashboard_style
                && dashboard_style < dashboard_host
                && dashboard_host < shadow_end,
            "active route CSS must precede its generated host inside the owning ShadowRoot: {html}"
        );

        let protocol = Protocol::new(protocol);
        let mut streamed = FlushTestWriter::default();
        WebUIHandler::new()
            .render_streaming(
                &protocol,
                &test_json!({}),
                &RenderOptions::new("index.html", "/"),
                &mut streamed,
            )
            .unwrap();
        for resource in ["app-shell", "shared-card", "dashboard-page"] {
            assert_eq!(
                streamed
                    .output
                    .matches(&format!(r#"data-webui-resource="{resource}""#))
                    .count(),
                1,
                "streaming must preserve tree-local delivery for {resource}: {}",
                streamed.output
            );
        }
        assert!(
            !streamed
                .output
                .contains(r#"data-webui-resource="detail-page""#),
            "streaming must omit inactive route CSS: {}",
            streamed.output
        );
    }

    #[test]
    fn document_styles_follow_doctype_before_headless_document_content() {
        for entry_fragments in [
            vec![
                WebUIFragment::raw("<!DOCTYPE html><html><body>"),
                structural_fragment("body_start"),
                WebUIFragment::raw("<my-card data-wl>"),
                WebUIFragment::component("my-card"),
                WebUIFragment::raw("</my-card></body></html>"),
            ],
            vec![
                WebUIFragment::raw("<!doctype html><html><my-card data-wl>"),
                WebUIFragment::component("my-card"),
                WebUIFragment::raw("</my-card></html>"),
            ],
        ] {
            let mut protocol = WebUIProtocol::new(HashMap::from([
                (
                    "index.html".to_string(),
                    FragmentList {
                        fragments: entry_fragments,
                    },
                ),
                (
                    "my-card".to_string(),
                    FragmentList {
                        fragments: vec![WebUIFragment::raw("<p>card</p>")],
                    },
                ),
            ]));
            protocol.set_css_strategy(webui_protocol::CssStrategy::Style);
            let component = protocol
                .components
                .entry("my-card".to_string())
                .or_default();
            component.css = ".card{color:red}".to_string();
            component.uses_shadow_dom = false;
            protocol.populate_style_closures(&["index.html"]);

            let mut writer = TestWriter::new();
            handle(
                &protocol,
                &test_json!({}),
                &RenderOptions::new("index.html", "/"),
                &mut writer,
            )
            .expect("render");
            let html = writer.get_content();

            assert!(
                html.to_ascii_lowercase().starts_with("<!doctype html>"),
                "the doctype must remain the first document token: {html}"
            );
            let style = html.find("data-webui-resource=\"my-card\"").expect("style");
            let document = html.find("<html").expect("document root");
            let component = html.find("<my-card data-wl>").expect("component host");
            assert!(
                style < document && style < component,
                "headless document styles must precede document content: {html}"
            );
        }
    }

    #[test]
    fn authored_shadow_component_can_render_as_entry_fragment() {
        let mut protocol = WebUIProtocol::new(HashMap::from([(
            "my-card".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::raw("<template shadowrootmode=\"open\">"),
                    structural_fragment("shadow_styles:my-card"),
                    WebUIFragment::raw("<p>card</p></template>"),
                ],
            },
        )]));
        protocol.set_css_strategy(webui_protocol::CssStrategy::Style);
        let component = protocol
            .components
            .entry("my-card".to_string())
            .or_default();
        component.css = ".card{color:red}".to_string();
        component.uses_shadow_dom = true;
        protocol.populate_style_closures(&["my-card"]);

        let mut writer = TestWriter::new();
        handle(
            &protocol,
            &test_json!({}),
            &RenderOptions::new("my-card", "/"),
            &mut writer,
        )
        .expect("render");
        let html = writer.get_content();

        assert!(html.starts_with("<template shadowrootmode=\"open\"><style "));
        assert_eq!(html.matches("data-webui-resource=\"my-card\"").count(), 1);
        assert!(html.ends_with("<p>card</p></template>"));
    }

    #[test]
    fn component_style_closures_escape_mixed_case_style_end_tags() {
        for strategy in [
            webui_protocol::CssStrategy::Style,
            webui_protocol::CssStrategy::Module,
        ] {
            let mut protocol = WebUIProtocol::new(HashMap::from([
                (
                    "index.html".to_string(),
                    FragmentList {
                        fragments: vec![
                            WebUIFragment::raw("<html><head>"),
                            structural_fragment("head_end"),
                            WebUIFragment::raw("</head><body><safe-card>"),
                            WebUIFragment::component("safe-card"),
                            WebUIFragment::raw("</safe-card></body></html>"),
                        ],
                    },
                ),
                (
                    "safe-card".to_string(),
                    FragmentList {
                        fragments: vec![WebUIFragment::raw("<p>Safe</p>")],
                    },
                ),
            ]));
            protocol.set_css_strategy(strategy);
            protocol.components.insert(
                "safe-card".to_string(),
                webui_protocol::ComponentData {
                    css: ".safe{content:'</StYlE>'}".to_string(),
                    ..Default::default()
                },
            );
            protocol.populate_style_closures(&["index.html"]);
            let mut writer = TestWriter::new();

            handle(
                &protocol,
                &test_json!({}),
                &RenderOptions::new("index.html", "/"),
                &mut writer,
            )
            .unwrap();

            let html = writer.get_content();
            assert!(html.contains(".safe{content:'<\\/StYlE>'}"), "{html}");
            assert!(!html.contains(".safe{content:'</StYlE>'}"), "{html}");
        }
    }

    #[test]
    fn shadow_closure_supports_link_and_module_ssr_paths() {
        for strategy in [
            webui_protocol::CssStrategy::Link,
            webui_protocol::CssStrategy::Module,
        ] {
            let mut protocol = WebUIProtocol::new(HashMap::from([
                (
                    "index.html".to_string(),
                    FragmentList {
                        fragments: vec![
                            WebUIFragment::raw("<html><head>".to_string()),
                            structural_fragment("head_end"),
                            WebUIFragment::raw("</head><body><my-card>".to_string()),
                            WebUIFragment::component("my-card"),
                            WebUIFragment::raw("</my-card></body></html>".to_string()),
                        ],
                    },
                ),
                (
                    "my-card".to_string(),
                    FragmentList {
                        fragments: vec![
                            WebUIFragment::raw("<template shadowrootmode=\"open\">".to_string()),
                            structural_fragment("shadow_styles:my-card"),
                            WebUIFragment::raw("<p>card</p></template>".to_string()),
                        ],
                    },
                ),
            ]));
            protocol.set_css_strategy(strategy);
            let component = protocol
                .components
                .entry("my-card".to_string())
                .or_default();
            component.css = ".card{color:red}".to_string();
            component.css_href = "/my-card.css".to_string();
            component.uses_shadow_dom = true;
            protocol.populate_style_closures(&["index.html"]);

            let mut writer = TestWriter::new();
            handle(
                &protocol,
                &test_json!({}),
                &RenderOptions::new("index.html", "/"),
                &mut writer,
            )
            .unwrap();
            let html = writer.get_content();
            match strategy {
                webui_protocol::CssStrategy::Link => {
                    let escaped_href = crate::html_encode::encode_safe("/my-card.css");
                    assert!(html.contains(&format!(
                        "<template shadowrootmode=\"open\"><link rel=\"stylesheet\" href=\"{escaped_href}\" data-webui-resource=\"my-card\" data-webui-strategy=\"link\">"
                    )));
                }
                webui_protocol::CssStrategy::Module => {
                    assert!(html.contains(
                        "<template shadowrootmode=\"open\"><style data-webui-resource=\"my-card\" data-webui-strategy=\"module\">.card{color:red}</style>"
                    ));
                    assert_eq!(html.matches("<script type=\"importmap\"").count(), 1);
                }
                webui_protocol::CssStrategy::Style => unreachable!(),
            }
        }
    }

    #[test]
    fn test_module_preloads_emit_in_head_in_compiler_order() {
        // The whole value of these hints is ordering: preloads are issued in
        // document order over one connection, so the largest chunk must go
        // first. The compiler sorts; the handler must not reorder or dedupe.
        let mut fragments = HashMap::new();
        fragments.insert(
            "index.html".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::raw(
                        r#"<html><head><script type="module" async src="/index.js"></script>"#
                            .to_string(),
                    ),
                    structural_fragment("head_end"),
                    WebUIFragment::raw("</head><body>".to_string()),
                    structural_fragment("body_end"),
                    WebUIFragment::raw("</body></html>".to_string()),
                ],
            },
        );

        let mut protocol = WebUIProtocol::new(fragments);
        protocol.module_preloads = vec!["/chunk-big.js".to_string(), "/chunk-small.js".to_string()];

        let state = test_json!({});
        let mut writer = TestWriter::new();
        handle(
            &protocol,
            &state,
            &RenderOptions::new("index.html", "/"),
            &mut writer,
        )
        .unwrap();

        let html = writer.get_content();
        let head_end = html.find("</head>").expect("</head> missing");
        let head = &html[..head_end];
        assert!(
            head.contains(
                r#"<link rel="modulepreload" href="/chunk-big.js"><link rel="modulepreload" href="/chunk-small.js">"#
            ),
            "hints must appear in <head> in the compiler's order: {html}"
        );
    }

    #[test]
    fn test_no_module_preloads_emits_nothing() {
        let mut fragments = HashMap::new();
        fragments.insert(
            "index.html".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::raw("<html><head>".to_string()),
                    structural_fragment("head_end"),
                    WebUIFragment::raw("</head><body>".to_string()),
                    structural_fragment("body_end"),
                    WebUIFragment::raw("</body></html>".to_string()),
                ],
            },
        );
        let protocol = WebUIProtocol::new(fragments);

        let state = test_json!({});
        let mut writer = TestWriter::new();
        handle(
            &protocol,
            &state,
            &RenderOptions::new("index.html", "/"),
            &mut writer,
        )
        .unwrap();

        assert!(
            !writer.get_content().contains("modulepreload"),
            "a build without hints must be byte-identical to before"
        );
    }

    #[test]
    fn test_link_strategy_head_links_follow_document_order() {
        // Regression for #381: Link-strategy <head> CSS <link> tags must be
        // emitted in document/traversal order, not alphabetical tag order.
        // Document order here is <z-widget> then <a-widget>; an alphabetical
        // sort would (incorrectly) place a-widget first.
        let mut fragments = HashMap::new();
        fragments.insert(
            "index.html".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::raw("<html><head>".to_string()),
                    structural_fragment("head_end"),
                    WebUIFragment::raw("</head><body><z-widget>".to_string()),
                    WebUIFragment::component("z-widget"),
                    WebUIFragment::raw("</z-widget><a-widget>".to_string()),
                    WebUIFragment::component("a-widget"),
                    WebUIFragment::raw("</a-widget>".to_string()),
                    structural_fragment("body_end"),
                    WebUIFragment::raw("</body></html>".to_string()),
                ],
            },
        );
        fragments.insert(
            "z-widget".to_string(),
            FragmentList {
                fragments: vec![WebUIFragment::raw("<div>z</div>".to_string())],
            },
        );
        fragments.insert(
            "a-widget".to_string(),
            FragmentList {
                fragments: vec![WebUIFragment::raw("<div>a</div>".to_string())],
            },
        );

        let mut protocol = WebUIProtocol::new(fragments);
        protocol.set_css_strategy(webui_protocol::CssStrategy::Link);

        let z = protocol
            .components
            .entry("z-widget".to_string())
            .or_default();
        z.css_href = "z-widget.css".to_string();
        z.template_json = r#"{"h":"<div>z</div>"}"#.to_string();

        let a = protocol
            .components
            .entry("a-widget".to_string())
            .or_default();
        a.css_href = "a-widget.css".to_string();
        a.template_json = r#"{"h":"<div>a</div>"}"#.to_string();
        protocol.populate_style_closures(&["index.html"]);

        let state = test_json!({});
        let mut writer = TestWriter::new();

        handle(
            &protocol,
            &state,
            &RenderOptions::new("index.html", "/"),
            &mut writer,
        )
        .unwrap();

        let html = writer.get_content();
        let head_end = html.find("</head>").expect("</head> missing");
        let head_section = &html[..head_end];

        let z_pos = head_section
            .find(r#"<link rel="stylesheet" href="z-widget.css""#)
            .expect("z-widget stylesheet link missing from <head>");
        let a_pos = head_section
            .find(r#"<link rel="stylesheet" href="a-widget.css""#)
            .expect("a-widget stylesheet link missing from <head>");

        assert!(
            z_pos < a_pos,
            "CSS <link> tags must follow document order (z-widget before \
             a-widget), not alphabetical order: {html}"
        );
    }

    #[test]
    fn test_css_module_emitted_in_component_light_dom() {
        // CSS module <style> tags are emitted inline in the component's light DOM,
        // not in <head>. This keeps SSR output lean — only rendered components
        // get their style definitions.
        let mut fragments = HashMap::new();
        fragments.insert(
            "index.html".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::raw("<html><head>".to_string()),
                    structural_fragment("head_end"),
                    WebUIFragment::raw("</head><body><my-card>".to_string()),
                    WebUIFragment::component("my-card"),
                    WebUIFragment::raw("</my-card>".to_string()),
                    WebUIFragment::raw("</body></html>".to_string()),
                ],
            },
        );
        fragments.insert(
            "my-card".to_string(),
            FragmentList {
                fragments: vec![WebUIFragment::raw(r#"<p>hi</p>"#.to_string())],
            },
        );

        let mut protocol = WebUIProtocol::new(fragments);
        protocol.set_css_strategy(webui_protocol::CssStrategy::Module);
        protocol
            .components
            .entry("my-card".to_string())
            .or_default()
            .css = "p{color:red}".to_string();
        protocol.populate_style_closures(&["index.html"]);
        let state = test_json!({});
        let mut writer = TestWriter::new();

        handle(
            &protocol,
            &state,
            &RenderOptions::new("index.html", "/"),
            &mut writer,
        )
        .unwrap();

        let html = writer.get_content();

        // CSS module importmap must be INSIDE the component tag (light DOM)
        let tag_open = html.find("<my-card>").expect("<my-card> missing");
        let css_pos = html
            .find(r#"<script type="importmap""#)
            .expect("CSS module importmap missing");
        let tag_close = html.rfind("</my-card>").expect("</my-card> missing");
        assert!(
            css_pos > tag_open && css_pos < tag_close,
            "CSS module should be inside component light DOM: {html}"
        );

        // <head> should NOT contain module styles
        let head_end = html.find("</head>").expect("</head> missing");
        assert!(
            css_pos > head_end,
            "CSS module should not be in <head>: {html}"
        );
    }

    #[test]
    fn styled_protocol_without_closures_is_rejected_for_full_ssr() {
        let mut protocol = WebUIProtocol::default();
        protocol.fragments = HashMap::from([
            (
                "index.html".to_string(),
                FragmentList {
                    fragments: vec![
                        WebUIFragment::raw("<legacy-card>"),
                        WebUIFragment::component("legacy-card"),
                        WebUIFragment::raw("</legacy-card>"),
                    ],
                },
            ),
            (
                "legacy-card".to_string(),
                FragmentList {
                    fragments: vec![WebUIFragment::raw("<p>Legacy</p>")],
                },
            ),
        ]);
        protocol.components.insert(
            "legacy-card".to_string(),
            webui_protocol::ComponentData {
                css: ".legacy{color:red}".to_string(),
                ..Default::default()
            },
        );
        protocol.set_css_strategy(webui_protocol::CssStrategy::Module);
        let mut writer = TestWriter::new();

        let error = handle(
            &protocol,
            &test_json!({}),
            &RenderOptions::new("index.html", "/"),
            &mut writer,
        )
        .expect_err("styled protocols require closure metadata");
        assert!(error
            .to_string()
            .contains("component style closure metadata is required"));
    }

    #[test]
    fn test_css_module_emitted_for_route_components() {
        // Route components get CSS modules emitted inline in their light DOM.
        let template = r#"<h1>Dashboard</h1>"#;

        let mut fragments = HashMap::new();
        fragments.insert(
            "index.html".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::raw("<html><head>".to_string()),
                    structural_fragment("head_end"),
                    WebUIFragment::raw("</head><body>".to_string()),
                    WebUIFragment::route("/", "dash-page"),
                    structural_fragment("body_end"),
                    WebUIFragment::raw("</body></html>".to_string()),
                ],
            },
        );
        fragments.insert(
            "dash-page".to_string(),
            FragmentList {
                fragments: vec![WebUIFragment::raw(template.to_string())],
            },
        );

        let mut protocol = WebUIProtocol::new(fragments);
        protocol.set_css_strategy(webui_protocol::CssStrategy::Module);
        let comp = protocol
            .components
            .entry("dash-page".to_string())
            .or_default();
        comp.css = "h1{font-size:2rem}".to_string();
        comp.template_json = r#"{"h":"<h1>Dashboard</h1>"}"#.to_string();
        protocol.populate_style_closures(&["index.html"]);
        let state = test_json!({});
        let mut writer = TestWriter::new();

        handle(
            &protocol,
            &state,
            &RenderOptions::new("index.html", "/"),
            &mut writer,
        )
        .unwrap();

        let html = writer.get_content();

        assert!(
            html.contains(r#""dash-page":"data:text/css,h1{font-size:2rem}""#),
            "Route component should have CSS module importmap with data: URI: {html}"
        );
        assert!(
            html.contains("<h1>Dashboard</h1>"),
            "Route component should render content: {html}"
        );
        let fallback = html
            .find(r#"data-webui-resource="dash-page" data-webui-strategy="module""#)
            .expect("route Module fallback");
        let host_open = html.find("<dash-page data-wl>").expect("route host");
        let content = html.find("<h1>Dashboard</h1>").expect("route content");
        assert!(
            fallback < host_open && host_open < content,
            "the Module fallback must precede the generated Light route host: {html}"
        );
        // The importmap belongs to the component's own Light DOM, never to the
        // route element. The router only treats `<link>`/`<style>` markers as
        // route-owned styles, so an importmap emitted as a direct route child
        // would be cleared on navigation and mistaken for the mounted
        // component. Keep it inside the host so that can never happen.
        let importmap = html
            .find(r#"<script type="importmap""#)
            .expect("route CSS module importmap");
        assert!(
            host_open < importmap,
            "the CSS module importmap must live inside the route host, not as a route child: {html}"
        );
    }

    #[test]
    fn test_head_css_link_skipped_for_components_without_css() {
        // Regression: components without CSS files must not get <link> tags
        // in <head>, otherwise the browser requests a 404.
        let mut fragments = HashMap::new();
        fragments.insert(
            "index.html".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::raw("<html><head>".to_string()),
                    structural_fragment("head_end"),
                    WebUIFragment::raw("</head><body>".to_string()),
                    WebUIFragment::component("has-css"),
                    WebUIFragment::component("no-css"),
                    structural_fragment("body_end"),
                    WebUIFragment::raw("</body></html>".to_string()),
                ],
            },
        );
        fragments.insert(
            "has-css".to_string(),
            FragmentList {
                fragments: vec![WebUIFragment::raw("<p>styled</p>".to_string())],
            },
        );
        fragments.insert(
            "no-css".to_string(),
            FragmentList {
                fragments: vec![WebUIFragment::raw("<p>plain</p>".to_string())],
            },
        );

        let mut protocol = WebUIProtocol::new(fragments);
        protocol.set_css_strategy(webui_protocol::CssStrategy::Link);

        // Only has-css has an external stylesheet (Link strategy)
        protocol
            .components
            .entry("has-css".to_string())
            .or_default()
            .css_href = "has-css.css".to_string();
        protocol
            .components
            .entry("no-css".to_string())
            .or_default()
            .template_json = r#"{"h":"<p>plain</p>"}"#.to_string();
        protocol.populate_style_closures(&["index.html"]);

        let state = test_json!({});
        let mut writer = TestWriter::new();

        let handler = WebUIHandler::with_plugin(|| {
            Box::new(crate::plugin::webui::WebUIHydrationPlugin::new())
        });
        handler
            .handle(
                &protocol,
                &state,
                &RenderOptions::new("index.html", "/"),
                &mut writer,
            )
            .unwrap();

        let html = writer.get_content();
        assert!(
            html.contains(r#"<link rel="stylesheet" href="has-css.css""#),
            "Component with CSS should get a <link rel=stylesheet> in <head>: {html}"
        );
        assert!(
            !html.contains("no-css.css"),
            "Component without CSS must NOT get a <link> in <head>: {html}"
        );
    }

    #[test]
    fn test_reachable_unrendered_components_get_templates_and_css_but_not_inventory() {
        // Simulates a page where app-shell renders cart-panel, but cart-panel
        // contains an <if> block with product-card inside. When the condition
        // is false (empty cart), product-card is NOT rendered — but it IS
        // reachable from the fragment graph. Its template metadata and CSS module
        // definition must be in the output so the client can mount it when
        // the <if> flips true. However, its bit must NOT be set in the
        // inventory — the inventory tracks what was actually rendered.
        let mut fragments = HashMap::new();
        fragments.insert(
            "index.html".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::raw("<html><head>".to_string()),
                    structural_fragment("head_end"),
                    WebUIFragment::raw("</head><body><app-shell>".to_string()),
                    WebUIFragment::component("app-shell"),
                    WebUIFragment::raw("</app-shell>".to_string()),
                    structural_fragment("body_end"),
                    WebUIFragment::raw("</body></html>".to_string()),
                ],
            },
        );
        // app-shell contains a cart panel
        fragments.insert(
            "app-shell".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::raw("<div>Shell</div>".to_string()),
                    WebUIFragment::component("cart-panel"),
                ],
            },
        );
        // cart-panel has an <if> block containing product-card
        fragments.insert(
            "cart-panel".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::raw("<aside>".to_string()),
                    WebUIFragment::if_cond(ConditionExpr::identifier("hasItems"), "cart-items"),
                    WebUIFragment::raw("</aside>".to_string()),
                ],
            },
        );
        // cart-items (if block body) contains product-card
        fragments.insert(
            "cart-items".to_string(),
            FragmentList {
                fragments: vec![WebUIFragment::component("product-card")],
            },
        );
        fragments.insert(
            "product-card".to_string(),
            FragmentList {
                fragments: vec![WebUIFragment::raw("<div>Card</div>".to_string())],
            },
        );

        let mut protocol = WebUIProtocol::new(fragments);
        protocol.set_css_strategy(webui_protocol::CssStrategy::Module);
        protocol.initial_state_strategy = InitialStateStrategy::Components as i32;
        for name in ["app-shell", "cart-panel", "product-card"] {
            let comp = protocol.components.entry(name.to_string()).or_default();
            comp.template_json = format!(r#"{{"h":"<div class=\"{name}\"></div>"}}"#);
            comp.css = format!(".{name}{{display:block}}");
            if name == "cart-panel" {
                comp.hydration_mode = StateProjectionMode::Keys as i32;
                comp.hydration_keys = vec!["hasItems".to_string()];
            }
            if name == "product-card" {
                comp.template_functions = r#"[function(v,s){return !!v("ready",s)}]"#.to_string();
            }
        }
        protocol.populate_style_closures(&["index.html"]);

        // Render with hasItems=false — product-card should NOT be rendered
        let state = test_json!({ "hasItems": false });
        let mut writer = TestWriter::new();

        let handler = WebUIHandler::with_plugin(|| {
            Box::new(crate::plugin::webui::WebUIHydrationPlugin::new())
        });
        handler
            .handle(
                &protocol,
                &state,
                &RenderOptions::new("index.html", "/"),
                &mut writer,
            )
            .unwrap();

        let html = writer.get_content();

        assert!(
            html.contains(r#"<script type="application/json" id="webui-data">"#),
            "non-executable SSR metadata should be emitted in the webui-data block: {html}"
        );
        assert!(
            html.contains(r#""state":{"hasItems":false}"#),
            "SSR state should live in the JSON data block: {html}"
        );
        assert!(
            html.contains(r#""inventory":"#),
            "SSR inventory should live in the JSON data block: {html}"
        );
        assert!(
            !html.contains("window.__webui={\""),
            "executable bootstrap must not embed the window.__webui JSON literal: {html}"
        );
        assert!(
            !html.contains(r#"document.getElementById("webui-data")"#),
            "SSR must not parse webui-data; client packages own that lazy load: {html}"
        );
        assert!(
            !html.contains("window.__webui=w;"),
            "executable bootstrap must not replace existing window.__webui registrations: {html}"
        );
        assert!(
            !html.contains("w.templateFns={\""),
            "template function emission must not replace existing templateFns registrations: {html}"
        );
        assert!(
            html.contains(r#"var f=w.templateFns||(w.templateFns={});f["product-card"]=[function(v,s){return !!v("ready",s)}];"#),
            "template functions should merge into the flat templateFns registry: {html}"
        );

        // product-card template IS in the output — it's a known component
        // whose template must be available for client-side <if> activation.
        assert!(
            html.contains(r#""product-card":{"h":"<div class=\"product-card\"><\/div>"}"#),
            "product-card template should be emitted even when unrendered: {html}"
        );

        // product-card CSS module IS in the output — reachable components need
        // their stylesheet definitions for client-side <if> activation.
        assert!(
            html.contains(r#""product-card":"data:text/css,"#),
            "reachable product-card CSS module importmap should be emitted: {html}"
        );

        // app-shell and cart-panel SHOULD be in the output (they were rendered)
        assert!(
            html.contains(r#""app-shell":{"h":"<div class=\"app-shell\"><\/div>"}"#),
            "rendered app-shell template should be emitted: {html}"
        );
        assert!(
            html.contains(r#""cart-panel":{"h":"<div class=\"cart-panel\"><\/div>"}"#),
            "rendered cart-panel template should be emitted: {html}"
        );
    }

    // ── CSP nonce on CSS module importmap ───────────────────────────
    //
    // When `RenderOptions::with_nonce(...)` is set, every inline
    // `<script type="importmap">` definition emitted during SSR for a
    // component CSS module must include `nonce="VALUE"` so strict CSP
    // `script-src 'nonce-...'` policies allow it. The without-nonce case
    // is already covered by other CSS module tests (e.g.
    // `test_css_module_emitted_for_route_components`).

    #[test]
    fn test_css_module_emits_nonce_attribute_when_nonce_set() {
        // Per-component first-render path (`emit_css_module`).
        let template = r#"<h1>Dashboard</h1>"#;

        let mut fragments = HashMap::new();
        fragments.insert(
            "index.html".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::raw("<html><head>".to_string()),
                    structural_fragment("head_end"),
                    WebUIFragment::raw("</head><body>".to_string()),
                    WebUIFragment::route("/", "dash-page"),
                    structural_fragment("body_end"),
                    WebUIFragment::raw("</body></html>".to_string()),
                ],
            },
        );
        fragments.insert(
            "dash-page".to_string(),
            FragmentList {
                fragments: vec![WebUIFragment::raw(template.to_string())],
            },
        );

        let mut protocol = WebUIProtocol::new(fragments);
        protocol.set_css_strategy(webui_protocol::CssStrategy::Module);
        let comp = protocol
            .components
            .entry("dash-page".to_string())
            .or_default();
        comp.css = "h1{font-size:2rem}".to_string();
        comp.template_json = r#"{"h":"<h1>Dashboard</h1>"}"#.to_string();
        protocol.populate_style_closures(&["index.html"]);
        let state = test_json!({});
        let mut writer = TestWriter::new();

        handle(
            &protocol,
            &state,
            &RenderOptions::new("index.html", "/").with_nonce("test-nonce-123"),
            &mut writer,
        )
        .unwrap();

        let html = writer.get_content();

        assert!(
            html.contains(
                r#"<script type="importmap" nonce="test-nonce-123" data-webui-resource="dash-page">{"imports":{"dash-page":"data:text/css,h1{font-size:2rem}"}}</script>"#
            ),
            "CSS module importmap tag should include nonce attribute in canonical order: {html}"
        );
    }

    #[test]
    fn test_unrendered_css_module_emits_nonce_attribute_when_nonce_set() {
        // Body-end emission path for reachable-but-unrendered components
        // (the second site touched by the patch). Triggered via a false
        // `<if>` block under hydration; requires the WebUI plugin so the
        // body_end hook executes.
        let mut fragments = HashMap::new();
        fragments.insert(
            "index.html".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::raw("<html><head>".to_string()),
                    structural_fragment("head_end"),
                    WebUIFragment::raw("</head><body><app-shell>".to_string()),
                    WebUIFragment::component("app-shell"),
                    WebUIFragment::raw("</app-shell>".to_string()),
                    structural_fragment("body_end"),
                    WebUIFragment::raw("</body></html>".to_string()),
                ],
            },
        );
        fragments.insert(
            "app-shell".to_string(),
            FragmentList {
                fragments: vec![WebUIFragment::if_cond(
                    ConditionExpr::identifier("hasItems"),
                    "cart-items",
                )],
            },
        );
        fragments.insert(
            "cart-items".to_string(),
            FragmentList {
                fragments: vec![WebUIFragment::component("product-card")],
            },
        );
        fragments.insert(
            "product-card".to_string(),
            FragmentList {
                fragments: vec![WebUIFragment::raw("<div>Card</div>".to_string())],
            },
        );

        let mut protocol = WebUIProtocol::new(fragments);
        protocol.set_css_strategy(webui_protocol::CssStrategy::Module);
        for name in ["app-shell", "product-card"] {
            let comp = protocol.components.entry(name.to_string()).or_default();
            comp.template_json = format!(r#"{{"h":"<div class=\"{name}\"></div>"}}"#);
            comp.css = format!(".{name}{{display:block}}");
        }
        protocol.populate_style_closures(&["index.html"]);

        // Render with hasItems=false so product-card is reachable but not
        // rendered, forcing its CSS module emission through the body_end path.
        let state = test_json!({ "hasItems": false });
        let mut writer = TestWriter::new();

        let handler = WebUIHandler::with_plugin(|| {
            Box::new(crate::plugin::webui::WebUIHydrationPlugin::new())
        });
        handler
            .handle(
                &protocol,
                &state,
                &RenderOptions::new("index.html", "/").with_nonce("test-nonce-123"),
                &mut writer,
            )
            .unwrap();

        let html = writer.get_content();

        assert!(
            html.contains(
                r#"<script type="importmap" nonce="test-nonce-123" data-webui-resource="product-card">{"imports":{"product-card":"data:text/css,.product-card{display:block}"}}</script>"#
            ),
            "Unrendered (body_end) CSS module importmap tag should include nonce attribute in canonical order: {html}"
        );
    }

    #[test]
    fn projected_state_excludes_non_hydration_keys() -> Result<()> {
        let mut fragments = HashMap::new();
        fragments.insert(
            "index.html".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::raw("<html><body><style>".to_string()),
                    WebUIFragment::signal("tokens.light", true),
                    WebUIFragment::raw("</style>".to_string()),
                    structural_fragment("body_end"),
                    WebUIFragment::raw("</body></html>".to_string()),
                ],
            },
        );
        fragments.insert(
            "app-shell".to_string(),
            FragmentList {
                fragments: vec![WebUIFragment::raw("<span>shell</span>".to_string())],
            },
        );
        let index_fragments = fragments
            .get_mut("index.html")
            .expect("index fixture should exist");
        index_fragments
            .fragments
            .insert(1, WebUIFragment::component("app-shell"));
        let mut protocol = WebUIProtocol::new(fragments);
        protocol.initial_state_strategy = InitialStateStrategy::Components as i32;
        // Only `name` is a hydration key. `tokens` is a server-only field
        // (used above to resolve SSR CSS variables) and is NOT in the component
        // hydration keys,
        // so projection MUST keep it out of the client state block.
        protocol.components.insert(
            "app-shell".to_string(),
            webui_protocol::ComponentData {
                hydration_mode: StateProjectionMode::Keys as i32,
                hydration_keys: vec!["name".to_string()],
                ..Default::default()
            },
        );
        let state = test_json!({
            "name": "Alice",
            "tokens": {
                "light": "--color-brand: red;"
            }
        });
        let mut writer = TestWriter::new();
        let handler = WebUIHandler::with_plugin(|| {
            Box::new(crate::plugin::webui::WebUIHydrationPlugin::new())
        });
        handler.handle(
            &protocol,
            &state,
            &RenderOptions::new("index.html", "/"),
            &mut writer,
        )?;
        let output = writer.get_content();

        // SSR still reads `tokens` to resolve the inline <style>...
        assert!(output.contains("--color-brand: red;"));
        // ...but only the hydration key reaches the client state.
        assert!(output.contains(r#""name":"Alice""#));
        assert!(!output.contains(r#""tokens""#));
        Ok(())
    }

    #[test]
    fn full_initial_strategy_preserves_complete_state() -> Result<()> {
        let mut fragments = HashMap::new();
        fragments.insert(
            "index.html".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::raw("<html><body>"),
                    structural_fragment("body_end"),
                    WebUIFragment::raw("</body></html>"),
                ],
            },
        );
        let protocol = WebUIProtocol::new(fragments);
        let state = test_json!({
            "client": "visible",
            "serverOnly": "also preserved",
        });
        let handler = WebUIHandler::with_plugin(|| {
            Box::new(crate::plugin::webui::WebUIHydrationPlugin::new())
        });
        let mut writer = TestWriter::new();
        handler.handle(
            &protocol,
            &state,
            &RenderOptions::new("index.html", "/"),
            &mut writer,
        )?;
        let output = writer.get_content();
        assert!(output.contains(r#""client":"visible""#));
        assert!(output.contains(r#""serverOnly":"also preserved""#));
        Ok(())
    }

    #[test]
    fn uncertain_hydration_surface_preserves_complete_state() -> Result<()> {
        let mut fragments = HashMap::new();
        fragments.insert(
            "index.html".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::raw("<html><body>"),
                    WebUIFragment::component("app-shell"),
                    structural_fragment("body_end"),
                    WebUIFragment::raw("</body></html>"),
                ],
            },
        );
        fragments.insert(
            "app-shell".to_string(),
            FragmentList {
                fragments: vec![WebUIFragment::raw("<p>Shell</p>")],
            },
        );
        let mut protocol = WebUIProtocol::new(fragments);
        protocol.initial_state_strategy = InitialStateStrategy::Components as i32;
        protocol.components.insert(
            "app-shell".to_string(),
            webui_protocol::ComponentData {
                hydration_mode: StateProjectionMode::All as i32,
                ..Default::default()
            },
        );
        let state = test_json!({
            "known": "value",
            "possiblyInherited": "must not be dropped",
        });
        let handler = WebUIHandler::with_plugin(|| {
            Box::new(crate::plugin::webui::WebUIHydrationPlugin::new())
        });
        let mut writer = TestWriter::new();
        handler.handle(
            &protocol,
            &state,
            &RenderOptions::new("index.html", "/"),
            &mut writer,
        )?;
        let output = writer.get_content();
        assert!(output.contains(r#""known":"value""#));
        assert!(output.contains(r#""possiblyInherited":"must not be dropped""#));
        Ok(())
    }

    #[test]
    fn missing_component_projection_metadata_preserves_complete_state() -> Result<()> {
        let mut fragments = HashMap::new();
        fragments.insert(
            "index.html".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::raw("<html><body>"),
                    WebUIFragment::component("app-shell"),
                    structural_fragment("body_end"),
                    WebUIFragment::raw("</body></html>"),
                ],
            },
        );
        fragments.insert(
            "app-shell".to_string(),
            FragmentList {
                fragments: vec![WebUIFragment::raw("<p>Shell</p>")],
            },
        );
        let mut protocol = WebUIProtocol::new(fragments);
        protocol.initial_state_strategy = InitialStateStrategy::Components as i32;
        let state = test_json!({
            "known": "value",
            "serverOnly": "must not be dropped",
        });
        let handler = WebUIHandler::with_plugin(|| {
            Box::new(crate::plugin::webui::WebUIHydrationPlugin::new())
        });
        let mut writer = TestWriter::new();
        handler.handle(
            &protocol,
            &state,
            &RenderOptions::new("index.html", "/"),
            &mut writer,
        )?;
        let output = writer.get_content();
        assert!(output.contains(r#""known":"value""#));
        assert!(output.contains(r#""serverOnly":"must not be dropped""#));
        Ok(())
    }

    #[test]
    fn unknown_projection_mode_preserves_complete_state() -> Result<()> {
        let mut fragments = HashMap::new();
        fragments.insert(
            "index.html".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::raw("<html><body>"),
                    WebUIFragment::component("app-shell"),
                    structural_fragment("body_end"),
                    WebUIFragment::raw("</body></html>"),
                ],
            },
        );
        fragments.insert(
            "app-shell".to_string(),
            FragmentList {
                fragments: vec![WebUIFragment::raw("<p>Shell</p>")],
            },
        );
        let mut protocol = WebUIProtocol::new(fragments);
        protocol.initial_state_strategy = InitialStateStrategy::Components as i32;
        protocol.components.insert(
            "app-shell".to_string(),
            webui_protocol::ComponentData {
                hydration_mode: i32::MAX,
                ..Default::default()
            },
        );
        let state = test_json!({
            "known": "value",
            "serverOnly": "must not be dropped",
        });
        let handler = WebUIHandler::with_plugin(|| {
            Box::new(crate::plugin::webui::WebUIHydrationPlugin::new())
        });
        let mut writer = TestWriter::new();
        handler.handle(
            &protocol,
            &state,
            &RenderOptions::new("index.html", "/"),
            &mut writer,
        )?;
        let output = writer.get_content();
        assert!(output.contains(r#""known":"value""#));
        assert!(output.contains(r#""serverOnly":"must not be dropped""#));
        Ok(())
    }

    #[test]
    fn legacy_navigation_keys_with_default_mode_remain_keyed() {
        let mut protocol = WebUIProtocol::new(HashMap::new());
        protocol.components.insert(
            "app-shell".to_string(),
            webui_protocol::ComponentData {
                navigation_keys: vec!["selected".to_string()],
                ..Default::default()
            },
        );

        match collect_navigation_state(&protocol, ["app-shell"]) {
            StateSelection::Keys(keys) => assert_eq!(keys, vec!["selected"]),
            StateSelection::Full | StateSelection::BorrowedKeys(_) => {
                panic!("legacy navigation keys should remain owned and projected")
            }
        }
    }

    #[test]
    fn legacy_navigation_without_projection_metadata_preserves_full_state() {
        let mut protocol = WebUIProtocol::new(HashMap::new());
        protocol.components.insert(
            "app-shell".to_string(),
            webui_protocol::ComponentData::default(),
        );

        assert!(matches!(
            collect_navigation_state(&protocol, ["app-shell"]),
            StateSelection::Full
        ));
    }

    #[test]
    fn empty_reachable_hydration_keys_exclude_all_state() -> Result<()> {
        let mut fragments = HashMap::new();
        fragments.insert(
            "index.html".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::raw("<html><body>".to_string()),
                    structural_fragment("body_end"),
                    WebUIFragment::raw("</body></html>".to_string()),
                ],
            },
        );
        let mut protocol = WebUIProtocol::new(fragments);
        protocol.initial_state_strategy = InitialStateStrategy::Components as i32;
        let state = test_json!({
            "title": "Legacy state",
            "serverOnly": "preserved",
        });
        let handler = WebUIHandler::with_plugin(|| {
            Box::new(crate::plugin::webui::WebUIHydrationPlugin::new())
        });

        let mut writer = TestWriter::new();
        handler.handle(
            &protocol,
            &state,
            &RenderOptions::new("index.html", "/"),
            &mut writer,
        )?;
        assert!(writer.get_content().contains(r#""state":{}"#));
        assert!(!writer.get_content().contains("Legacy state"));
        assert!(!writer.get_content().contains("preserved"));
        Ok(())
    }

    #[test]
    fn scriptless_component_state_is_navigation_only() -> Result<()> {
        let mut fragments = HashMap::new();
        fragments.insert(
            "index.html".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::raw("<html><body>"),
                    WebUIFragment::component("items-page"),
                    structural_fragment("body_end"),
                    WebUIFragment::raw("</body></html>"),
                ],
            },
        );
        fragments.insert(
            "items-page".to_string(),
            FragmentList {
                fragments: vec![WebUIFragment::raw("<p>Items</p>")],
            },
        );
        let mut protocol = WebUIProtocol::new(fragments);
        protocol.initial_state_strategy = InitialStateStrategy::Components as i32;
        protocol.components.insert(
            "items-page".to_string(),
            webui_protocol::ComponentData {
                template_json: r#"{"h":"<p>Items</p>","th":1}"#.into(),
                navigation_mode: Some(StateProjectionMode::Keys as i32),
                navigation_keys: vec!["items".into()],
                ..Default::default()
            },
        );
        let state = test_json!({
            "items": ["STATE_SENTINEL"],
            "serverOnly": "SECRET_SENTINEL",
        });
        let handler = WebUIHandler::with_plugin(|| {
            Box::new(crate::plugin::webui::WebUIHydrationPlugin::new())
        });
        let mut writer = TestWriter::new();

        handler.handle(
            &protocol,
            &state,
            &RenderOptions::new("index.html", "/"),
            &mut writer,
        )?;

        let output = writer.get_content();
        assert!(output.contains(r#""state":{}"#));
        assert!(!output.contains("STATE_SENTINEL"));
        assert!(!output.contains("SECRET_SENTINEL"));
        Ok(())
    }

    #[test]
    fn write_selected_state_projects_and_escapes() {
        // `keep` is in the sorted key set and its value contains a `</` that
        // must be escaped; `drop` is absent and must be projected out.
        let state = test_json!({
            "drop": "secret",
            "keep": "</script><b>"
        });
        let keys = ["keep"];
        let selection = StateSelection::Keys(keys.to_vec());
        let mut sink = TestWriter::new();
        let mut scratch = Vec::new();
        write_selected_state(&mut sink, &mut scratch, &state, &selection).unwrap();
        assert_eq!(sink.get_content(), r#"{"keep":"<\/script><b>"}"#);
    }

    #[test]
    fn write_selected_state_non_object_projection_emits_empty_object() {
        let state = test_json!("scalar state has nothing hydratable");
        let keys: [&str; 0] = [];
        let selection = StateSelection::Keys(keys.to_vec());
        let mut sink = TestWriter::new();
        let mut scratch = Vec::new();
        write_selected_state(&mut sink, &mut scratch, &state, &selection).unwrap();
        assert_eq!(sink.get_content(), "{}");
    }

    #[test]
    fn write_selected_state_schema_first_skips_missing_and_duplicate_keys() {
        let state = test_json!({
            "keptA": 1,
            "keptB": 2,
            "serverOnlyA": 3,
            "serverOnlyB": 4,
        });
        let keys = ["keptA", "keptA", "keptB", "missing"];
        let selection = StateSelection::Keys(keys.to_vec());
        let mut sink = TestWriter::new();
        let mut scratch = Vec::new();
        write_selected_state(&mut sink, &mut scratch, &state, &selection).unwrap();
        assert_eq!(sink.get_content(), r#"{"keptA":1,"keptB":2}"#);
    }

    #[test]
    fn write_selected_state_map_first_matches_schema_first_output() {
        let state = test_json!({
            "keptA": 1,
            "keptB": 2,
        });
        let keys = ["keptA", "keptB", "missingA", "missingB"];
        let selection = StateSelection::Keys(keys.to_vec());
        let mut sink = TestWriter::new();
        let mut scratch = Vec::new();
        write_selected_state(&mut sink, &mut scratch, &state, &selection).unwrap();
        assert_eq!(sink.get_content(), r#"{"keptA":1,"keptB":2}"#);
    }

    #[test]
    fn write_selected_state_full_preserves_and_escapes_state() {
        let state = test_json!({
            "serverOnly": "</script><b>",
            "value": 42,
        });
        let mut sink = TestWriter::new();
        let mut scratch = Vec::new();
        write_selected_state(&mut sink, &mut scratch, &state, &StateSelection::Full).unwrap();
        assert_eq!(
            sink.get_content(),
            r#"{"serverOnly":"<\/script><b>","value":42}"#
        );
    }

    #[test]
    fn json_scratch_capacity_reused_across_checkpoint_payloads() {
        // Simulates several checkpoint bootstraps serialized in one render: a
        // single request-local buffer must grow once for the largest payload and
        // then be reused for subsequent payloads without reallocating. This is
        // the deterministic seam behind the streaming path's allocation gate —
        // `write_script_safe_json` is `pub(crate)`, so no public API is exposed.
        let mut sink = TestWriter::new();
        let mut scratch: Vec<u8> = Vec::new();
        // No allocation until serialization actually needs it.
        assert_eq!(scratch.capacity(), 0);

        // The largest payload first establishes the high-water capacity.
        let mut large = serde_json::Map::new();
        for index in 0..64 {
            large.insert(
                format!("component_{index:04}"),
                Value::String("x".repeat(48)),
            );
        }
        let large = Value::Object(large);
        write_script_safe_json(&mut sink, &mut scratch, &large).unwrap();
        let high_water = scratch.capacity();
        assert!(high_water > 0);

        // Subsequent smaller checkpoint payloads reuse the same buffer: capacity
        // never grows (no per-checkpoint allocation) and never shrinks.
        for seq in 0..8 {
            let small = test_json!({ "inventory": "", "state": {}, "seq": seq });
            write_script_safe_json(&mut sink, &mut scratch, &small).unwrap();
            assert_eq!(
                scratch.capacity(),
                high_water,
                "scratch reallocated on checkpoint {seq}"
            );
        }
    }

    #[test]
    fn bootstrap_state_excludes_inactive_route_hydration_keys() -> Result<()> {
        let mut fragments = HashMap::new();
        fragments.insert(
            "index.html".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::raw("<html><body>"),
                    WebUIFragment::route_from(webui_protocol::WebUiFragmentRoute {
                        path: "/".to_string(),
                        fragment_id: "home-page".to_string(),
                        exact: true,
                        ..Default::default()
                    }),
                    WebUIFragment::route_from(webui_protocol::WebUiFragmentRoute {
                        path: "/admin".to_string(),
                        fragment_id: "admin-page".to_string(),
                        exact: true,
                        ..Default::default()
                    }),
                    structural_fragment("body_end"),
                    WebUIFragment::raw("</body></html>"),
                ],
            },
        );
        fragments.insert(
            "home-page".to_string(),
            FragmentList {
                fragments: vec![WebUIFragment::raw("<p>Home</p>")],
            },
        );
        fragments.insert(
            "admin-page".to_string(),
            FragmentList {
                fragments: vec![WebUIFragment::raw("<p>Admin</p>")],
            },
        );

        let mut protocol = WebUIProtocol::new(fragments);
        protocol.initial_state_strategy = InitialStateStrategy::Components as i32;
        protocol.components.insert(
            "home-page".to_string(),
            webui_protocol::ComponentData {
                template_json: "{}".to_string(),
                hydration_mode: StateProjectionMode::Keys as i32,
                hydration_keys: vec!["homeTitle".to_string()],
                ..Default::default()
            },
        );
        protocol.components.insert(
            "admin-page".to_string(),
            webui_protocol::ComponentData {
                template_json: "{}".to_string(),
                hydration_mode: StateProjectionMode::Keys as i32,
                hydration_keys: vec!["adminToken".to_string()],
                ..Default::default()
            },
        );
        let state = test_json!({
            "homeTitle": "Welcome",
            "adminToken": "TOP_SECRET_SENTINEL",
        });
        let handler = WebUIHandler::with_plugin(|| {
            Box::new(crate::plugin::webui::WebUIHydrationPlugin::new())
        });
        let mut writer = TestWriter::new();
        handler.handle(
            &protocol,
            &state,
            &RenderOptions::new("index.html", "/"),
            &mut writer,
        )?;
        let output = writer.get_content();
        assert!(output.contains(r#""homeTitle":"Welcome""#));
        assert!(!output.contains("TOP_SECRET_SENTINEL"));
        assert!(!output.contains(r#""adminToken""#));
        Ok(())
    }

    #[test]
    fn test_component_attr_name_aria() {
        // component_attr_name correctly maps ARIA attributes via the shared table
        assert_eq!(component_attr_name("aria-describedby"), "ariaDescribedBy");
        assert_eq!(component_attr_name("aria-labelledby"), "ariaLabelledBy");
        assert_eq!(
            component_attr_name("aria-activedescendant"),
            "ariaActiveDescendant"
        );
        assert_eq!(component_attr_name("aria-label"), "ariaLabel");
        assert_eq!(component_attr_name("aria-hidden"), "ariaHidden");
    }

    #[test]
    fn test_component_attr_name_html_global() {
        assert_eq!(component_attr_name("readonly"), "readOnly");
        assert_eq!(component_attr_name("tabindex"), "tabIndex");
        assert_eq!(component_attr_name("accesskey"), "accessKey");
        assert_eq!(component_attr_name("contenteditable"), "contentEditable");
        assert_eq!(component_attr_name("crossorigin"), "crossOrigin");
        assert_eq!(component_attr_name("inputmode"), "inputMode");
        assert_eq!(component_attr_name("maxlength"), "maxLength");
        assert_eq!(component_attr_name("minlength"), "minLength");
        assert_eq!(component_attr_name("novalidate"), "noValidate");
        assert_eq!(component_attr_name("formaction"), "formAction");
        assert_eq!(component_attr_name("ismap"), "isMap");
        assert_eq!(component_attr_name("usemap"), "useMap");
    }

    #[test]
    fn test_component_attr_name_strips_colon() {
        assert_eq!(component_attr_name(":readonly"), "readOnly");
        assert_eq!(component_attr_name(":aria-describedby"), "ariaDescribedBy");
        assert_eq!(component_attr_name(":data-title"), "dataTitle");
    }

    #[test]
    fn test_component_attr_name_regular() {
        assert_eq!(component_attr_name("data-title"), "dataTitle");
        assert_eq!(component_attr_name("key-hyphen"), "keyHyphen");
        assert_eq!(component_attr_name("simple"), "simple");
    }

    // ── allowed_query SSR emission tests ─────────────────────────────

    fn make_query_route_protocol() -> WebUIProtocol {
        use webui_protocol::WebUiFragmentRoute;

        let mut fragments = HashMap::new();

        fragments.insert(
            "index.html".to_string(),
            FragmentList {
                fragments: vec![WebUIFragment::route_from(WebUiFragmentRoute {
                    path: "/".into(),
                    fragment_id: "app-shell".into(),
                    exact: false,
                    children: vec![
                        WebUiFragmentRoute {
                            path: "compose".into(),
                            fragment_id: "compose-page".into(),
                            exact: true,
                            allowed_query: "action,to,subject".into(),
                            keep_alive: false,
                            ..Default::default()
                        },
                        WebUiFragmentRoute {
                            path: "settings".into(),
                            fragment_id: "settings-page".into(),
                            exact: true,
                            keep_alive: false,
                            ..Default::default()
                        },
                    ],
                    keep_alive: false,
                    ..Default::default()
                })],
            },
        );

        fragments.insert(
            "app-shell".to_string(),
            FragmentList {
                fragments: vec![WebUIFragment::raw("<h1>App</h1>"), WebUIFragment::outlet()],
            },
        );
        fragments.insert(
            "compose-page".to_string(),
            FragmentList {
                fragments: vec![WebUIFragment::raw("<p>Compose</p>")],
            },
        );
        fragments.insert(
            "settings-page".to_string(),
            FragmentList {
                fragments: vec![WebUIFragment::raw("<p>Settings</p>")],
            },
        );

        WebUIProtocol::new(fragments)
    }

    #[test]
    fn test_matched_route_omits_query_attr_from_dom() {
        let protocol = make_query_route_protocol();
        let state = test_json!({});
        let handler = WebUIHandler::new();
        let mut writer = TestWriter::new();

        handler
            .handle(
                &protocol,
                &state,
                &RenderOptions::new("index.html", "/compose"),
                &mut writer,
            )
            .expect("render failed");

        let html = writer.get_content();
        // query attr is no longer in DOM — it's in the SSR chain JSON instead
        assert!(
            !html.contains(r#"query="action,to,subject""#),
            "query attr should not be in DOM output (moved to SSR chain JSON): {html}"
        );
    }

    #[test]
    fn test_nonmatched_route_omits_query_attr_from_dom() {
        let protocol = make_query_route_protocol();
        let state = test_json!({});
        let handler = WebUIHandler::new();
        let mut writer = TestWriter::new();

        handler
            .handle(
                &protocol,
                &state,
                &RenderOptions::new("index.html", "/settings"),
                &mut writer,
            )
            .expect("render failed");

        let html = writer.get_content();
        // query attr should not be on hidden siblings either
        assert!(
            !html.contains(r#"query="#),
            "hidden route should not have query attr: {html}"
        );
    }

    #[test]
    fn test_route_without_query_has_no_query_attr() {
        let protocol = make_query_route_protocol();
        let state = test_json!({});
        let handler = WebUIHandler::new();
        let mut writer = TestWriter::new();

        handler
            .handle(
                &protocol,
                &state,
                &RenderOptions::new("index.html", "/settings"),
                &mut writer,
            )
            .expect("render failed");

        let html = writer.get_content();
        // Find the settings route element and verify it has no query attr
        let settings_idx = html
            .find(r#"component="settings-page""#)
            .expect("settings route should exist");
        let settings_tag = &html[settings_idx.saturating_sub(60)..settings_idx + 40];
        assert!(
            !settings_tag.contains("query="),
            "route without allowed_query should not emit query attr: {settings_tag}"
        );
    }

    // ── Per-render head_inject / body_inject (replaces the byte-scanner
    //    InjectingStreamingWriter approach with structural signal-based
    //    injection) ───────────────────────────────────────────────────

    fn build_head_body_protocol() -> WebUIProtocol {
        let mut fragments = HashMap::new();
        fragments.insert(
            "index.html".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::raw("<html><head><title>x</title>".to_string()),
                    structural_fragment("head_end"),
                    WebUIFragment::raw("</head><body>hello".to_string()),
                    structural_fragment("body_end"),
                    WebUIFragment::raw("</body></html>".to_string()),
                ],
            },
        );
        WebUIProtocol::new(fragments)
    }

    #[test]
    fn head_inject_emits_at_head_end_boundary() {
        let protocol = build_head_body_protocol();
        let state = test_json!({});
        let mut writer = TestWriter::new();
        let opts = RenderOptions::new("index.html", "/").with_head_inject("<link rel=preload>");
        handle(&protocol, &state, &opts, &mut writer).unwrap();
        let html = writer.get_content();
        // The inject must appear immediately before `</head>`.
        let inject_idx = html
            .find("<link rel=preload>")
            .expect("inject HTML missing");
        let head_close_idx = html.find("</head>").expect("</head> missing");
        assert!(
            inject_idx < head_close_idx,
            "head_inject must appear before </head>: {html}"
        );
        // No duplicate.
        assert_eq!(html.matches("<link rel=preload>").count(), 1);
    }

    #[test]
    fn component_render_policy_css_emits_once_in_head_with_nonce() {
        let mut protocol = build_head_body_protocol();
        protocol.component_render_css = concat!(
            r#"lazy-card:not([w-render="eager"]){content-visibility:auto;"#,
            "contain-intrinsic-block-size:auto 18rem;}"
        )
        .to_string();
        let state = test_json!({});
        let mut writer = TestWriter::new();
        let opts = RenderOptions::new("index.html", "/").with_nonce("test-nonce");
        handle(&protocol, &state, &opts, &mut writer).unwrap();
        let html = writer.get_content();

        let style = concat!(
            r#"<style data-webui-render-policy nonce="test-nonce">"#,
            r#"lazy-card:not([w-render="eager"]){content-visibility:auto;"#,
            "contain-intrinsic-block-size:auto 18rem;}</style>"
        );
        let style_index = html.find(style).expect("render policy style missing");
        let head_close = html.find("</head>").expect("</head> missing");
        assert!(style_index < head_close);
        assert_eq!(html.matches("data-webui-render-policy").count(), 1);
    }

    #[test]
    fn body_inject_emits_at_body_end_boundary() {
        let protocol = build_head_body_protocol();
        let state = test_json!({});
        let mut writer = TestWriter::new();
        let opts = RenderOptions::new("index.html", "/").with_body_inject("<script>lr</script>");
        handle(&protocol, &state, &opts, &mut writer).unwrap();
        let html = writer.get_content();
        let inject_idx = html
            .find("<script>lr</script>")
            .expect("inject HTML missing");
        let body_close_idx = html.find("</body>").expect("</body> missing");
        assert!(
            inject_idx < body_close_idx,
            "body_inject must appear before </body>: {html}"
        );
        assert_eq!(html.matches("<script>lr</script>").count(), 1);
    }

    // ── Reserved `$webui` state inject namespace ──────────────────────

    /// Protocol carrying all three structural boundaries, so `body_start`
    /// placement can be asserted alongside `head_end` / `body_end`.
    fn build_all_boundaries_protocol() -> WebUIProtocol {
        let mut fragments = HashMap::new();
        fragments.insert(
            "index.html".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::raw("<html><head><title>x</title>".to_string()),
                    structural_fragment("head_end"),
                    WebUIFragment::raw("</head><body>".to_string()),
                    structural_fragment("body_start"),
                    WebUIFragment::raw("hello".to_string()),
                    structural_fragment("body_end"),
                    WebUIFragment::raw("</body></html>".to_string()),
                ],
            },
        );
        WebUIProtocol::new(fragments)
    }

    fn state_inject_options<'a>() -> RenderOptions<'a> {
        RenderOptions::new("index.html", "/")
    }

    fn render_with(protocol: &WebUIProtocol, state: &Value, options: &RenderOptions<'_>) -> String {
        let mut writer = TestWriter::new();
        handle(protocol, state, options, &mut writer).unwrap();
        writer.get_content().to_string()
    }

    #[test]
    fn state_inject_emits_at_every_structural_boundary() {
        let protocol = build_all_boundaries_protocol();
        let state = test_json!({
            "$webui": {
                "headEnd": "<meta name=he>",
                "bodyStart": "<span id=bs></span>",
                "bodyEnd": "<script>be</script>",
            }
        });
        let html = render_with(&protocol, &state, &state_inject_options());

        let head_end = html.find("<meta name=he>").expect("headEnd missing");
        let head_close = html.find("</head>").expect("</head> missing");
        let body_open = html.find("<body>").expect("<body> missing");
        let body_start = html.find("<span id=bs></span>").expect("bodyStart missing");
        let hello = html.find("hello").expect("body content missing");
        let body_end = html.find("<script>be</script>").expect("bodyEnd missing");
        let body_close = html.find("</body>").expect("</body> missing");

        assert!(
            head_end < head_close,
            "headEnd must precede </head>: {html}"
        );
        assert!(
            body_open < body_start && body_start < hello,
            "bodyStart must sit immediately after <body>: {html}"
        );
        assert!(
            hello < body_end && body_end < body_close,
            "bodyEnd must precede </body>: {html}"
        );

        for needle in [
            "<meta name=he>",
            "<span id=bs></span>",
            "<script>be</script>",
        ] {
            assert_eq!(html.matches(needle).count(), 1, "duplicated {needle}");
        }
    }

    #[test]
    fn state_inject_follows_render_options_inject() {
        let protocol = build_all_boundaries_protocol();
        let state = test_json!({
            "$webui": { "headEnd": "<!--state-he-->", "bodyEnd": "<!--state-be-->" }
        });
        let options = RenderOptions::new("index.html", "/")
            .with_head_inject("<!--opt-he-->")
            .with_body_inject("<!--opt-be-->");
        let html = render_with(&protocol, &state, &options);

        let opt_he = html.find("<!--opt-he-->").expect("option headEnd missing");
        let state_he = html.find("<!--state-he-->").expect("state headEnd missing");
        let opt_be = html.find("<!--opt-be-->").expect("option bodyEnd missing");
        let state_be = html.find("<!--state-be-->").expect("state bodyEnd missing");

        assert!(
            opt_he < state_he,
            "RenderOptions head_inject must precede the state-supplied value: {html}"
        );
        assert!(
            opt_be < state_be,
            "RenderOptions body_inject must precede the state-supplied value: {html}"
        );
    }

    #[test]
    fn malformed_state_inject_values_are_inert() {
        let protocol = build_all_boundaries_protocol();
        // Absent key, wrong container type, and per-member null / empty /
        // non-string values must all render without output and without error.
        for state in [
            test_json!({}),
            test_json!({ "$webui": "not-an-object" }),
            test_json!({ "$webui": [] }),
            test_json!({ "$webui": null }),
            test_json!({ "$webui": { "headEnd": null, "bodyStart": "", "bodyEnd": 42 } }),
            test_json!({ "$webui": { "unknownMember": "<b>x</b>" } }),
        ] {
            let html = render_with(&protocol, &state, &state_inject_options());
            assert!(
                html.contains("<html><head><title>x</title></head><body>hello</body></html>"),
                "malformed reserved state must render the document unchanged: {html}"
            );
            assert!(!html.contains("<b>x</b>"), "unknown member leaked: {html}");
            assert!(!html.contains("42"), "non-string member leaked: {html}");
        }
    }

    #[test]
    fn state_inject_never_reaches_the_hydration_payload() {
        let protocol = build_all_boundaries_protocol();
        let state = test_json!({
            "visible": "keep",
            "$webui": { "bodyEnd": "<script>secret</script>" }
        });
        let handler = WebUIHandler::with_plugin(|| {
            Box::new(crate::plugin::webui::WebUIHydrationPlugin::new())
        });
        let runtime = Protocol::new(protocol.clone());
        let mut writer = TestWriter::new();
        handler
            .render(&runtime, &state, &state_inject_options(), &mut writer)
            .unwrap();
        let html = writer.get_content();

        let data_start = html
            .find(r#"<script type="application/json" id="webui-data""#)
            .expect("hydration block missing");
        let data_end = html[data_start..]
            .find("</script>")
            .map(|offset| data_start + offset)
            .expect("hydration block never closes");
        let payload = &html[data_start..data_end];
        assert!(
            !payload.contains("$webui"),
            "reserved key must be stripped from the hydration payload: {payload}"
        );
        assert!(
            payload.contains("visible"),
            "ordinary state must survive the filter: {payload}"
        );
        // Per the documented precedence the injected HTML is emitted after
        // the built-in hydration block, and still before `</body>`.
        let inject = html
            .find("<script>secret</script>")
            .expect("inject missing");
        let body_close = html.find("</body>").expect("</body> missing");
        assert!(
            data_end < inject && inject < body_close,
            "state bodyEnd must follow the hydration block and precede </body>: {html}"
        );
    }

    #[test]
    fn projected_hydration_strips_reserved_state_inject_key() {
        let mut fragments = HashMap::new();
        fragments.insert(
            "index.html".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::raw("<html><body>"),
                    WebUIFragment::component("app-shell"),
                    structural_fragment("body_end"),
                    WebUIFragment::raw("</body></html>"),
                ],
            },
        );
        fragments.insert(
            "app-shell".to_string(),
            FragmentList {
                fragments: vec![WebUIFragment::raw("<p>Shell</p>")],
            },
        );
        let mut document = WebUIProtocol::new(fragments);
        document.initial_state_strategy = InitialStateStrategy::Components as i32;
        document.components.insert(
            "app-shell".to_string(),
            webui_protocol::ComponentData {
                hydration_mode: StateProjectionMode::Keys as i32,
                hydration_keys: vec![STATE_INJECT_KEY.to_string(), "visible".to_string()],
                ..Default::default()
            },
        );
        let state = test_json!({
            "$webui": { "bodyEnd": "<script>secret</script>" },
            "serverOnly": "drop",
            "visible": "keep",
        });
        let handler = WebUIHandler::with_plugin(|| {
            Box::new(crate::plugin::webui::WebUIHydrationPlugin::new())
        });
        let mut writer = TestWriter::new();

        handler
            .render(
                &Protocol::new(document),
                &state,
                &RenderOptions::new("index.html", "/"),
                &mut writer,
            )
            .unwrap();
        let html = writer.get_content();
        let data_start = html
            .find(r#"<script type="application/json" id="webui-data""#)
            .expect("hydration block missing");
        let data_end = html[data_start..]
            .find("</script>")
            .map(|offset| data_start + offset)
            .expect("hydration block never closes");
        let payload = &html[data_start..data_end];

        assert!(
            !payload.contains(STATE_INJECT_KEY),
            "reserved key leaked: {payload}"
        );
        assert!(payload.contains(r#""visible":"keep""#), "{payload}");
        assert!(!payload.contains("serverOnly"), "{payload}");
        assert!(html.contains("<script>secret</script>"), "{html}");
    }

    #[test]
    fn write_selected_state_strips_reserved_key_from_full_state() {
        let state = test_json!({ "a": 1, "$webui": { "bodyEnd": "<b>x</b>" }, "z": 2 });
        let mut sink = TestWriter::new();
        let mut scratch = Vec::new();
        write_selected_state(&mut sink, &mut scratch, &state, &StateSelection::Full).unwrap();
        let json = sink.get_content();
        assert!(!json.contains("$webui"), "reserved key leaked: {json}");
        assert!(
            json.contains("\"a\":1") && json.contains("\"z\":2"),
            "{json}"
        );
    }

    #[test]
    fn write_selected_state_strips_reserved_key_from_borrowed_projection() {
        let state = test_json!({
            "$webui": { "bodyEnd": "<b>x</b>" },
            "keep": 1,
        });
        let keys = [STATE_INJECT_KEY, "keep", "missing"];
        let mut sink = TestWriter::new();
        let mut scratch = Vec::new();

        write_selected_state(
            &mut sink,
            &mut scratch,
            &state,
            &StateSelection::BorrowedKeys(&keys),
        )
        .unwrap();

        assert_eq!(sink.get_content(), r#"{"keep":1}"#);
    }

    #[test]
    fn write_selected_state_full_is_unchanged_without_reserved_key() {
        let state = test_json!({ "a": 1, "z": 2 });
        let mut sink = TestWriter::new();
        let mut scratch = Vec::new();
        write_selected_state(&mut sink, &mut scratch, &state, &StateSelection::Full).unwrap();
        assert_eq!(sink.get_content(), r#"{"a":1,"z":2}"#);
    }

    #[test]
    fn body_start_hook_dedupes_on_malformed_protocol() {
        let mut fragments = HashMap::new();
        fragments.insert(
            "index.html".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::raw("<html><head></head><body>".to_string()),
                    structural_fragment("body_start"),
                    structural_fragment("body_start"),
                    WebUIFragment::raw("</body></html>".to_string()),
                ],
            },
        );
        let protocol = WebUIProtocol::new(fragments);
        let state = test_json!({ "$webui": { "bodyStart": "<i>once</i>" } });
        let html = render_with(&protocol, &state, &state_inject_options());
        assert_eq!(
            html.matches("<i>once</i>").count(),
            1,
            "duplicate body_start signals must not duplicate the inject: {html}"
        );
    }

    #[test]
    fn injects_are_no_op_when_unset() {
        let protocol = build_head_body_protocol();
        let state = test_json!({});
        let mut writer = TestWriter::new();
        handle(
            &protocol,
            &state,
            &RenderOptions::new("index.html", "/"),
            &mut writer,
        )
        .unwrap();
        let html = writer.get_content();
        assert!(!html.contains("<link rel=preload>"));
        assert!(!html.contains("<script>lr</script>"));
    }

    #[test]
    fn empty_inject_string_treated_as_unset() {
        let protocol = build_head_body_protocol();
        let state = test_json!({});
        let mut writer = TestWriter::new();
        let opts = RenderOptions::new("index.html", "/")
            .with_head_inject("")
            .with_body_inject("");
        handle(&protocol, &state, &opts, &mut writer).unwrap();
        // No injection happens — empty strings are normalised to None
        // by the builder, so the output is identical to the no-options case.
        let html = writer.get_content();
        assert!(html.contains("</head>"));
        assert!(html.contains("</body>"));
    }

    #[test]
    fn inject_html_is_passed_through_verbatim() {
        // The handler does NOT escape the inject string — hosts pass
        // raw HTML they trust. This test pins that contract: a `<` in
        // the inject is emitted as-is, not encoded as `&lt;`.
        let protocol = build_head_body_protocol();
        let state = test_json!({});
        let mut writer = TestWriter::new();
        let opts =
            RenderOptions::new("index.html", "/").with_body_inject("<script>var x=1;</script>");
        handle(&protocol, &state, &opts, &mut writer).unwrap();
        assert!(writer.get_content().contains("<script>var x=1;</script>"));
    }

    /// Both injects fire and appear at the correct structural
    /// positions. Critically, this is robust against `</head>` /
    /// `</body>` literals appearing elsewhere in the document — the
    /// signal-based emitter cannot mis-fire on byte patterns inside
    /// HTML comments, `<iframe srcdoc>`, or inline scripts (which the
    /// previous byte-scanner could).
    #[test]
    fn injects_robust_against_marker_literals_in_content() {
        let mut fragments = HashMap::new();
        // The body intentionally contains `</body>` and `</head>`
        // literals before the actual structural close — these came
        // from a (hypothetical) iframe srcdoc or comment.
        fragments.insert(
            "index.html".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::raw("<html><head><title>x</title>".to_string()),
                    structural_fragment("head_end"),
                    WebUIFragment::raw(
                        "</head><body><!-- </body> </head> --><p>hi</p>".to_string(),
                    ),
                    structural_fragment("body_end"),
                    WebUIFragment::raw("</body></html>".to_string()),
                ],
            },
        );
        let protocol = WebUIProtocol::new(fragments);
        let state = test_json!({});
        let mut writer = TestWriter::new();
        let opts = RenderOptions::new("index.html", "/")
            .with_head_inject("<HEAD-INJ>")
            .with_body_inject("<BODY-INJ>");
        handle(&protocol, &state, &opts, &mut writer).unwrap();
        let html = writer.get_content();
        // The head inject sits between `<title>x</title>` and the
        // first `</head>` — the structural one, not the comment one.
        let head_inj_idx = html.find("<HEAD-INJ>").expect("head inject missing");
        let head_close_idx = html.find("</head>").expect("</head> missing");
        assert!(head_inj_idx < head_close_idx);
        // The body inject sits before the structural `</body>` — NOT
        // before the `</body>` literal in the comment (which would
        // require the inject to appear inside `<p>hi</p>` somewhere).
        let body_inj_idx = html.find("<BODY-INJ>").expect("body inject missing");
        // Find the LAST `</body>` (the structural one).
        let body_close_idx = html.rfind("</body>").expect("</body> missing");
        assert!(
            body_inj_idx < body_close_idx,
            "body_inject must precede the structural </body>: {html}"
        );
        // And the comment is preserved verbatim.
        assert!(html.contains("<!-- </body> </head> -->"));
    }

    /// Coverage-14: both `head_inject` AND `body_inject` set in the
    /// same render. Each fires at the correct structural boundary and
    /// neither leaks into the other's region.
    #[test]
    fn both_injects_fire_at_correct_boundaries() {
        let protocol = build_head_body_protocol();
        let state = test_json!({});
        let mut writer = TestWriter::new();
        let opts = RenderOptions::new("index.html", "/")
            .with_head_inject("<META-HEAD>")
            .with_body_inject("<SCRIPT-BODY>");
        handle(&protocol, &state, &opts, &mut writer).unwrap();
        let html = writer.get_content();
        let head_idx = html.find("<META-HEAD>").expect("head inject missing");
        let head_close = html.find("</head>").expect("</head> missing");
        let body_idx = html.find("<SCRIPT-BODY>").expect("body inject missing");
        let body_close = html.find("</body>").expect("</body> missing");
        assert!(head_idx < head_close, "head_inject before </head>");
        assert!(head_close < body_idx, "body_inject after </head>");
        assert!(body_idx < body_close, "body_inject before </body>");
        assert_eq!(html.matches("<META-HEAD>").count(), 1);
        assert_eq!(html.matches("<SCRIPT-BODY>").count(), 1);
    }

    /// Coverage-15 / Bug-3 (security defense): a malformed protocol
    /// emitting `head_end` and `body_end` more than once must NOT
    /// duplicate the host inject HTML. Without the dedup guard,
    /// double-emission would amplify Security-2 (a 1 MiB inject ×
    /// 1000 duplicate signals = 1 GiB output).
    #[test]
    fn injects_dedupe_against_duplicate_signals() {
        let mut fragments = HashMap::new();
        fragments.insert(
            "index.html".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::raw("<html><head>".to_string()),
                    structural_fragment("head_end"),
                    structural_fragment("head_end"), // duplicate
                    structural_fragment("head_end"), // triplicate
                    WebUIFragment::raw("</head><body>".to_string()),
                    structural_fragment("body_end"),
                    structural_fragment("body_end"), // duplicate
                    WebUIFragment::raw("</body></html>".to_string()),
                ],
            },
        );
        let protocol = WebUIProtocol::new(fragments);
        let state = test_json!({});
        let mut writer = TestWriter::new();
        let opts = RenderOptions::new("index.html", "/")
            .with_head_inject("<HINJ>")
            .with_body_inject("<BINJ>");
        handle(&protocol, &state, &opts, &mut writer).unwrap();
        let html = writer.get_content();
        assert_eq!(
            html.matches("<HINJ>").count(),
            1,
            "head_inject must emit exactly once even with duplicate head_end signals"
        );
        assert_eq!(
            html.matches("<BINJ>").count(),
            1,
            "body_inject must emit exactly once even with duplicate body_end signals"
        );
    }

    /// Coverage-15: a Shadow-DOM / component-only protocol that has NO
    /// `<head>` / `<body>` tags must NOT emit the inject (the signals
    /// never fire). Verifies the injects are no-ops, not panics.
    #[test]
    fn injects_no_op_when_no_head_or_body_signals() {
        let mut fragments = HashMap::new();
        fragments.insert(
            "index.html".to_string(),
            FragmentList {
                fragments: vec![WebUIFragment::raw(
                    "<my-component>hi</my-component>".to_string(),
                )],
            },
        );
        let protocol = WebUIProtocol::new(fragments);
        let state = test_json!({});
        let mut writer = TestWriter::new();
        let opts = RenderOptions::new("index.html", "/")
            .with_head_inject("<HINJ>")
            .with_body_inject("<BINJ>");
        handle(&protocol, &state, &opts, &mut writer).unwrap();
        let html = writer.get_content();
        assert!(!html.contains("<HINJ>"), "head_inject must not appear");
        assert!(!html.contains("<BINJ>"), "body_inject must not appear");
        assert!(html.contains("<my-component>"));
    }

    /// Coverage-19: the handler's `&self` is shared across threads.
    /// Two concurrent renders with different inject values must NOT
    /// cross-contaminate (each thread sees only its own inject).
    /// Per-render mutable state lives on the `WebUIProcessContext`,
    /// which is stack-allocated per call.
    #[test]
    fn concurrent_renders_with_different_injects_do_not_cross_contaminate() {
        let protocol = std::sync::Arc::new(build_head_body_protocol());
        let state = std::sync::Arc::new(test_json!({}));
        let handler = std::sync::Arc::new(WebUIHandler::new());

        const N_THREADS: usize = 16;
        let mut handles = Vec::with_capacity(N_THREADS);
        for tid in 0..N_THREADS {
            let h = std::sync::Arc::clone(&handler);
            let p = std::sync::Arc::clone(&protocol);
            let s = std::sync::Arc::clone(&state);
            handles.push(std::thread::spawn(move || {
                let head = format!("<HEAD-T{tid}>");
                let body = format!("<BODY-T{tid}>");
                let mut writer = TestWriter::new();
                let opts = RenderOptions::new("index.html", "/")
                    .with_head_inject(&head)
                    .with_body_inject(&body);
                h.handle(&p, &s, &opts, &mut writer).unwrap();
                let html = writer.get_content();
                // Must contain my own injects exactly once.
                assert_eq!(html.matches(&head).count(), 1);
                assert_eq!(html.matches(&body).count(), 1);
                // Must NOT contain any other thread's inject.
                for other in 0..N_THREADS {
                    if other == tid {
                        continue;
                    }
                    let other_head = format!("<HEAD-T{other}>");
                    let other_body = format!("<BODY-T{other}>");
                    assert!(
                        !html.contains(&other_head),
                        "tid {tid} saw {other}'s head_inject"
                    );
                    assert!(
                        !html.contains(&other_body),
                        "tid {tid} saw {other}'s body_inject"
                    );
                }
            }));
        }
        for h in handles {
            h.join().expect("worker panicked");
        }
    }

    /// Coverage-17: a large (1 MiB) head_inject must round-trip
    /// correctly without panic, truncation, or excessive overhead.
    /// (No size cap is enforced by the handler — the host owns the
    /// safety contract; see `with_head_inject` doc comment.)
    #[test]
    fn large_inject_roundtrips_without_truncation() {
        let protocol = build_head_body_protocol();
        let state = test_json!({});
        let mut writer = TestWriter::new();
        let big = "x".repeat(1024 * 1024);
        let opts = RenderOptions::new("index.html", "/").with_head_inject(&big);
        handle(&protocol, &state, &opts, &mut writer).unwrap();
        let html = writer.get_content();
        assert!(
            html.contains(&big),
            "large head_inject must be present verbatim ({} bytes)",
            big.len()
        );
        // Sanity: only one copy.
        assert_eq!(html.matches(&big).count(), 1);
    }

    /// `with_nonce("")` must normalize to `None` (no `<meta>` emitted),
    /// matching the empty-string semantics of `with_head_inject` /
    /// `with_body_inject`. An empty content attribute is browser-
    /// ignored noise.
    #[test]
    fn empty_nonce_treated_as_unset() {
        let protocol = build_head_body_protocol();
        let state = test_json!({});
        let mut writer = TestWriter::new();
        let opts = RenderOptions::new("index.html", "/").with_nonce("");
        handle(&protocol, &state, &opts, &mut writer).unwrap();
        assert!(
            !writer.get_content().contains("webui-nonce"),
            "empty nonce must not emit <meta name=\"webui-nonce\">"
        );
    }

    /// Regression for the bug Akrosh caught: the `pub` fields on
    /// `RenderOptions` let a caller bypass the `with_*` builder
    /// normalisation, e.g.:
    ///
    /// ```ignore
    /// RenderOptions { nonce: Some(""), ..RenderOptions::new(e, p) }
    /// ```
    ///
    /// Without defensive normalisation at handler init, this would
    /// emit `<script nonce="">` on every inline script. Under a
    /// strict `Content-Security-Policy: script-src 'nonce-...'` an
    /// empty nonce is a HARD CSP failure that blocks every inline
    /// script — a complete inline-script-execution outage.
    ///
    /// The handler now treats `Some("")` identically to `None` for
    /// all three injection points (nonce / head_inject / body_inject)
    /// regardless of how the option was populated.
    #[test]
    fn empty_field_bypass_is_normalised_at_handler_init() {
        let protocol = build_head_body_protocol();
        let state = test_json!({});

        // Bypass the `with_nonce` builder by writing the field directly.
        let opts_with_empty_nonce = RenderOptions {
            nonce: Some(""),
            ..RenderOptions::new("index.html", "/")
        };
        let mut writer = TestWriter::new();
        handle(&protocol, &state, &opts_with_empty_nonce, &mut writer).unwrap();
        let html = writer.get_content();
        assert!(
            !html.contains("webui-nonce"),
            "field-bypass empty nonce must not emit `<meta name=\"webui-nonce\">`"
        );
        assert!(
            !html.contains("nonce=\"\""),
            "field-bypass empty nonce must not emit `nonce=\"\"` (would be a hard CSP failure)"
        );

        // Same defence for inject fields.
        let opts_with_empty_injects = RenderOptions {
            head_inject: Some(""),
            body_inject: Some(""),
            ..RenderOptions::new("index.html", "/")
        };
        let mut writer = TestWriter::new();
        handle(&protocol, &state, &opts_with_empty_injects, &mut writer).unwrap();
        // No assertion needed beyond "doesn't panic and doesn't emit
        // empty inject markers" — the head_end / body_end paths must
        // treat the empty inject as no-op the same way the builder does.
    }

    /// Regression for the deep-audit's Bug-6 claim. The for-loop hot-
    /// path optimisation (insert key once + `get_mut`-swap value
    /// in-place) was suspected of corrupting the outer scope when a
    /// nested `<for>` loop reuses the same variable name. This test
    /// proves the optimisation is correct under that condition by
    /// requiring the outer `item` to be visible before, between, and
    /// after the inner loop, with its value preserved across inner
    /// iterations.
    ///
    /// Trace through the optimisation on `outer = [A, B]`,
    /// `inner = [X, Y]` with both loops using `item` as the variable:
    ///
    ///   outer pre-insert "item": Null
    ///   iter 1: get_mut → write A
    ///     emit "outer:A"
    ///     enter inner: saved = remove("item") = Some(A)
    ///                  pre-insert "item": Null
    ///                  iter 1: write X → emit "inner:X"
    ///                  iter 2: write Y → emit "inner:Y"
    ///                  restore: insert("item", A)   ← outer's A back
    ///     emit "outer:A again"               ← reads A correctly
    ///   iter 2: get_mut → write B (overwrites the restored A,
    ///                              but that's correct — we're now
    ///                              in iter 2 of the outer loop)
    ///     emit "outer:B"
    ///     enter inner: saved = remove("item") = Some(B), …, restore B
    ///     emit "outer:B again"
    ///
    /// If the audit's claim were correct — that the outer's `get_mut`
    /// somehow held a reference past the inner loop and clobbered the
    /// restoration — we'd see corrupted values in the "outer:X again"
    /// emissions. The assertion below pins the correct sequence.
    #[test]
    fn nested_for_loops_reusing_same_variable_name_dont_corrupt_scope() {
        let mut fragments = HashMap::new();
        fragments.insert(
            "index.html".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::raw("["),
                    WebUIFragment::for_loop("item", "outer", "outer_body"),
                    WebUIFragment::raw("]"),
                ],
            },
        );
        fragments.insert(
            "outer_body".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::raw("(O="),
                    WebUIFragment::signal("item.tag", false),
                    WebUIFragment::for_loop("item", "inner", "inner_body"),
                    WebUIFragment::raw(",O="),
                    WebUIFragment::signal("item.tag", false),
                    WebUIFragment::raw(")"),
                ],
            },
        );
        fragments.insert(
            "inner_body".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::raw("[I="),
                    WebUIFragment::signal("item.tag", false),
                    WebUIFragment::raw("]"),
                ],
            },
        );
        let protocol = WebUIProtocol::new(fragments);
        let state = test_json!({
            "outer": [{"tag": "A"}, {"tag": "B"}],
            "inner": [{"tag": "X"}, {"tag": "Y"}],
        });
        let mut writer = TestWriter::new();
        handle(
            &protocol,
            &state,
            &RenderOptions::new("index.html", "/"),
            &mut writer,
        )
        .unwrap();
        // Expected sequence:
        //   outer iter 1 (item=A):
        //     emit "(O=A"               ← outer A before inner
        //     inner iter 1 (item=X) emit "[I=X]"
        //     inner iter 2 (item=Y) emit "[I=Y]"
        //     emit ",O=A)"              ← outer A AFTER inner restore
        //   outer iter 2 (item=B):
        //     emit "(O=B"
        //     inner iter 1 (item=X) emit "[I=X]"
        //     inner iter 2 (item=Y) emit "[I=Y]"
        //     emit ",O=B)"
        assert_eq!(
            writer.get_content(),
            "[(O=A[I=X][I=Y],O=A)(O=B[I=X][I=Y],O=B)]",
            "outer `item` must stay bound to its iteration value across the inner loop's save/restore"
        );
    }

    #[derive(Default)]
    struct FlushTestWriter {
        output: String,
        flushes: Vec<usize>,
        fail_flush: bool,
        fail_flush_at: Option<usize>,
        flush_attempts: usize,
        ended: bool,
    }

    impl ResponseWriter for FlushTestWriter {
        fn write(&mut self, content: &str) -> Result<()> {
            self.output.push_str(content);
            Ok(())
        }

        fn end(&mut self) -> Result<()> {
            self.ended = true;
            Ok(())
        }
    }

    impl FlushWriter for FlushTestWriter {
        fn flush(&mut self) -> Result<()> {
            let attempt = self.flush_attempts;
            self.flush_attempts += 1;
            if self.fail_flush || self.fail_flush_at == Some(attempt) {
                return Err(HandlerError::ClientDisconnected);
            }
            self.flushes.push(self.output.len());
            Ok(())
        }
    }

    fn streaming_protocol(with_boundaries: bool) -> Protocol {
        streaming_protocol_with_state_strategy(with_boundaries, InitialStateStrategy::Components)
    }

    fn streaming_protocol_with_state_strategy(
        with_boundaries: bool,
        state_strategy: InitialStateStrategy,
    ) -> Protocol {
        let mut fragments = HashMap::new();
        let mut entry = vec![
            WebUIFragment::raw("<!DOCTYPE html><html><HEAD data-shell=\"main\">"),
            structural_fragment("head_start"),
            WebUIFragment::raw("<script type=\"module\" async src=\"/index.js\"></script>"),
            structural_fragment("head_end"),
            WebUIFragment::raw("</HEAD><body>"),
            structural_fragment("body_start"),
        ];
        if with_boundaries {
            entry.push(structural_fragment("boundary_start:0"));
        }
        entry.extend([
            WebUIFragment::raw("<my-counter"),
            structural_fragment("streaming_root:my-counter"),
            WebUIFragment::raw(">"),
            WebUIFragment::component("my-counter"),
            WebUIFragment::raw("</my-counter>"),
        ]);
        if with_boundaries {
            entry.push(structural_fragment("boundary_end:0"));
        }
        entry.extend([
            WebUIFragment::raw("<app-footer>slow tail</app-footer>"),
            structural_fragment("body_end"),
            WebUIFragment::raw("</body></html>"),
        ]);
        fragments.insert("index.html".to_string(), FragmentList { fragments: entry });
        fragments.insert(
            "my-counter".to_string(),
            FragmentList {
                fragments: vec![WebUIFragment::raw("<button>Count</button>")],
            },
        );

        let mut document = WebUIProtocol::new(fragments);
        document.initial_state_strategy = state_strategy as i32;
        document.components.insert(
            "my-counter".to_string(),
            webui_protocol::ComponentData {
                template_json: r#"{"h":"<button>Count</button>","th":1}"#.to_string(),
                template_functions: "[function(){return true}]".to_string(),
                hydration_mode: StateProjectionMode::Keys as i32,
                hydration_keys: vec!["count".to_string()],
                ..Default::default()
            },
        );
        Protocol::new(document)
    }

    #[test]
    fn handler_error_stays_small() {
        // Boxing the cold `StreamingBoundary` payload keeps `HandlerError` — and
        // therefore `Result<(), HandlerError>` threaded through the hot legacy
        // render path — down to a single `String`-sized payload plus a
        // discriminant word. If the boundary payload is un-boxed back to
        // `{ signal, reason }` it grows to two `String`s (48-byte payload) and
        // this fails.
        assert!(
            std::mem::size_of::<HandlerError>()
                <= std::mem::size_of::<String>() + std::mem::size_of::<usize>(),
            "HandlerError grew to {} bytes",
            std::mem::size_of::<HandlerError>()
        );
    }

    #[test]
    fn streaming_render_flushes_bootstrap_before_slow_tail_and_emits_terminal() {
        let protocol = streaming_protocol(true);
        let handler = WebUIHandler::with_plugin(|| {
            Box::new(crate::plugin::webui::WebUIHydrationPlugin::new())
        });
        let state = test_json!({ "count": 1, "serverOnly": "secret" });
        let mut writer = FlushTestWriter::default();

        handler
            .render_streaming(
                &protocol,
                &state,
                &RenderOptions::new("index.html", "/"),
                &mut writer,
            )
            .unwrap();

        assert!(writer.ended);
        assert_eq!(
            writer.flushes.len(),
            2,
            "boundary commit plus one coalesced terminal-tail flush"
        );
        let first_flush = &writer.output[..writer.flushes[0]];
        assert!(first_flush.contains("<!--wb:0-->"));
        assert!(first_flush.contains("<!--/wb:0-->"));
        assert!(first_flush.contains(r#"[2,0,0,0,{"componentStyles":"#));
        assert!(first_flush.contains(r#""inventory":"01","state":{"count":1}"#));
        assert!(first_flush.contains(r#""templates":{"my-counter":"#));
        assert!(!first_flush.contains("slow tail"));
        assert!(!writer.output.contains("id=\"webui-data\""));
        // The terminal flush commits the scriptless tail without manufacturing
        // another state/template projection.
        assert!(writer.output.contains("[2,1,3,0,{}]"));
        assert!(!writer.output.contains("[2,1,0,1,"));
        assert!(!writer.output.contains("[2,2,3,0,{}]"));

        let marker = writer
            .output
            .find(STREAMING_MARKER)
            .expect("streaming marker");
        let attributed_head = writer
            .output
            .find("<HEAD data-shell=\"main\">")
            .expect("attributed mixed-case head");
        let authored_script = writer
            .output
            .find("src=\"/index.js\"")
            .expect("entry script");
        assert!(attributed_head < marker && marker < authored_script);
    }

    #[test]
    fn streaming_terminal_tail_never_resends_full_state() {
        let protocol = streaming_protocol_with_state_strategy(true, InitialStateStrategy::Full);
        let handler = WebUIHandler::with_plugin(|| {
            Box::new(crate::plugin::webui::WebUIHydrationPlugin::new())
        });
        let mut writer = FlushTestWriter::default();

        handler
            .render_streaming(
                &protocol,
                &test_json!({ "count": 1, "serverOnly": "secret" }),
                &RenderOptions::new("index.html", "/").with_body_inject(" \n"),
                &mut writer,
            )
            .unwrap();

        assert_eq!(
            writer.output.matches("serverOnly").count(),
            1,
            "full state belongs only to the interactive boundary"
        );
        assert!(writer.output.contains("[2,1,3,0,{}]"));
    }

    /// Streaming must place the reserved-state injects exactly where the
    /// ordinary render does, so a host can switch modes without its
    /// boundary HTML moving.
    #[test]
    fn state_inject_placement_matches_between_render_modes() {
        let entry = vec![
            WebUIFragment::raw("<html><head>"),
            structural_fragment("head_start"),
            structural_fragment("head_end"),
            WebUIFragment::raw("</head><body>"),
            structural_fragment("body_start"),
            WebUIFragment::raw("<main>static</main>"),
            structural_fragment("body_end"),
            WebUIFragment::raw("</body></html>"),
        ];
        let fragments =
            HashMap::from([("index.html".to_string(), FragmentList { fragments: entry })]);
        let protocol = Protocol::new(WebUIProtocol::new(fragments));
        let state = test_json!({
            "$webui": {
                "headEnd": "<meta name=he>",
                "bodyStart": "<span id=bs></span>",
                "bodyEnd": "<script>be</script>",
            }
        });
        let options = RenderOptions::new("index.html", "/");

        let mut ordinary = TestWriter::new();
        WebUIHandler::new()
            .render(&protocol, &state, &options, &mut ordinary)
            .unwrap();
        let ordinary_html = ordinary.get_content().to_string();

        let mut streamed = FlushTestWriter::default();
        WebUIHandler::new()
            .render_streaming(&protocol, &state, &options, &mut streamed)
            .unwrap();
        let streamed_html = &streamed.output;

        for html in [ordinary_html.as_str(), streamed_html.as_str()] {
            let head_end = html.find("<meta name=he>").expect("headEnd missing");
            let head_close = html.find("</head>").expect("</head> missing");
            let body_start = html.find("<span id=bs></span>").expect("bodyStart missing");
            let main = html.find("<main>static</main>").expect("content missing");
            let body_end = html.find("<script>be</script>").expect("bodyEnd missing");
            let body_close = html.find("</body>").expect("</body> missing");
            assert!(head_end < head_close, "headEnd misplaced: {html}");
            assert!(body_start < main, "bodyStart misplaced: {html}");
            assert!(
                main < body_end && body_end < body_close,
                "bodyEnd misplaced: {html}"
            );
        }

        // The streaming response still terminates with its single empty
        // terminal record: an inject must not perturb the record stream.
        assert!(
            streamed_html.contains(",3,0,{}]"),
            "streaming must still end in one empty terminal record: {streamed_html}"
        );
    }

    #[test]
    fn streaming_state_inject_emits_and_strips_reserved_key() {
        let entry = vec![
            WebUIFragment::raw("<html><head>"),
            structural_fragment("head_start"),
            structural_fragment("head_end"),
            WebUIFragment::raw("</head><body>"),
            structural_fragment("body_start"),
            structural_fragment("body_end"),
            WebUIFragment::raw("</body></html>"),
        ];
        let fragments =
            HashMap::from([("index.html".to_string(), FragmentList { fragments: entry })]);
        let protocol = Protocol::new(WebUIProtocol::new(fragments));
        let state = test_json!({ "$webui": { "bodyEnd": "<script>be</script>" } });

        let mut writer = FlushTestWriter::default();
        WebUIHandler::new()
            .render_streaming(
                &protocol,
                &state,
                &RenderOptions::new("index.html", "/"),
                &mut writer,
            )
            .unwrap();
        assert!(writer.output.contains("<script>be</script>"));
        assert!(!writer.output.contains("$webui"));
    }

    #[test]
    fn static_streaming_document_uses_one_empty_terminal_record() {
        let fragments = HashMap::from([(
            "index.html".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::raw("<html><head>"),
                    structural_fragment("head_start"),
                    structural_fragment("head_end"),
                    WebUIFragment::raw("</head><body>"),
                    structural_fragment("body_start"),
                    WebUIFragment::raw("<main>static</main>"),
                    structural_fragment("body_end"),
                    WebUIFragment::raw("</body></html>"),
                ],
            },
        )]);
        let protocol = Protocol::new(WebUIProtocol::new(fragments));
        let mut writer = FlushTestWriter::default();

        WebUIHandler::new()
            .render_streaming(
                &protocol,
                &test_json!({ "serverOnly": "secret" }),
                &RenderOptions::new("index.html", "/"),
                &mut writer,
            )
            .unwrap();

        assert_eq!(writer.flushes.len(), 1);
        assert!(writer.output.contains(STREAMING_MARKER));
        assert!(writer.output.contains("[2,0,3,0,{}]"));
        assert!(!writer.output.contains("serverOnly"));
        assert!(!writer.output.contains("id=\"webui-data\""));
        assert!(!writer.output.contains("<!--wb:"));
    }

    #[test]
    fn streaming_render_emits_nonce_and_functions_before_sentinel() {
        let protocol = streaming_protocol(true);
        let handler = WebUIHandler::with_plugin(|| {
            Box::new(crate::plugin::webui::WebUIHydrationPlugin::new())
        });
        let mut writer = FlushTestWriter::default();
        handler
            .render_streaming(
                &protocol,
                &test_json!({ "count": 1 }),
                &RenderOptions::new("index.html", "/").with_nonce("test-nonce-123"),
                &mut writer,
            )
            .unwrap();

        let boundary_end = writer.output.find("<!--/wb:0-->").expect("end marker");
        let envelope = writer.output[boundary_end..]
            .find("data-webui-boundary nonce=\"test-nonce-123\"")
            .map(|index| index + boundary_end)
            .expect("nonce-bearing envelope");
        let functions = writer
            .output
            .find("templateFns")
            .expect("function side channel");
        let sentinel = writer
            .output
            .find("<webui-hydrate>")
            .expect("hydration sentinel");
        assert!(boundary_end < envelope && envelope < functions && functions < sentinel);
        assert!(writer.output.contains("<script nonce=\"test-nonce-123\">"));
    }

    #[test]
    fn streaming_render_rejects_out_of_order_and_nested_boundaries() {
        fn protocol(signals: &[&str]) -> Protocol {
            let mut protocol_signals = Vec::with_capacity(signals.len() + 1);
            protocol_signals.push(structural_fragment("head_start"));
            protocol_signals.extend(signals.iter().map(|signal| structural_fragment(*signal)));
            let fragments = HashMap::from([(
                "index.html".to_string(),
                FragmentList {
                    fragments: protocol_signals,
                },
            )]);
            Protocol::new(WebUIProtocol::new(fragments))
        }

        let handler = WebUIHandler::new();
        let mut writer = FlushTestWriter::default();
        let result = handler.render_streaming(
            &protocol(&["boundary_start:1"]),
            &test_json!({}),
            &RenderOptions::new("index.html", "/"),
            &mut writer,
        );
        match result {
            Err(HandlerError::StreamingBoundary(err)) => {
                assert_eq!(err.signal, "boundary_start:1");
                assert!(err.reason.contains("expected boundary sequence 0"));
            }
            other => panic!("expected ordered-boundary error, got {other:?}"),
        }

        let mut writer = FlushTestWriter::default();
        let result = handler.render_streaming(
            &protocol(&["boundary_start:0", "boundary_start:1"]),
            &test_json!({}),
            &RenderOptions::new("index.html", "/"),
            &mut writer,
        );
        assert!(matches!(result, Err(HandlerError::StreamingBoundary(_))));
    }

    #[test]
    fn streaming_render_propagates_boundary_flush_disconnect() {
        let protocol = streaming_protocol(true);
        let handler = WebUIHandler::new();
        let mut writer = FlushTestWriter {
            fail_flush: true,
            ..FlushTestWriter::default()
        };
        let result = handler.render_streaming(
            &protocol,
            &test_json!({}),
            &RenderOptions::new("index.html", "/"),
            &mut writer,
        );
        assert!(matches!(result, Err(HandlerError::ClientDisconnected)));
        assert!(!writer.ended);
    }

    #[test]
    fn streaming_render_requires_structural_head_start_before_writing() {
        let fragments = HashMap::from([(
            "index.html".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::raw(
                        "<html><HEAD data-shell=\"main\"><script src=\"/early.js\"></script>",
                    ),
                    structural_fragment("head_end"),
                    WebUIFragment::raw("</HEAD><body>"),
                    structural_fragment("body_start"),
                    structural_fragment("body_end"),
                ],
            },
        )]);
        let protocol = Protocol::new(WebUIProtocol::new(fragments));
        let mut writer = FlushTestWriter::default();

        let result = WebUIHandler::new().render_streaming(
            &protocol,
            &test_json!({}),
            &RenderOptions::new("index.html", "/"),
            &mut writer,
        );

        assert!(matches!(
            result,
            Err(HandlerError::MissingStreamingHeadStart { before: "head_end" })
        ));
        assert!(
            writer.output.is_empty(),
            "preflight must fail before output"
        );
        assert!(writer.flushes.is_empty());
        assert!(!writer.ended);
    }

    #[test]
    fn streaming_render_rejects_duplicate_head_start() {
        let fragments = HashMap::from([(
            "index.html".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::raw("<html><head>"),
                    structural_fragment("head_start"),
                    structural_fragment("head_start"),
                    structural_fragment("head_end"),
                    WebUIFragment::raw("</head><body>"),
                    structural_fragment("body_start"),
                    structural_fragment("body_end"),
                ],
            },
        )]);
        let protocol = Protocol::new(WebUIProtocol::new(fragments));
        let mut writer = FlushTestWriter::default();

        let result = WebUIHandler::new().render_streaming(
            &protocol,
            &test_json!({}),
            &RenderOptions::new("index.html", "/"),
            &mut writer,
        );

        assert!(matches!(
            result,
            Err(HandlerError::DuplicateStreamingHeadStart)
        ));
        assert_eq!(writer.output.matches(STREAMING_MARKER).count(), 1);
        assert!(writer.flushes.is_empty());
    }

    #[test]
    fn legacy_render_is_byte_identical_when_boundary_signals_are_present() {
        let handler = WebUIHandler::with_plugin(|| {
            Box::new(crate::plugin::webui::WebUIHydrationPlugin::new())
        });
        let state = test_json!({
            "count": 1,
            "head_start": "must not render",
            "boundary_start:0": "must not render",
            "boundary_end:0": "must not render",
        });
        let mut with_boundaries = TestWriter::new();
        let mut without_boundaries = TestWriter::new();
        handler
            .render(
                &streaming_protocol(true),
                &state,
                &RenderOptions::new("index.html", "/"),
                &mut with_boundaries,
            )
            .unwrap();
        handler
            .render(
                &streaming_protocol(false),
                &state,
                &RenderOptions::new("index.html", "/"),
                &mut without_boundaries,
            )
            .unwrap();
        assert_eq!(
            with_boundaries.get_content(),
            without_boundaries.get_content()
        );
    }

    #[test]
    fn parser_head_attributes_preserve_legacy_ordinary_render_bytes() {
        for source in [
            r#"<html><head data-theme="dark"><title>T</title></head><body>x</body></html>"#,
            r#"<html><head data-theme="{{theme}}"><title>T</title></head><body>x</body></html>"#,
        ] {
            let mut parser = HtmlParser::new();
            parser
                .parse("index.html", source)
                .expect("parse head fixture");
            let protocol = Protocol::new(WebUIProtocol::new(parser.into_fragment_records()));
            let mut writer = TestWriter::new();
            WebUIHandler::new()
                .render(
                    &protocol,
                    &test_json!({ "theme": "light" }),
                    &RenderOptions::new("index.html", "/"),
                    &mut writer,
                )
                .unwrap();

            assert_eq!(
                writer.get_content(),
                "<html><head><title>T</title></head><body>x</body></html>"
            );
        }
    }

    #[test]
    fn mixed_case_native_tags_preserve_ordinary_bytes_and_stream_structurally() {
        let source =
            r#"<html><HEAD data-theme="dark"><title>T</title></HEAD><BODY>x</BODY></html>"#;
        let mut parser = HtmlParser::new();
        parser.parse("index.html", source).expect("parse fixture");
        let protocol = Protocol::new(WebUIProtocol::new(parser.into_fragment_records()));

        let mut ordinary = TestWriter::new();
        WebUIHandler::new()
            .render(
                &protocol,
                &test_json!({}),
                &RenderOptions::new("index.html", "/"),
                &mut ordinary,
            )
            .unwrap();
        assert_eq!(ordinary.get_content(), source);

        let mut streaming = FlushTestWriter::default();
        WebUIHandler::new()
            .render_streaming(
                &protocol,
                &test_json!({}),
                &RenderOptions::new("index.html", "/"),
                &mut streaming,
            )
            .unwrap();
        assert!(streaming.output.contains(r#"<HEAD data-theme="dark">"#));
        assert!(streaming.output.contains("</HEAD><BODY>x"));
        let opening = streaming.output.find("<HEAD").expect("mixed-case head");
        let marker = streaming.output.find(STREAMING_MARKER).expect("marker");
        let title = streaming.output.find("<title>").expect("title");
        assert!(opening < marker && marker < title);
        assert_eq!(streaming.output.matches("data-webui-boundary").count(), 1);
    }

    /// Only `}}}webui:`-namespaced signals are compiler-owned, so an
    /// unprefixed `body_end` is ordinary authored content.
    #[test]
    fn unnamespaced_signal_is_ordinary_content() {
        let fragments = HashMap::from([(
            "index.html".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::raw("<html><head>"),
                    structural_fragment("head_end"),
                    WebUIFragment::raw("</head><body>"),
                    WebUIFragment::signal("body_end".to_string(), false),
                    structural_fragment("body_end"),
                    WebUIFragment::raw("</body></html>"),
                ],
            },
        )]);
        let protocol = Protocol::new(WebUIProtocol::new(fragments));
        let mut writer = TestWriter::new();
        WebUIHandler::new()
            .render(
                &protocol,
                &test_json!({ "body_end": "content" }),
                &RenderOptions::new("index.html", "/"),
                &mut writer,
            )
            .expect("current protocol must render");
        assert!(writer.get_content().contains("content"));
    }

    #[test]
    fn authored_raw_signal_keys_remain_content_in_both_render_modes() {
        let source = concat!(
            "<html><head></head><body>",
            "{{{head_start}}}|{{{head_end}}}|{{{body_start}}}|{{{body_end}}}|",
            "{{{boundary_start:0}}}|{{{boundary_end:0}}}|{{{streaming_root:forged}}}",
            "</body></html>",
        );
        let state = test_json!({
            "head_start": "hs",
            "head_end": "he",
            "body_start": "bs",
            "body_end": "be",
            "boundary_start:0": "b0s",
            "boundary_end:0": "b0e",
            "streaming_root:forged": "root",
        });
        let mut parser = HtmlParser::new();
        parser.parse("index.html", source).expect("parse fixture");
        let protocol = Protocol::new(WebUIProtocol::new(parser.into_fragment_records()));
        let expected = "<html><head></head><body>hs|he|bs|be|b0s|b0e|root</body></html>";

        let mut ordinary = TestWriter::new();
        WebUIHandler::new()
            .render(
                &protocol,
                &state,
                &RenderOptions::new("index.html", "/"),
                &mut ordinary,
            )
            .unwrap();
        assert_eq!(ordinary.get_content(), expected);

        let mut streaming = FlushTestWriter::default();
        WebUIHandler::new()
            .render_streaming(
                &protocol,
                &state,
                &RenderOptions::new("index.html", "/"),
                &mut streaming,
            )
            .unwrap();
        assert!(streaming.output.contains("<body>hs|he|bs|be|b0s|b0e|root"));
        assert!(!streaming.output.contains("<!--wb:"));
        assert_eq!(streaming.output.matches(STREAMING_MARKER).count(), 1);
        assert_eq!(streaming.output.matches("data-webui-boundary").count(), 1);
    }

    #[test]
    fn streaming_rejects_structural_signals_after_terminal() {
        let fragments = HashMap::from([(
            "index.html".to_string(),
            FragmentList {
                fragments: vec![
                    WebUIFragment::raw("<html><head>"),
                    structural_fragment("head_start"),
                    structural_fragment("head_end"),
                    WebUIFragment::raw("</head><body>"),
                    structural_fragment("body_start"),
                    structural_fragment("body_end"),
                    WebUIFragment::raw("</body>"),
                    structural_fragment("boundary_start:0"),
                    WebUIFragment::raw("</html>"),
                ],
            },
        )]);
        let protocol = Protocol::new(WebUIProtocol::new(fragments));
        let mut streaming = FlushTestWriter::default();
        let result = WebUIHandler::new().render_streaming(
            &protocol,
            &test_json!({}),
            &RenderOptions::new("index.html", "/"),
            &mut streaming,
        );

        match result {
            Err(HandlerError::StreamingBoundary(error)) => {
                assert_eq!(error.signal, "boundary_start:0");
                assert!(error.reason.contains("after the body_end terminal record"));
            }
            other => panic!("expected post-terminal rejection, got {other:?}"),
        }
        assert!(streaming.output.is_empty());
        assert!(!streaming.output.contains("<!--wb:0-->"));

        let mut ordinary = TestWriter::new();
        WebUIHandler::new()
            .render(
                &protocol,
                &test_json!({}),
                &RenderOptions::new("index.html", "/"),
                &mut ordinary,
            )
            .unwrap();
        assert_eq!(
            ordinary.get_content(),
            "<html><head></head><body></body></html>"
        );
    }

    /// Build a streaming protocol with one boundary per `hosts` entry. Each
    /// boundary wraps a component host whose opening tag is split so the
    /// compiler-owned `streaming_root:<tag>` signal (optionally emitted) lands
    /// inside the tag, exactly as `HtmlParser` produces. Components `comp-a` and
    /// `comp-b` carry disjoint templates and disjoint hydration keys.
    fn disjoint_streaming_protocol_ext(
        hosts: &[&str],
        emit_root_signal: bool,
        include_styles: bool,
    ) -> Protocol {
        let mut fragments = HashMap::new();
        let mut entry = vec![
            WebUIFragment::raw("<!DOCTYPE html><html><head>"),
            structural_fragment("head_start"),
            structural_fragment("head_end"),
            WebUIFragment::raw("</head><body>"),
            structural_fragment("body_start"),
        ];
        for (sequence, host) in hosts.iter().enumerate() {
            entry.push(structural_fragment(format!("boundary_start:{sequence}")));
            entry.push(WebUIFragment::raw(format!("<{host}")));
            if emit_root_signal {
                entry.push(structural_fragment(format!("streaming_root:{host}")));
            }
            entry.push(WebUIFragment::raw(">"));
            entry.push(WebUIFragment::component(*host));
            entry.push(WebUIFragment::raw(format!("</{host}>")));
            entry.push(structural_fragment(format!("boundary_end:{sequence}")));
        }
        entry.push(structural_fragment("body_end"));
        entry.push(WebUIFragment::raw("</body></html>"));
        fragments.insert("index.html".to_string(), FragmentList { fragments: entry });
        fragments.insert(
            "comp-a".to_string(),
            FragmentList {
                fragments: vec![WebUIFragment::raw("<b>A</b>")],
            },
        );
        fragments.insert(
            "comp-b".to_string(),
            FragmentList {
                fragments: vec![WebUIFragment::raw("<b>B</b>")],
            },
        );

        let mut document = WebUIProtocol::new(fragments);
        document.initial_state_strategy = InitialStateStrategy::Components as i32;
        document.streaming_boundaries.insert(
            "index.html".to_string(),
            webui_protocol::StreamingBoundaryList {
                names: (0..hosts.len())
                    .map(|index| format!("boundary-{index}"))
                    .collect(),
            },
        );
        document.components.insert(
            "comp-a".to_string(),
            webui_protocol::ComponentData {
                template_json: r#"{"h":"<i>A</i>","th":1}"#.to_string(),
                hydration_mode: StateProjectionMode::Keys as i32,
                hydration_keys: vec!["a_count".to_string()],
                ..Default::default()
            },
        );
        document.components.insert(
            "comp-b".to_string(),
            webui_protocol::ComponentData {
                template_json: r#"{"h":"<i>B</i>","th":1}"#.to_string(),
                hydration_mode: StateProjectionMode::Keys as i32,
                hydration_keys: vec!["b_count".to_string()],
                ..Default::default()
            },
        );
        if include_styles {
            document.set_css_strategy(webui_protocol::CssStrategy::Style);
            document.components.get_mut("comp-a").unwrap().css = ".comp-a{color:red}".to_string();
            document.components.get_mut("comp-b").unwrap().css = ".comp-b{color:blue}".to_string();
            document.populate_style_closures(&["index.html"]);
        }
        Protocol::new(document)
    }

    fn disjoint_streaming_protocol(hosts: &[&str]) -> Protocol {
        disjoint_streaming_protocol_ext(hosts, true, false)
    }

    fn styled_disjoint_streaming_protocol(hosts: &[&str]) -> Protocol {
        disjoint_streaming_protocol_ext(hosts, true, true)
    }

    fn streaming_plan_validation_protocol(signals: &[&str], names: &[&str]) -> Protocol {
        let mut entry = vec![
            WebUIFragment::raw("<html><head>"),
            structural_fragment("head_start"),
            structural_fragment("head_end"),
            WebUIFragment::raw("</head><body>"),
            structural_fragment("body_start"),
        ];
        entry.extend(signals.iter().map(structural_fragment));
        entry.extend([
            structural_fragment("body_end"),
            WebUIFragment::raw("</body></html>"),
        ]);

        let mut document = WebUIProtocol::new(HashMap::from([(
            "index.html".to_string(),
            FragmentList { fragments: entry },
        )]));
        document.streaming_boundaries.insert(
            "index.html".to_string(),
            webui_protocol::StreamingBoundaryList {
                names: names.iter().map(|name| (*name).to_string()).collect(),
            },
        );
        Protocol::new(document)
    }

    #[test]
    fn streaming_response_rejects_cached_malformed_boundary_plan_before_writing() {
        let protocol =
            streaming_plan_validation_protocol(&["boundary_start:0", "boundary_end:1"], &["one"]);
        let handler = WebUIHandler::new();
        let mut writer = FlushTestWriter::default();
        let options = RenderOptions::new("index.html", "/");

        let error = match handler.stream_response(&protocol, &options, &mut writer) {
            Ok(_) => panic!("malformed boundary plan unexpectedly opened a response"),
            Err(error) => error,
        };

        assert!(
            matches!(error, HandlerError::StreamingBoundary(_)),
            "error: {error}"
        );
        assert!(writer.output.is_empty());
        assert!(writer.flushes.is_empty());
    }

    #[test]
    fn streaming_response_rejects_boundary_name_count_mismatch_before_writing() {
        let protocol = streaming_plan_validation_protocol(
            &["boundary_start:0", "boundary_end:0"],
            &["one", "two"],
        );
        let handler = WebUIHandler::new();
        let mut writer = FlushTestWriter::default();
        let options = RenderOptions::new("index.html", "/");

        let error = match handler.stream_response(&protocol, &options, &mut writer) {
            Ok(_) => panic!("mismatched boundary names unexpectedly opened a response"),
            Err(error) => error,
        };

        assert!(
            matches!(error, HandlerError::Invariant(_)),
            "error: {error}"
        );
        assert!(writer.output.is_empty());
        assert!(writer.flushes.is_empty());
    }

    #[test]
    fn streaming_response_interleaves_projected_updates_with_later_boundaries() {
        let protocol = disjoint_streaming_protocol(&["comp-a", "comp-b"]);
        let handler = WebUIHandler::with_plugin(|| {
            Box::new(crate::plugin::webui::WebUIHydrationPlugin::new())
        });
        let mut writer = FlushTestWriter::default();
        let options = RenderOptions::new("index.html", "/");
        let mut response = handler
            .stream_response(&protocol, &options, &mut writer)
            .unwrap();
        let first = response.boundary("boundary-0").unwrap();
        let second = response.boundary("boundary-1").unwrap();

        response.write_shell(&test_json!({})).unwrap();
        response
            .write_boundary(
                first,
                &test_json!({ "a_count": 1, "serverOnly": "secret" }),
                BoundaryMode::Updatable,
            )
            .unwrap();
        response
            .update(first, &test_json!({ "a_count": 7, "serverOnly": "secret" }))
            .unwrap();
        response
            .write_boundary(
                second,
                &test_json!({ "b_count": 2, "serverOnly": "secret" }),
                BoundaryMode::Final,
            )
            .unwrap();
        response.finish(&test_json!({})).unwrap();

        assert_eq!(writer.flushes.len(), 5);
        assert!(writer.output.contains(r#"[2,0,1,0,{"#));
        assert!(writer.output.contains(r#""state":{"a_count":1}"#));
        assert!(writer.output.contains(r#"[2,1,2,0,{"a_count":7}]"#));
        assert!(writer.output.contains(r#"[2,2,0,1,{"#));
        assert!(writer.output.contains(r#""state":{"b_count":2}"#));
        assert!(writer.output.contains("[2,3,3,0,{}]"));
        assert_eq!(writer.output.matches("serverOnly").count(), 0);

        let update_start = writer.output.find("[2,1,2,0,").unwrap();
        let update_end = writer.output[update_start..]
            .find("</script>")
            .map(|offset| update_start + offset)
            .unwrap();
        let update = &writer.output[update_start..update_end];
        assert!(!update.contains("inventory"));
        assert!(!update.contains("templates"));
    }

    #[test]
    fn owned_streaming_session_matches_borrowed_response_bytes() {
        let protocol = Arc::new(disjoint_streaming_protocol(&["comp-a", "comp-b"]));
        let handler = Arc::new(WebUIHandler::with_plugin(|| {
            Box::new(crate::plugin::webui::WebUIHydrationPlugin::new())
        }));

        let mut writer = FlushTestWriter::default();
        let options = RenderOptions::new("index.html", "/");
        let mut response = handler
            .stream_response(&protocol, &options, &mut writer)
            .unwrap();
        let first = response.boundary("boundary-0").unwrap();
        let second = response.boundary("boundary-1").unwrap();
        response.write_shell(&test_json!({})).unwrap();
        response
            .write_boundary(
                first,
                &test_json!({ "a_count": 1 }),
                BoundaryMode::Updatable,
            )
            .unwrap();
        response
            .update(first, &test_json!({ "a_count": 7 }))
            .unwrap();
        response
            .write_boundary(second, &test_json!({ "b_count": 2 }), BoundaryMode::Final)
            .unwrap();
        response.finish(&test_json!({})).unwrap();

        let mut session = StreamingSession::new(
            Arc::clone(&handler),
            Arc::clone(&protocol),
            SessionOptions::new("index.html", "/"),
        )
        .unwrap();
        let session_first = session.boundary("boundary-0").unwrap();
        let session_second = session.boundary("boundary-1").unwrap();
        assert_eq!(session_first, first);
        assert_eq!(session_second, second);
        assert_eq!(session.boundary_count(), 2);

        let chunks: Vec<Vec<u8>> = vec![
            session.write_shell(&test_json!({})).unwrap(),
            session
                .write_boundary(
                    session_first,
                    &test_json!({ "a_count": 1 }),
                    BoundaryMode::Updatable,
                )
                .unwrap(),
            session
                .update(session_first, &test_json!({ "a_count": 7 }))
                .unwrap(),
            session
                .write_boundary(
                    session_second,
                    &test_json!({ "b_count": 2 }),
                    BoundaryMode::Final,
                )
                .unwrap(),
            session.finish(&test_json!({})).unwrap(),
        ];

        // One chunk per host call, matching the borrowed path's flush count.
        assert_eq!(chunks.len(), writer.flushes.len());
        assert!(session.is_finished());
        let joined = String::from_utf8(chunks.concat()).unwrap();
        assert_eq!(joined, writer.output);
    }

    #[test]
    fn owned_streaming_session_rejects_use_after_finish() {
        let protocol = Arc::new(disjoint_streaming_protocol(&["comp-a"]));
        let handler = Arc::new(WebUIHandler::new());
        let mut session =
            StreamingSession::new(handler, protocol, SessionOptions::new("index.html", "/"))
                .unwrap();
        let boundary = session.boundary("boundary-0").unwrap();
        session.write_shell(&test_json!({})).unwrap();
        session
            .write_boundary(boundary, &test_json!({ "a_count": 1 }), BoundaryMode::Final)
            .unwrap();
        session.finish(&test_json!({})).unwrap();

        let error = session.write_shell(&test_json!({})).unwrap_err();
        assert!(error.to_string().contains("already finished"));
        assert!(session.finish(&test_json!({})).is_err());
    }

    #[test]
    fn owned_streaming_session_surfaces_unknown_boundary_names() {
        let protocol = Arc::new(disjoint_streaming_protocol(&["comp-a"]));
        let handler = Arc::new(WebUIHandler::new());
        let session =
            StreamingSession::new(handler, protocol, SessionOptions::new("index.html", "/"))
                .unwrap();
        let error = session.boundary("boundary-O").unwrap_err().to_string();
        assert!(error.contains("boundary-0"));
    }

    #[test]
    fn owned_streaming_session_stays_usable_after_a_rejected_call() {
        let protocol = Arc::new(disjoint_streaming_protocol(&["comp-a"]));
        let handler = Arc::new(WebUIHandler::new());
        let mut session =
            StreamingSession::new(handler, protocol, SessionOptions::new("index.html", "/"))
                .unwrap();
        let boundary = session.boundary("boundary-0").unwrap();
        session.write_shell(&test_json!({})).unwrap();
        // Rejected before any byte is written, so the session is not poisoned.
        assert!(session.update(boundary, &test_json!({ "a": 1 })).is_err());
        assert!(!session.is_finished());
        session
            .write_boundary(boundary, &test_json!({ "a_count": 1 }), BoundaryMode::Final)
            .unwrap();
        session.finish(&test_json!({})).unwrap();
    }

    #[test]
    fn owned_streaming_session_recovers_from_a_rejected_finish() {
        let protocol = Arc::new(disjoint_streaming_protocol(&["comp-a", "comp-b"]));
        let handler = Arc::new(WebUIHandler::new());
        let mut session =
            StreamingSession::new(handler, protocol, SessionOptions::new("index.html", "/"))
                .unwrap();
        let first = session.boundary("boundary-0").unwrap();
        let second = session.boundary("boundary-1").unwrap();
        session.write_shell(&test_json!({})).unwrap();
        session
            .write_boundary(first, &test_json!({ "a_count": 1 }), BoundaryMode::Final)
            .unwrap();

        // Rejected before any byte is written, so the response must survive.
        let error = session.finish(&test_json!({})).unwrap_err();
        assert!(
            error
                .to_string()
                .contains("every boundary must be committed"),
            "unexpected error: {error}"
        );
        assert!(!session.is_finished());

        session
            .write_boundary(second, &test_json!({ "b_count": 1 }), BoundaryMode::Final)
            .unwrap();
        let tail = session.finish(&test_json!({})).unwrap();
        assert!(!tail.is_empty());
        assert!(session.is_finished());
    }

    #[test]
    fn owned_streaming_session_rejects_finish_before_the_shell() {
        let protocol = Arc::new(disjoint_streaming_protocol(&["comp-a"]));
        let handler = Arc::new(WebUIHandler::new());
        let mut session =
            StreamingSession::new(handler, protocol, SessionOptions::new("index.html", "/"))
                .unwrap();
        let boundary = session.boundary("boundary-0").unwrap();

        let error = session.finish(&test_json!({})).unwrap_err();
        assert!(
            error.to_string().contains("write_shell must be called"),
            "unexpected error: {error}"
        );
        assert!(!session.is_finished());

        session.write_shell(&test_json!({})).unwrap();
        session
            .write_boundary(boundary, &test_json!({ "a_count": 1 }), BoundaryMode::Final)
            .unwrap();
        session.finish(&test_json!({})).unwrap();
        assert!(session.is_finished());
    }

    #[test]
    fn streaming_response_is_poisoned_after_partial_boundary_flush_failure() {
        let protocol = disjoint_streaming_protocol(&["comp-a"]);
        let handler = WebUIHandler::new();
        let mut writer = FlushTestWriter {
            fail_flush_at: Some(1),
            ..FlushTestWriter::default()
        };
        let options = RenderOptions::new("index.html", "/");
        let mut response = handler
            .stream_response(&protocol, &options, &mut writer)
            .unwrap();
        let boundary = response.boundary("boundary-0").unwrap();
        response.write_shell(&test_json!({})).unwrap();

        let first_error = response
            .write_boundary(boundary, &test_json!({ "a_count": 1 }), BoundaryMode::Final)
            .unwrap_err();
        assert!(matches!(first_error, HandlerError::ClientDisconnected));

        let retry_error = response
            .write_boundary(boundary, &test_json!({ "a_count": 1 }), BoundaryMode::Final)
            .unwrap_err();
        assert!(
            retry_error
                .to_string()
                .contains("unusable after a previous render or transport failure"),
            "error: {retry_error}"
        );

        drop(response);
        assert_eq!(writer.output.matches("<!--wb:0-->").count(), 1);
        assert_eq!(writer.flushes.len(), 1);
        assert!(!writer.ended);
    }

    #[test]
    fn streaming_response_rejects_updates_to_final_boundaries() {
        let protocol = disjoint_streaming_protocol(&["comp-a"]);
        let handler = WebUIHandler::new();
        let mut writer = FlushTestWriter::default();
        let options = RenderOptions::new("index.html", "/");
        let mut response = handler
            .stream_response(&protocol, &options, &mut writer)
            .unwrap();
        let boundary = response.boundary("boundary-0").unwrap();
        response.write_shell(&test_json!({})).unwrap();
        response
            .write_boundary(boundary, &test_json!({ "a_count": 1 }), BoundaryMode::Final)
            .unwrap();

        let error = response
            .update(boundary, &test_json!({ "a_count": 2 }))
            .unwrap_err();
        assert!(
            error.to_string().contains("committed as final"),
            "error: {error}"
        );
    }

    #[test]
    fn streaming_response_rejects_non_object_updates_before_writing() {
        let protocol = disjoint_streaming_protocol(&["comp-a"]);
        let handler = WebUIHandler::new();
        let mut writer = FlushTestWriter::default();
        let options = RenderOptions::new("index.html", "/");
        let mut response = handler
            .stream_response(&protocol, &options, &mut writer)
            .unwrap();
        let boundary = response.boundary("boundary-0").unwrap();
        response.write_shell(&test_json!({})).unwrap();
        response
            .write_boundary(
                boundary,
                &test_json!({ "a_count": 1 }),
                BoundaryMode::Updatable,
            )
            .unwrap();
        let error = response
            .update(boundary, &test_json!("invalid"))
            .unwrap_err();
        assert!(error.to_string().contains("JSON object"), "error: {error}");

        response
            .update(boundary, &test_json!({ "a_count": 2 }))
            .unwrap();
        response.finish(&test_json!({})).unwrap();
        assert_eq!(writer.flushes.len(), 4);
        assert_eq!(writer.output.matches("data-webui-boundary").count(), 3);
        assert!(writer.output.contains(r#"[2,1,2,0,{"a_count":2}]"#));
    }

    #[test]
    fn streaming_response_unknown_boundary_suggests_valid_name() {
        let protocol = disjoint_streaming_protocol(&["comp-a"]);
        let handler = WebUIHandler::new();
        let mut writer = FlushTestWriter::default();
        let options = RenderOptions::new("index.html", "/");
        let response = handler
            .stream_response(&protocol, &options, &mut writer)
            .unwrap();

        let error = response.boundary("boundry-0").unwrap_err();
        assert!(
            error.to_string().contains("did you mean `boundary-0`?"),
            "error: {error}"
        );
    }

    fn streaming_root_validation_protocol(
        mut host_fragments: Vec<WebUIFragment>,
        tail: Vec<WebUIFragment>,
    ) -> Protocol {
        let mut entry = vec![
            WebUIFragment::raw("<html><head>"),
            structural_fragment("head_start"),
            structural_fragment("head_end"),
            WebUIFragment::raw("</head><body>"),
            structural_fragment("body_start"),
            structural_fragment("boundary_start:0"),
        ];
        entry.append(&mut host_fragments);
        entry.extend(tail);
        let fragments = HashMap::from([
            ("index.html".to_string(), FragmentList { fragments: entry }),
            (
                "comp-a".to_string(),
                FragmentList {
                    fragments: vec![WebUIFragment::raw("<b>A</b>")],
                },
            ),
            (
                "comp-b".to_string(),
                FragmentList {
                    fragments: vec![WebUIFragment::raw("<b>B</b>")],
                },
            ),
        ]);
        Protocol::new(WebUIProtocol::new(fragments))
    }

    fn completed_streaming_tail() -> Vec<WebUIFragment> {
        vec![
            WebUIFragment::raw("</comp-a>"),
            structural_fragment("boundary_end:0"),
            structural_fragment("body_end"),
            WebUIFragment::raw("</body></html>"),
        ]
    }

    fn assert_streaming_root_error(
        protocol: &Protocol,
        expected_signal: &str,
        expected_reason: &str,
    ) {
        let mut writer = FlushTestWriter::default();
        let result = WebUIHandler::new().render_streaming(
            protocol,
            &test_json!({}),
            &RenderOptions::new("index.html", "/"),
            &mut writer,
        );
        match result {
            Err(HandlerError::StreamingBoundary(error)) => {
                assert_eq!(error.signal, expected_signal);
                assert!(
                    error.reason.contains(expected_reason),
                    "reason: {}",
                    error.reason
                );
            }
            other => panic!("expected streaming-root rejection, got {other:?}"),
        }
    }

    #[test]
    fn streaming_root_signal_injects_data_ws_inside_boundary() {
        let protocol = disjoint_streaming_protocol(&["comp-a"]);
        let handler = WebUIHandler::with_plugin(|| {
            Box::new(crate::plugin::webui::WebUIHydrationPlugin::new())
        });
        let mut writer = FlushTestWriter::default();
        handler
            .render_streaming(
                &protocol,
                &test_json!({ "a_count": 1 }),
                &RenderOptions::new("index.html", "/"),
                &mut writer,
            )
            .unwrap();
        // The parser-owned signal is consumed to inject exactly ` data-ws`
        // inside the host's opening tag, before the custom element upgrades.
        assert!(
            writer.output.contains("<comp-a data-ws>"),
            "streamed host must carry data-ws: {}",
            writer.output
        );
    }

    #[test]
    fn streaming_root_signal_preserves_ordinary_output_bytes() {
        // Ordinary rendering ignores `streaming_root` byte-for-byte: identical
        // output with and without the signal, and never a `data-ws` attribute.
        let with_signal = disjoint_streaming_protocol_ext(&["comp-a", "comp-b"], true, false);
        let without_signal = disjoint_streaming_protocol_ext(&["comp-a", "comp-b"], false, false);
        let state = test_json!({ "a_count": 1, "b_count": 2 });
        let plugin = || {
            WebUIHandler::with_plugin(
                || Box::new(crate::plugin::webui::WebUIHydrationPlugin::new()),
            )
        };
        let mut with_writer = TestWriter::new();
        let mut without_writer = TestWriter::new();
        plugin()
            .render(
                &with_signal,
                &state,
                &RenderOptions::new("index.html", "/"),
                &mut with_writer,
            )
            .unwrap();
        plugin()
            .render(
                &without_signal,
                &state,
                &RenderOptions::new("index.html", "/"),
                &mut without_writer,
            )
            .unwrap();
        // The rendered DOM (everything up to the inert data block) is
        // deterministic and is where a leaked `data-ws` would appear. The
        // ordinary template map is HashSet-ordered, so compare the DOM prefix
        // for byte identity rather than the whole document.
        let dom_prefix = |content: &str| -> String {
            content
                .split_once(r#"<script type="application/json" id="webui-data""#)
                .map_or_else(|| content.to_string(), |(head, _)| head.to_string())
        };
        assert_eq!(
            dom_prefix(&with_writer.get_content()),
            dom_prefix(&without_writer.get_content())
        );
        assert!(!with_writer.get_content().contains("data-ws"));
        assert!(with_writer.get_content().contains("<comp-a>"));
    }

    #[test]
    fn streaming_component_without_root_signal_is_rejected() {
        let protocol = streaming_root_validation_protocol(
            vec![
                WebUIFragment::raw("<comp-a>"),
                WebUIFragment::component("comp-a"),
            ],
            completed_streaming_tail(),
        );

        assert_streaming_root_error(
            &protocol,
            "streaming_root:comp-a",
            "has no matching compiler-owned root signal",
        );
    }

    #[test]
    fn duplicate_and_misplaced_streaming_root_signals_are_rejected() {
        let duplicate = streaming_root_validation_protocol(
            vec![
                WebUIFragment::raw("<comp-a"),
                structural_fragment("streaming_root:comp-a"),
                structural_fragment("streaming_root:comp-a"),
                WebUIFragment::raw(">"),
                WebUIFragment::component("comp-a"),
            ],
            completed_streaming_tail(),
        );
        assert_streaming_root_error(&duplicate, "streaming_root:comp-a", "is misplaced");

        let after_tag_close = streaming_root_validation_protocol(
            vec![
                WebUIFragment::raw("<comp-a>"),
                structural_fragment("streaming_root:comp-a"),
                WebUIFragment::component("comp-a"),
            ],
            completed_streaming_tail(),
        );
        assert_streaming_root_error(&after_tag_close, "streaming_root:comp-a", "is misplaced");

        let detached_from_opening = streaming_root_validation_protocol(
            vec![
                WebUIFragment::raw("not an opening tag"),
                structural_fragment("streaming_root:comp-a"),
                WebUIFragment::raw(">"),
                WebUIFragment::component("comp-a"),
            ],
            completed_streaming_tail(),
        );
        assert_streaming_root_error(
            &detached_from_opening,
            "streaming_root:comp-a",
            "unclosed component opening-tag close",
        );
    }

    #[test]
    fn mismatched_streaming_root_signal_is_rejected() {
        let protocol = streaming_root_validation_protocol(
            vec![
                WebUIFragment::raw("<comp-a"),
                structural_fragment("streaming_root:comp-b"),
                WebUIFragment::raw(">"),
                WebUIFragment::component("comp-a"),
            ],
            completed_streaming_tail(),
        );

        assert_streaming_root_error(
            &protocol,
            "streaming_root:comp-b",
            "opening host renders component <comp-a>",
        );
    }

    #[test]
    fn pending_streaming_root_is_rejected_at_structural_ends() {
        let protocol = streaming_root_validation_protocol(
            vec![
                WebUIFragment::raw("<comp-a"),
                structural_fragment("streaming_root:comp-a"),
                WebUIFragment::raw(">"),
            ],
            vec![structural_fragment("boundary_end:0")],
        );
        assert_streaming_root_error(&protocol, "streaming_root:comp-a", "is misplaced");
    }

    #[test]
    fn streaming_root_outside_boundary_is_rejected() {
        let fragments = HashMap::from([
            (
                "index.html".to_string(),
                FragmentList {
                    fragments: vec![
                        WebUIFragment::raw("<html><head>"),
                        structural_fragment("head_start"),
                        structural_fragment("head_end"),
                        WebUIFragment::raw("</head><body>"),
                        structural_fragment("body_start"),
                        WebUIFragment::raw("<comp-a"),
                        structural_fragment("streaming_root:comp-a"),
                        WebUIFragment::raw(">"),
                        WebUIFragment::component("comp-a"),
                        WebUIFragment::raw("</comp-a>"),
                        structural_fragment("body_end"),
                        WebUIFragment::raw("</body></html>"),
                    ],
                },
            ),
            (
                "comp-a".to_string(),
                FragmentList {
                    fragments: vec![WebUIFragment::raw("<b>A</b>")],
                },
            ),
        ]);
        let protocol = Protocol::new(WebUIProtocol::new(fragments));
        let mut writer = FlushTestWriter::default();
        let result = WebUIHandler::new().render_streaming(
            &protocol,
            &test_json!({}),
            &RenderOptions::new("index.html", "/"),
            &mut writer,
        );
        match result {
            Err(HandlerError::StreamingBoundary(err)) => {
                assert_eq!(err.signal, "streaming_root:comp-a");
                assert!(
                    err.reason.contains("outside any <boundary>"),
                    "reason: {}",
                    err.reason
                );
            }
            other => panic!("expected outside-boundary rejection, got {other:?}"),
        }
    }

    fn parser_route_protocol(with_boundary: bool) -> Protocol {
        let mut parser = HtmlParser::with_options(DomStrategy::Light);
        parser
            .component_registry_mut()
            .register_component(ComponentRegistration::new(
                "route-page",
                "<p>route content</p>",
                None,
                true,
            ))
            .expect("register route component");
        let route = r#"<route path="/" component="route-page" exact />"#;
        let html = if with_boundary {
            format!(
                "<html><head></head><body>\
                 <boundary name=\"route\">{route}</boundary>\
                 </body></html>"
            )
        } else {
            format!("<html><head></head><body>{route}</body></html>")
        };
        parser
            .parse("index.html", &html)
            .expect("parse route streaming fixture");
        let mut document = WebUIProtocol::new(parser.into_fragment_records());
        document.components.insert(
            "route-page".to_string(),
            webui_protocol::ComponentData::default(),
        );
        Protocol::new(document)
    }

    #[test]
    fn streaming_parser_route_host_is_marked_inside_explicit_boundary() {
        let protocol = parser_route_protocol(true);
        let mut writer = FlushTestWriter::default();
        WebUIHandler::new()
            .render_streaming(
                &protocol,
                &test_json!({}),
                &RenderOptions::new("index.html", "/"),
                &mut writer,
            )
            .unwrap();

        assert!(
            writer
                .output
                .contains("<route-page data-wl data-ws><p>route content</p></route-page>"),
            "matched route host must be deferred before upgrade: {}",
            writer.output
        );
    }

    #[test]
    fn streaming_parser_route_host_outside_boundary_is_rejected() {
        let protocol = parser_route_protocol(false);
        let mut writer = FlushTestWriter::default();
        let result = WebUIHandler::new().render_streaming(
            &protocol,
            &test_json!({}),
            &RenderOptions::new("index.html", "/"),
            &mut writer,
        );

        match result {
            Err(HandlerError::StreamingBoundary(error)) => {
                assert_eq!(error.signal, "streaming_root:route-page");
                assert!(error.reason.contains("outside any <boundary>"));
            }
            other => panic!("expected route boundary rejection, got {other:?}"),
        }
        assert!(
            !writer.output.contains("<route-page>"),
            "an unmarked custom-element opening must never be completed"
        );
    }

    #[test]
    fn streaming_checkpoint_preserves_relative_route_base_for_metadata() {
        let entry = vec![
            WebUIFragment::raw("<html><head>"),
            structural_fragment("head_start"),
            structural_fragment("head_end"),
            WebUIFragment::raw("</head><body>"),
            structural_fragment("body_start"),
            structural_fragment("boundary_start:0"),
            WebUIFragment::route_from(webui_protocol::WebUiFragmentRoute {
                path: "/account".into(),
                fragment_id: "account-shell".into(),
                exact: false,
                ..Default::default()
            }),
            structural_fragment("boundary_end:0"),
            structural_fragment("body_end"),
            WebUIFragment::raw("</body></html>"),
        ];
        let fragments = HashMap::from([
            ("index.html".to_string(), FragmentList { fragments: entry }),
            (
                "account-shell".to_string(),
                FragmentList {
                    fragments: vec![WebUIFragment::route_from(
                        webui_protocol::WebUiFragmentRoute {
                            path: "./details".into(),
                            fragment_id: "details-page".into(),
                            exact: true,
                            pending_component: "details-loading".into(),
                            error_component: "details-error".into(),
                            ..Default::default()
                        },
                    )],
                },
            ),
            (
                "details-page".to_string(),
                FragmentList {
                    fragments: vec![WebUIFragment::raw("<p>details</p>")],
                },
            ),
            (
                "details-loading".to_string(),
                FragmentList {
                    fragments: vec![WebUIFragment::raw("<p>loading</p>")],
                },
            ),
            (
                "details-error".to_string(),
                FragmentList {
                    fragments: vec![WebUIFragment::raw("<p>error</p>")],
                },
            ),
        ]);
        let mut document = WebUIProtocol::new(fragments);
        document.initial_state_strategy = InitialStateStrategy::Components as i32;
        for name in [
            "account-shell",
            "details-page",
            "details-loading",
            "details-error",
        ] {
            document.components.insert(
                name.to_string(),
                webui_protocol::ComponentData {
                    template_json: format!(r#"{{"h":"<{name}></{name}>","th":1}}"#),
                    ..Default::default()
                },
            );
        }
        let protocol = Protocol::new(document);
        let handler = WebUIHandler::with_plugin(|| {
            Box::new(crate::plugin::webui::WebUIHydrationPlugin::new())
        });
        let mut writer = FlushTestWriter::default();

        handler
            .render_streaming(
                &protocol,
                &test_json!({}),
                &RenderOptions::new("index.html", "/account/details"),
                &mut writer,
            )
            .unwrap();

        let first = &writer.output[..writer.flushes[0]];
        assert!(
            first.contains(r#""details-loading":"#),
            "relative-route pending metadata missing: {first}"
        );
        assert!(
            first.contains(r#""details-error":"#),
            "relative-route error metadata missing: {first}"
        );
    }

    #[test]
    fn streaming_checkpoint_keeps_independent_siblings_beside_route_roots() {
        let entry = vec![
            WebUIFragment::raw("<html><head>"),
            structural_fragment("head_start"),
            structural_fragment("head_end"),
            WebUIFragment::raw("</head><body>"),
            structural_fragment("body_start"),
            structural_fragment("boundary_start:0"),
            WebUIFragment::raw("<static-shell"),
            structural_fragment("streaming_root:static-shell"),
            WebUIFragment::raw(">"),
            WebUIFragment::component("static-shell"),
            WebUIFragment::raw("</static-shell>"),
            WebUIFragment::raw("<route-shell"),
            structural_fragment("streaming_root:route-shell"),
            WebUIFragment::raw(">"),
            WebUIFragment::component("route-shell"),
            WebUIFragment::raw("</route-shell>"),
            structural_fragment("boundary_end:0"),
            structural_fragment("body_end"),
            WebUIFragment::raw("</body></html>"),
        ];
        let fragments = HashMap::from([
            ("index.html".to_string(), FragmentList { fragments: entry }),
            (
                "static-shell".to_string(),
                FragmentList {
                    fragments: vec![WebUIFragment::if_cond(
                        ConditionExpr::identifier("show_hidden"),
                        "static-hidden-if",
                    )],
                },
            ),
            (
                "static-hidden-if".to_string(),
                FragmentList {
                    fragments: vec![WebUIFragment::component("static-hidden")],
                },
            ),
            (
                "static-hidden".to_string(),
                FragmentList {
                    fragments: vec![WebUIFragment::raw("<p>hidden</p>")],
                },
            ),
            (
                "route-shell".to_string(),
                FragmentList {
                    fragments: vec![WebUIFragment::route_from(
                        webui_protocol::WebUiFragmentRoute {
                            path: "/account".into(),
                            fragment_id: "route-page".into(),
                            exact: true,
                            ..Default::default()
                        },
                    )],
                },
            ),
            (
                "route-page".to_string(),
                FragmentList {
                    fragments: vec![WebUIFragment::raw("<p>route</p>")],
                },
            ),
        ]);
        let mut document = WebUIProtocol::new(fragments);
        document.set_css_strategy(webui_protocol::CssStrategy::Link);
        document.initial_state_strategy = InitialStateStrategy::Components as i32;
        for (name, keys) in [
            ("static-shell", vec!["show_hidden", "static_count"]),
            ("static-hidden", vec!["hidden_count"]),
            ("route-shell", vec!["route_shell_count"]),
            ("route-page", vec!["route_count"]),
        ] {
            document.components.insert(
                name.to_string(),
                webui_protocol::ComponentData {
                    template_json: format!(r#"{{"h":"<{name}></{name}>","th":1}}"#),
                    hydration_mode: StateProjectionMode::Keys as i32,
                    hydration_keys: keys.into_iter().map(str::to_string).collect(),
                    css_href: format!("/{name}.css"),
                    ..Default::default()
                },
            );
        }
        document.populate_style_closures(&["index.html"]);
        let protocol = Protocol::new(document);
        assert_eq!(
            protocol
                .component_reachability()
                .is_route_dependent(protocol.component_index()["route-shell"]),
            Some(true),
            "fixture must exercise request-aware checkpoint reachability"
        );
        let handler = WebUIHandler::with_plugin(|| {
            Box::new(crate::plugin::webui::WebUIHydrationPlugin::new())
        });
        let mut writer = FlushTestWriter::default();

        handler
            .render_streaming(
                &protocol,
                &test_json!({
                    "show_hidden": false,
                    "static_count": 1,
                    "hidden_count": 2,
                    "route_shell_count": 3,
                    "route_count": 4,
                    "unrelated": "private",
                }),
                &RenderOptions::new("index.html", "/account"),
                &mut writer,
            )
            .unwrap();

        let checkpoint = &writer.output[..writer.flushes[0]];
        for template in ["static-shell", "static-hidden", "route-shell", "route-page"] {
            assert!(
                checkpoint.contains(&format!(r#""{template}":"#)),
                "{template} metadata missing: {checkpoint}"
            );
        }
        for state in [
            r#""show_hidden":false"#,
            r#""static_count":1"#,
            r#""hidden_count":2"#,
            r#""route_shell_count":3"#,
            r#""route_count":4"#,
        ] {
            assert!(
                checkpoint.contains(state),
                "{state} state missing: {checkpoint}"
            );
        }
        assert!(
            !checkpoint.contains("unrelated"),
            "checkpoint leaked unrelated state: {checkpoint}"
        );
        assert!(
            checkpoint.contains(
                r#""css":["/static-shell.css","/static-hidden.css","/route-shell.css","/route-page.css"]"#
            ),
            "checkpoint metadata lost source order: {checkpoint}"
        );
        assert!(
            checkpoint.contains(r#""componentStyles":"#)
                && checkpoint.contains(r#""strategy":"link""#)
                && checkpoint.contains(r#""static-shell":["static-shell","static-hidden"]"#),
            "checkpoint is missing tree-local style metadata: {checkpoint}"
        );
        for resource in ["static-shell", "static-hidden", "route-shell", "route-page"] {
            assert_eq!(
                writer
                    .output
                    .matches(&format!(r#"data-webui-resource="{resource}""#))
                    .count(),
                1,
                "Document delivery state must persist across streamed checkpoints"
            );
        }
    }

    #[test]
    fn streaming_checkpoints_carry_boundary_local_templates_and_state() {
        // Three boundaries: comp-a, comp-b, then comp-a reused. Each checkpoint
        // envelope must be locally scoped — templates/state for exactly the
        // components rendered since the previous checkpoint, with reused tags
        // carrying hydration state but never a duplicate template.
        let protocol = disjoint_streaming_protocol(&["comp-a", "comp-b", "comp-a"]);
        let handler = WebUIHandler::with_plugin(|| {
            Box::new(crate::plugin::webui::WebUIHydrationPlugin::new())
        });
        let state = test_json!({ "a_count": 1, "b_count": 2, "serverOnly": "secret" });
        let mut writer = FlushTestWriter::default();
        handler
            .render_streaming(
                &protocol,
                &state,
                &RenderOptions::new("index.html", "/"),
                &mut writer,
            )
            .unwrap();

        assert_eq!(
            writer.flushes.len(),
            4,
            "three boundary commits plus one terminal"
        );
        let segment = |index: usize| -> &str {
            let start = if index == 0 {
                0
            } else {
                writer.flushes[index - 1]
            };
            &writer.output[start..writer.flushes[index]]
        };
        let b0 = segment(0);
        let b1 = segment(1);
        let b2 = segment(2);

        // Boundary 0: only comp-a's template/state; no comp-b artifacts.
        assert!(b0.contains(r#""templates":{"comp-a":"#), "b0: {b0}");
        assert!(b0.contains(r#""a_count":1"#), "b0: {b0}");
        assert!(b0.contains(r#"[2,0,0,0,{"componentStyles":"#), "b0: {b0}");
        assert!(b0.contains(r#""inventory":"01""#), "b0: {b0}");
        assert!(!b0.contains("comp-b"), "b0 leaked comp-b: {b0}");
        assert!(!b0.contains("b_count"), "b0 leaked b_count: {b0}");

        // Boundary 1: only comp-b's template/state; no duplicate comp-a template.
        assert!(b1.contains(r#""templates":{"comp-b":"#), "b1: {b1}");
        assert!(b1.contains(r#""b_count":2"#), "b1: {b1}");
        assert!(b1.contains(r#"[2,1,0,1,{"componentStyles":"#), "b1: {b1}");
        assert!(b1.contains(r#""inventory":"02""#), "b1: {b1}");
        assert!(
            !b1.contains(r#""templates":{"comp-a"#),
            "b1 re-sent comp-a: {b1}"
        );
        assert!(!b1.contains("a_count"), "b1 leaked a_count: {b1}");

        // Boundary 2: comp-a reused — state present, template absent (empty delta).
        assert!(b2.contains(r#""a_count":1"#), "b2: {b2}");
        assert!(b2.contains(r#"[2,2,0,2,{"componentStyles":"#), "b2: {b2}");
        assert!(b2.contains(r#""inventory":"""#), "b2: {b2}");
        assert!(
            !b2.contains(r#""templates""#),
            "b2 re-sent a template: {b2}"
        );

        // Server-only state never leaks into any envelope.
        assert!(!writer.output.contains("serverOnly"));
    }

    #[test]
    fn streaming_checkpoints_emit_style_resources_and_closures_once() {
        let protocol = styled_disjoint_streaming_protocol(&["comp-a", "comp-b", "comp-a"]);
        let handler = WebUIHandler::with_plugin(|| {
            Box::new(crate::plugin::webui::WebUIHydrationPlugin::new())
        });
        let mut writer = FlushTestWriter::default();
        handler
            .render_streaming(
                &protocol,
                &test_json!({ "a_count": 1, "b_count": 2 }),
                &RenderOptions::new("index.html", "/"),
                &mut writer,
            )
            .unwrap();

        let segment = |index: usize| -> &str {
            let start = if index == 0 {
                0
            } else {
                writer.flushes[index - 1]
            };
            &writer.output[start..writer.flushes[index]]
        };
        let checkpoint_styles = |checkpoint: &str| -> Value {
            let script = checkpoint
                .rfind(r#"<script type="application/json" data-webui-boundary"#)
                .unwrap();
            let payload_start = script + checkpoint[script..].find('>').unwrap() + 1;
            let payload_end =
                payload_start + checkpoint[payload_start..].find("</script>").unwrap();
            let record: Value =
                serde_json::from_str(&checkpoint[payload_start..payload_end]).unwrap();
            record[4]["componentStyles"].clone()
        };

        let first = checkpoint_styles(segment(0));
        assert!(first["resources"]["comp-a"].is_object());
        assert!(first["resources"]["comp-b"].is_object());
        assert!(first["closures"]["index.html"].is_array());
        assert!(first["closures"]["comp-a"].is_array());

        let second = checkpoint_styles(segment(1));
        assert_eq!(second["resources"], test_json!({}));
        assert_eq!(second["closures"], test_json!({ "comp-b": ["comp-b"] }));

        let repeated = checkpoint_styles(segment(2));
        assert_eq!(repeated["resources"], test_json!({}));
        assert_eq!(repeated["closures"], test_json!({}));
        for resource in ["comp-a", "comp-b"] {
            assert_eq!(
                [&first, &second, &repeated]
                    .iter()
                    .filter(|styles| {
                        styles["resources"]
                            .as_object()
                            .is_some_and(|resources| resources.contains_key(resource))
                    })
                    .count(),
                1,
                "{resource} style metadata was serialized more than once"
            );
        }
    }

    #[test]
    fn streaming_checkpoint_carries_reachable_unrendered_metadata_without_inventory() {
        let entry = vec![
            WebUIFragment::raw("<html><head>"),
            structural_fragment("head_start"),
            structural_fragment("head_end"),
            WebUIFragment::raw("</head><body>"),
            structural_fragment("body_start"),
            structural_fragment("boundary_start:0"),
            WebUIFragment::raw("<comp-a"),
            structural_fragment("streaming_root:comp-a"),
            WebUIFragment::raw(">"),
            WebUIFragment::component("comp-a"),
            WebUIFragment::raw("</comp-a>"),
            structural_fragment("boundary_end:0"),
            structural_fragment("boundary_start:1"),
            WebUIFragment::raw("<comp-hidden"),
            structural_fragment("streaming_root:comp-hidden"),
            WebUIFragment::raw(">"),
            WebUIFragment::component("comp-hidden"),
            WebUIFragment::raw("</comp-hidden>"),
            structural_fragment("boundary_end:1"),
            structural_fragment("body_end"),
            WebUIFragment::raw("</body></html>"),
        ];
        let fragments = HashMap::from([
            ("index.html".to_string(), FragmentList { fragments: entry }),
            (
                "comp-a".to_string(),
                FragmentList {
                    fragments: vec![WebUIFragment::if_cond(
                        ConditionExpr::identifier("show_hidden"),
                        "hidden-if",
                    )],
                },
            ),
            (
                "hidden-if".to_string(),
                FragmentList {
                    fragments: vec![WebUIFragment::component("comp-hidden")],
                },
            ),
            (
                "comp-hidden".to_string(),
                FragmentList {
                    fragments: vec![WebUIFragment::raw("<p>hidden</p>")],
                },
            ),
        ]);
        let mut document = WebUIProtocol::new(fragments);
        document.set_css_strategy(webui_protocol::CssStrategy::Module);
        document.initial_state_strategy = InitialStateStrategy::Components as i32;
        document.components.insert(
            "comp-a".to_string(),
            webui_protocol::ComponentData {
                template_json: r#"{"h":"<div></div>","th":1}"#.to_string(),
                hydration_mode: StateProjectionMode::Keys as i32,
                hydration_keys: vec!["show_hidden".to_string()],
                ..Default::default()
            },
        );
        document.components.insert(
            "comp-hidden".to_string(),
            webui_protocol::ComponentData {
                template_json: r#"{"h":"<p>hidden</p>","th":1}"#.to_string(),
                css: ".hidden{display:block}".to_string(),
                hydration_mode: StateProjectionMode::Keys as i32,
                hydration_keys: vec!["hidden_count".to_string()],
                ..Default::default()
            },
        );
        document.populate_style_closures(&["index.html"]);
        let protocol = Protocol::new(document);
        let handler = WebUIHandler::with_plugin(|| {
            Box::new(crate::plugin::webui::WebUIHydrationPlugin::new())
        });
        let mut writer = FlushTestWriter::default();

        handler
            .render_streaming(
                &protocol,
                &test_json!({
                    "show_hidden": false,
                    "hidden_count": 7,
                    "unrelated": "private",
                }),
                &RenderOptions::new("index.html", "/"),
                &mut writer,
            )
            .unwrap();

        let boundary = |index: usize| -> &str {
            let start = if index == 0 {
                0
            } else {
                writer.flushes[index - 1]
            };
            &writer.output[start..writer.flushes[index]]
        };
        let first = boundary(0);
        let second = boundary(1);

        assert!(first.contains(r#""comp-a":"#), "first: {first}");
        assert!(first.contains(r#""comp-hidden":"#), "first: {first}");
        assert!(first.contains(r#""show_hidden":false"#), "first: {first}");
        assert!(first.contains(r#""hidden_count":7"#), "first: {first}");
        assert!(first.contains(r#""inventory":"01""#), "first: {first}");
        assert!(
            first.contains(r#"<script type="importmap""#),
            "first: {first}"
        );
        assert!(
            !first.contains("unrelated"),
            "first leaked unrelated state: {first}"
        );

        assert!(
            !second.contains(r#""templates""#),
            "second re-sent metadata: {second}"
        );
        assert!(
            !second.contains(r#"<script type="importmap""#),
            "second re-sent CSS: {second}"
        );
        assert!(second.contains(r#""hidden_count":7"#), "second: {second}");
        assert!(second.contains(r#""inventory":"02""#), "second: {second}");
        assert_eq!(
            writer.output.matches(r#"<script type="importmap""#).count(),
            1
        );
    }

    #[test]
    fn streaming_flush_count_is_boundaries_plus_one() {
        for &count in &[1usize, 3, 10, 100] {
            let hosts: Vec<&str> = (0..count)
                .map(|index| if index % 2 == 0 { "comp-a" } else { "comp-b" })
                .collect();
            let protocol = disjoint_streaming_protocol(&hosts);
            let handler = WebUIHandler::with_plugin(|| {
                Box::new(crate::plugin::webui::WebUIHydrationPlugin::new())
            });
            let mut writer = FlushTestWriter::default();
            handler
                .render_streaming(
                    &protocol,
                    &test_json!({ "a_count": 1, "b_count": 2 }),
                    &RenderOptions::new("index.html", "/"),
                    &mut writer,
                )
                .unwrap();
            assert_eq!(
                writer.flushes.len(),
                count + 1,
                "expected boundaries+1 flushes for {count} boundaries"
            );
        }
    }

    #[test]
    fn checkpoint_state_key_scratch_reuses_allocation_and_stays_local() {
        let mut protocol = WebUIProtocol::new(HashMap::new());
        protocol.initial_state_strategy = InitialStateStrategy::Components as i32;
        protocol.components.insert(
            "comp-a".to_string(),
            webui_protocol::ComponentData {
                hydration_mode: StateProjectionMode::Keys as i32,
                hydration_keys: vec!["a".to_string(), "shared".to_string()],
                ..Default::default()
            },
        );
        protocol.components.insert(
            "comp-b".to_string(),
            webui_protocol::ComponentData {
                hydration_mode: StateProjectionMode::Keys as i32,
                hydration_keys: vec!["b".to_string(), "shared".to_string()],
                ..Default::default()
            },
        );

        let mut scratch = Vec::with_capacity(INITIAL_KEY_CAPACITY);
        assert!(!collect_hydration_state_into(
            &protocol,
            ["comp-a"],
            &mut scratch,
        ));
        assert_eq!(scratch, ["a", "shared"]);
        let pointer = scratch.as_ptr();
        let capacity = scratch.capacity();

        assert!(!collect_hydration_state_into(
            &protocol,
            ["comp-b"],
            &mut scratch,
        ));
        assert_eq!(scratch, ["b", "shared"]);
        assert_eq!(scratch.as_ptr(), pointer);
        assert_eq!(scratch.capacity(), capacity);
    }
}

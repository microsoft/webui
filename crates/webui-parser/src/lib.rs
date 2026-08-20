// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

//! Directive parser for WebUI template directives.
//!
//! This module handles parsing WebUI-specific directives like <for>, <if>, etc.
mod asset_filename;
mod comment_policy;
mod component_policy;
mod component_registry;
mod condition_parser;
mod css_link;
mod css_parser;
mod diagnostic;
mod error;
mod handlebars_parser;
mod html_parser;
pub mod plugin;
mod route_parser;
mod suggest;

pub use asset_filename::{
    AssetFileNameTemplate, AssetFileNameTemplateError, DEFAULT_ASSET_FILE_NAME_TEMPLATE,
};
pub use component_registry::{Component, ComponentRegistration, ComponentRegistry};
pub use condition_parser::ConditionParser;
pub use css_link::{CssLinkHref, CssLinkOptions, DEFAULT_CSS_FILE_NAME_TEMPLATE};
pub use css_parser::CssParser;
pub use diagnostic::{codes, Diagnostic, Severity};
pub use error::{ParserError, Result};
pub use handlebars_parser::HandlebarsParser;
pub use webui_tokens::CssFallbackChain;

use crate::component_policy::{
    parse_component_render_policy, HYDRATE_ATTR as COMPONENT_HYDRATE_ATTR,
    RENDER_ATTR as COMPONENT_RENDER_ATTR,
    RESERVE_BLOCK_SIZE_ATTR as COMPONENT_RESERVE_BLOCK_SIZE_ATTR,
};
use crate::html_parser::{self as html, Attrs, Element, Event, Walker};
use crate::plugin::{AttributeAction, ParserPlugin, ParserPluginArtifacts};
use std::collections::{HashMap, HashSet};
use std::ops::Range;
use webui_protocol::{
    web_ui_fragment, web_ui_fragment::Fragment, ConditionExpr, FragmentList, WebUIFragment,
    WebUIFragmentAttribute, WebUIFragmentRecords, WebUiFragmentRoute,
};

/// Maximum template size accepted by the parser.
///
/// Build-time parsing should never need to ingest arbitrarily large single
/// templates. This cap prevents accidental or malicious OOM-scale inputs while
/// staying far above normal component/page sizes.
const MAX_TEMPLATE_BYTES: usize = 16 * 1024 * 1024;

/// Maximum nested HTML element depth accepted by semantic parsing.
///
/// The scanner itself is iterative, but semantic fragment construction still
/// enters child ranges. This limit keeps pathological nesting from exhausting
/// the Rust call stack while preserving generous headroom for real templates.
const MAX_TEMPLATE_DEPTH: usize = 512;

/// Prefix for compiler-owned structural signals.
///
/// `}}}` cannot occur in a parsed double- or triple-brace expression: it closes
/// the binding first. This keeps transport structure distinct from every
/// authored state key without changing the protobuf schema.
const STRUCTURAL_SIGNAL_PREFIX: &str = "}}}webui:";

fn structural_signal(value: impl AsRef<str>) -> WebUIFragment {
    WebUIFragment::signal(
        format!("{STRUCTURAL_SIGNAL_PREFIX}{}", value.as_ref()),
        true,
    )
}

/// How an element affects the HTML insertion mode that decides whether an
/// unknown element (the generated `<webui-hydrate>` sentinel) is
/// foster-parented away from its payload script.
#[derive(Clone, Copy, PartialEq, Eq)]
enum FosterContext {
    /// Does not change the current insertion mode.
    Transparent,
    /// "in table"/"in table body"/"in row"/"in column group"/"in select":
    /// unknown elements are foster-parented out of this subtree.
    Hostile,
    /// `<td>`/`<th>`/`<caption>` return to "in body" rules, so a boundary
    /// inside them is safe even though an ancestor is hostile.
    Barrier,
}

/// Classify `name` for foster-parenting purposes.
///
/// Dispatching on length first means the common non-table element (`div`,
/// `span`, `p`, `section`, …) costs one integer compare and a fallthrough,
/// keeping this off the parser's measured hot path.
#[inline]
fn foster_context_of(name: &str) -> FosterContext {
    match name.len() {
        2 => {
            if name.eq_ignore_ascii_case("tr") {
                FosterContext::Hostile
            } else if name.eq_ignore_ascii_case("td") || name.eq_ignore_ascii_case("th") {
                FosterContext::Barrier
            } else {
                FosterContext::Transparent
            }
        }
        5 => {
            if name.eq_ignore_ascii_case("table")
                || name.eq_ignore_ascii_case("tbody")
                || name.eq_ignore_ascii_case("thead")
                || name.eq_ignore_ascii_case("tfoot")
            {
                FosterContext::Hostile
            } else {
                FosterContext::Transparent
            }
        }
        6 => {
            if name.eq_ignore_ascii_case("select") {
                FosterContext::Hostile
            } else {
                FosterContext::Transparent
            }
        }
        7 => {
            if name.eq_ignore_ascii_case("caption") {
                FosterContext::Barrier
            } else {
                FosterContext::Transparent
            }
        }
        8 => {
            if name.eq_ignore_ascii_case("optgroup") || name.eq_ignore_ascii_case("colgroup") {
                FosterContext::Hostile
            } else {
                FosterContext::Transparent
            }
        }
        _ => FosterContext::Transparent,
    }
}

/// Return the browser parsing context that prevents a nested boundary sentinel
/// from becoming an active custom element.
#[inline]
fn boundary_parent_scope(name: &str) -> Option<&'static str> {
    match name.len() {
        3 if name.eq_ignore_ascii_case("xmp") => Some("<xmp> raw-text content"),
        5 if name.eq_ignore_ascii_case("title") => Some("<title> text content"),
        6 if name.eq_ignore_ascii_case("iframe") => Some("<iframe> raw-text content"),
        6 if name.eq_ignore_ascii_case("script") => Some("<script> raw-text content"),
        7 if name.eq_ignore_ascii_case("noembed") => Some("<noembed> raw-text content"),
        8 if name.eq_ignore_ascii_case("template") => Some("<template> inert content"),
        8 if name.eq_ignore_ascii_case("textarea") => Some("<textarea> text content"),
        8 if name.eq_ignore_ascii_case("noframes") => Some("<noframes> raw-text content"),
        8 if name.eq_ignore_ascii_case("noscript") => Some("<noscript> inert content"),
        9 if name.eq_ignore_ascii_case("plaintext") => Some("<plaintext> text content"),
        _ => None,
    }
}

/// Strategy for how component CSS is delivered in rendered output.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[cfg_attr(feature = "cli", derive(clap::ValueEnum))]
pub enum CssStrategy {
    /// Emit `<link rel="stylesheet" href="component.css">` tags (default).
    #[default]
    Link,
    /// Embed CSS content in `<style>` tags within the component.
    Style,
    /// Register each component's CSS module as a `<script type="importmap">`
    /// data-URI entry. The client runtime applies the registered sheet via
    /// the document's adopted stylesheets.
    Module,
}

impl std::fmt::Display for CssStrategy {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CssStrategy::Link => write!(f, "link"),
            CssStrategy::Style => write!(f, "style"),
            CssStrategy::Module => write!(f, "module"),
        }
    }
}

impl std::str::FromStr for CssStrategy {
    type Err = String;

    fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
        match s {
            "link" => Ok(CssStrategy::Link),
            "style" => Ok(CssStrategy::Style),
            "module" => Ok(CssStrategy::Module),
            other => Err(format!(
                "Unknown CSS strategy: {other}. Use \"link\", \"style\", or \"module\"."
            )),
        }
    }
}

/// Strategy for how component DOM is structured.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[cfg_attr(feature = "cli", derive(clap::ValueEnum))]
pub enum DomStrategy {
    /// Use shadow DOM with declarative shadow roots for SSR (default).
    #[default]
    Shadow,
    /// Use light DOM — component content is rendered as direct children.
    Light,
}

impl std::fmt::Display for DomStrategy {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DomStrategy::Shadow => write!(f, "shadow"),
            DomStrategy::Light => write!(f, "light"),
        }
    }
}

impl std::str::FromStr for DomStrategy {
    type Err = String;

    fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
        match s {
            "shadow" => Ok(DomStrategy::Shadow),
            "light" => Ok(DomStrategy::Light),
            other => Err(format!(
                "Unknown DOM strategy: {other}. Use \"shadow\" or \"light\"."
            )),
        }
    }
}

/// Strategy for preserving legal comments in generated output.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[cfg_attr(feature = "cli", derive(clap::ValueEnum))]
pub enum LegalComments {
    /// Strip every HTML and CSS comment.
    None,
    /// Preserve legal CSS comments inline and strip all other comments.
    #[default]
    Inline,
}

impl std::fmt::Display for LegalComments {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LegalComments::None => write!(f, "none"),
            LegalComments::Inline => write!(f, "inline"),
        }
    }
}

impl std::str::FromStr for LegalComments {
    type Err = String;

    fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
        match s {
            "none" => Ok(LegalComments::None),
            "inline" => Ok(LegalComments::Inline),
            other => Err(format!(
                "Unknown legal comments strategy: {other}. Use \"none\" or \"inline\"."
            )),
        }
    }
}

/// Build/output options that affect parser-generated component templates.
#[derive(Debug, Clone, Default)]
pub struct ParserOptions {
    /// Strategy for how component CSS is delivered.
    pub css_strategy: CssStrategy,
    /// Strategy for how component DOM is rendered.
    pub dom_strategy: DomStrategy,
    /// Link-mode CSS filename/href options.
    pub css_link_options: CssLinkOptions,
    /// Legal comment preservation strategy.
    pub legal_comments: LegalComments,
}

impl ParserOptions {
    /// Create parser output options.
    ///
    /// # Errors
    ///
    /// Returns [`ParserError::Css`] when Link-mode CSS link options are invalid.
    pub fn try_new(
        css_strategy: CssStrategy,
        dom_strategy: DomStrategy,
        css_file_name_template: &str,
        css_public_base: Option<&str>,
        legal_comments: LegalComments,
    ) -> Result<Self> {
        let css_link_options = if css_strategy == CssStrategy::Link {
            CssLinkOptions::try_new(
                css_file_name_template.to_string(),
                css_public_base.map(std::string::ToString::to_string),
            )?
        } else {
            CssLinkOptions::default()
        };

        Ok(Self {
            css_strategy,
            dom_strategy,
            css_link_options,
            legal_comments,
        })
    }
}

impl From<CssStrategy> for ParserOptions {
    fn from(css_strategy: CssStrategy) -> Self {
        Self {
            css_strategy,
            ..Self::default()
        }
    }
}

impl From<DomStrategy> for ParserOptions {
    fn from(dom_strategy: DomStrategy) -> Self {
        Self {
            dom_strategy,
            ..Self::default()
        }
    }
}

impl From<(CssStrategy, DomStrategy)> for ParserOptions {
    fn from((css_strategy, dom_strategy): (CssStrategy, DomStrategy)) -> Self {
        Self {
            css_strategy,
            dom_strategy,
            ..Self::default()
        }
    }
}

/// Framework plugin to load for build-time and render-time processing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "cli", derive(clap::ValueEnum))]
pub enum Plugin {
    /// Deprecated compatibility alias for the FAST 2 hydration plugin.
    #[cfg_attr(feature = "cli", value(name = "fast"))]
    Fast,
    /// Deprecated FAST 2 hydration plugin with legacy marker output.
    #[cfg_attr(feature = "cli", value(name = "fast-v2"))]
    FastV2,
    /// FAST 3 hydration plugin with compact marker output.
    #[cfg_attr(feature = "cli", value(name = "fast-v3"))]
    FastV3,
    /// WebUI plugin — full component model with shadow DOM support.
    #[cfg_attr(feature = "cli", value(name = "webui"))]
    WebUI,
}

impl std::fmt::Display for Plugin {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Plugin::Fast => write!(f, "fast"),
            Plugin::FastV2 => write!(f, "fast-v2"),
            Plugin::FastV3 => write!(f, "fast-v3"),
            Plugin::WebUI => write!(f, "webui"),
        }
    }
}

impl std::str::FromStr for Plugin {
    type Err = String;

    fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
        match s {
            "fast" => Ok(Plugin::Fast),
            "fast-v2" => Ok(Plugin::FastV2),
            "fast-v3" => Ok(Plugin::FastV3),
            "webui" => Ok(Plugin::WebUI),
            other => Err(format!(
                "Unknown plugin: {other}. Use \"webui\", \"fast-v3\", \"fast-v2\", or \"fast\"."
            )),
        }
    }
}

/// Counter for generating unique fragment IDs.
struct FragmentIdCounter {
    /// Map of counter types to their current values.
    counters: HashMap<String, usize>,
}

impl FragmentIdCounter {
    /// Create a new fragment ID counter.
    fn new() -> Self {
        Self {
            counters: HashMap::new(),
        }
    }

    /// Generate a unique fragment ID.
    fn next_id(&mut self, prefix: &str) -> String {
        let count = self.counters.entry(prefix.to_string()).or_insert(0);
        *count += 1;
        format!("{}-{}", prefix, count)
    }
}

struct ParseContext {
    fragments: Vec<WebUIFragment>,
    raw_buffer: String,
}

#[derive(Default)]
struct FragmentCssTokens {
    definitions: Vec<String>,
    fallback_chains: Vec<CssFallbackChain>,
}

/// CSS token analysis produced from the parsed template/component graph.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CssTokenAnalysis {
    /// Sorted, deduplicated token candidates stored in the protocol.
    pub protocol_tokens: Vec<String>,
    /// Fallback chains that still need theme/literal coverage after local and
    /// ancestor custom-property definitions are considered.
    pub fallback_chains: Vec<CssFallbackChain>,
    /// Source location of each unresolved token's first `var()` usage, keyed by
    /// token name. Used to point theme-validation diagnostics at the offending
    /// CSS. Build-time only; never serialized.
    pub(crate) token_sites: HashMap<String, TokenSite>,
}

/// Where an unresolved CSS token is first referenced, for diagnostics.
///
/// `owner` is a file-like label (`my-card.css`, or the entry file for inline
/// `<style>`). `position`/`snippet` are populated for component CSS where the
/// source is a standalone file; inline styles record the owner only.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct TokenSite {
    owner: String,
    position: Option<(usize, usize)>,
    snippet: Option<String>,
}

enum ParseOp<'a> {
    Parse {
        range: Range<usize>,
        depth: usize,
    },
    EmitClose(&'a str),
    EndHead(&'a str),
    EndBody(&'a str),
    /// Close a `<boundary>` opened with response-local sequence
    /// number `seq`: emits the `boundary_end:<seq>` raw signal and clears
    /// [`HtmlParser::in_boundary`].
    EndBoundary(u32),
    /// Restore [`HtmlParser::foster_context_depth`] to the saved value when
    /// leaving an element that changed it.
    ///
    /// Save/restore of the single overwritten value keeps ancestor tracking
    /// allocation-free: no ancestor stack is retained.
    RestoreFosterDepth(u32),
    /// Restore the enclosing component/raw/inert context after its child range
    /// has been parsed.
    RestoreBoundaryParentScope(Option<&'static str>),
    CompleteFor {
        parent: ParseContext,
        item: String,
        collection: String,
        fragment_id: String,
        keep_empty: bool,
    },
    CompleteIf {
        parent: ParseContext,
        condition: ConditionExpr,
        fragment_id: String,
    },
}

enum TokenGraphOp<'a> {
    EnterFragment(&'a str),
    EnterComponent(&'a str),
    EnterRoute(&'a WebUiFragmentRoute),
    ExitDefinitions(&'a [String]),
}

impl Default for HtmlParser {
    fn default() -> Self {
        Self::new()
    }
}

/// Parser for WebUI directives.
pub struct HtmlParser {
    /// CSS parser.
    css_parser: CssParser,

    /// Fragment ID counter.
    id_counter: FragmentIdCounter,

    /// Condition parser for parsing conditions in directives.
    condition_parser: ConditionParser,

    /// Handlebars parser for parsing handlebars expressions.
    handlebars_parser: HandlebarsParser,

    /// Component registry for WebUI components.
    component_registry: ComponentRegistry,

    /// Map of fragment IDs to their fragments
    fragment_records: WebUIFragmentRecords,

    /// Buffer for accumulating raw content
    raw_buffer: String,

    /// Parser output options for generated component templates.
    options: ParserOptions,

    /// Optional parser plugin for framework-specific behavior.
    plugin: Option<Box<dyn ParserPlugin>>,

    /// Top-level fragments parsed by callers. Token graph traversal starts
    /// from these roots after parsing completes.
    token_roots: Vec<String>,

    /// CSS custom property definitions and fallback chains from inline
    /// `<style>` tags, keyed by owning fragment id.
    fragment_css_tokens: HashMap<String, FragmentCssTokens>,

    /// Fragment IDs currently being parsed, used to reject recursive component
    /// references before they can recurse through template parsing.
    in_progress_fragments: HashSet<String>,

    /// The fragment ID (entry file or component tag) currently being parsed.
    /// Used to name the owning template in authoring [`Diagnostic`]s.
    current_fragment_id: String,

    /// Next `<boundary>` response-local sequence number
    /// (`boundary_start:<seq>` / `boundary_end:<seq>`). Reset to `0` at the
    /// start of each top-level [`HtmlParser::parse`] call.
    boundary_sequence: u32,

    /// `<boundary name>` values already used in the current top-level
    /// parse, so a duplicate name can be rejected.
    boundary_names: HashSet<String>,

    /// `<boundary name>` values in declaration order for the current entry.
    /// The build stores this table in `protocol.bin` for one cold host lookup.
    boundary_names_in_order: Vec<String>,

    /// `true` while parsing is inside an open `<boundary>`, used to
    /// reject a nested boundary.
    in_boundary: bool,

    /// Number of currently open native `<body>` elements. A depth rather than
    /// a boolean preserves the parent state while recursively parsing a
    /// component template.
    body_depth: usize,

    /// Stack of open `<if>`/`<for>` scope descriptions, used to reject a
    /// `<boundary>` that would cut through a conditionally-rendered or
    /// repeated scope instead of independently wrapping it.
    structural_scopes: Vec<&'static str>,

    /// Number of enclosing elements whose HTML insertion mode would
    /// foster-parent an unknown element such as the generated
    /// `<webui-hydrate>` sentinel.
    ///
    /// Non-zero means a `<boundary>` here would be split from its
    /// payload script by the browser's parser, so it must be rejected at build
    /// time. `<td>`/`<th>`/`<caption>` reset it to `0` because those switch
    /// back to "in body" insertion rules.
    foster_context_depth: u32,

    /// Enclosing component host content or native raw/inert HTML context.
    ///
    /// A generated sentinel in any of these contexts is either text or lives
    /// in an inert fragment, so it cannot commit a streaming boundary.
    boundary_parent_scope: Option<&'static str>,

    /// `src` of every authored `<script type="module">` outside any
    /// `<boundary>`, in document order, from the current top-level parse.
    ///
    /// These are the page's critical module entries. The build resolves each
    /// against the projection manifest to emit `<link rel="modulepreload">`
    /// for the shared chunks they statically import, which the browser's
    /// preload scanner cannot discover on its own.
    ///
    /// Scripts *inside* a boundary are deliberately excluded: an island loader
    /// is meant to be requested only when the parser reaches its chunk, so
    /// preloading it would undo the deferral the author asked for.
    module_entry_srcs: Vec<String>,
}

struct BuiltComponentTemplate {
    ssr: String,
    artifact: Option<String>,
}

#[derive(Clone, Copy)]
struct ComponentTemplateMode {
    preserve_runtime_attrs: bool,
    policy_wrapper: bool,
}

impl BuiltComponentTemplate {
    fn artifact(&self) -> &str {
        self.artifact.as_deref().unwrap_or(&self.ssr)
    }
}

struct ComponentTemplateInput<'a> {
    tag_name: &'a str,
    html: &'a str,
    artifact_html: Option<&'a str>,
    css_content: Option<&'a str>,
    artifact_needed: bool,
}

fn add_token_definitions(definitions: &[String], available_counts: &mut HashMap<String, usize>) {
    for definition in definitions {
        let count = available_counts.entry(definition.clone()).or_insert(0);
        *count += 1;
    }
}

fn remove_token_definitions(definitions: &[String], available_counts: &mut HashMap<String, usize>) {
    for definition in definitions {
        if let Some(count) = available_counts.get_mut(definition) {
            if *count == 1 {
                available_counts.remove(definition);
            } else {
                *count -= 1;
            }
        }
    }
}

/// Accumulators for the token-graph walk: the unresolved fallback chains and
/// the per-token source locations, grouped so walk helpers stay within the
/// argument-count budget.
#[derive(Default)]
struct UnresolvedTokens {
    chains: Vec<CssFallbackChain>,
    sites: HashMap<String, TokenSite>,
}

fn record_unresolved_requirements(
    source: &[CssFallbackChain],
    available_counts: &HashMap<String, usize>,
    owner: &str,
    css_source: Option<&str>,
    out: &mut UnresolvedTokens,
) {
    for requirement in source {
        let tokens: Vec<String> = requirement
            .tokens
            .iter()
            .filter(|token| !available_counts.contains_key(*token))
            .cloned()
            .collect();
        if tokens.is_empty() {
            continue;
        }
        for token in &tokens {
            if !out.sites.contains_key(token) {
                out.sites
                    .insert(token.clone(), token_site(owner, css_source, token));
            }
        }
        out.chains.push(CssFallbackChain {
            tokens,
            has_literal_fallback: requirement.has_literal_fallback,
        });
    }
}

/// Build the [`TokenSite`] for `token`. When `css_source` is available (a
/// component's standalone CSS), the token's `var()` usage is located for a
/// precise `line:column` and snippet; otherwise only the `owner` is recorded.
fn token_site(owner: &str, css_source: Option<&str>, token: &str) -> TokenSite {
    let (position, snippet) = match css_source.and_then(|css| locate_css_token(css, token)) {
        Some((line, column, snippet)) => (Some((line, column)), Some(snippet)),
        None => (None, None),
    };
    TokenSite {
        owner: owner.to_string(),
        position,
        snippet,
    }
}

/// Locate the first `var(--token)` **usage** (not a `--token:` definition) in
/// `css`, returning its 1-based `(line, column)` and a one-line snippet.
///
/// Iterative byte scan — no regex, no recursion. Cold-ish (build time, only for
/// unresolved tokens).
fn locate_css_token(css: &str, token: &str) -> Option<(usize, usize, String)> {
    let bytes = css.as_bytes();
    let name = token.as_bytes();
    let mut index = 0;
    while index + 2 + name.len() <= bytes.len() {
        let is_prefixed = bytes[index] == b'-'
            && bytes[index + 1] == b'-'
            && bytes.get(index + 2..index + 2 + name.len()) == Some(name);
        if is_prefixed {
            let after = index + 2 + name.len();
            let is_exact_name = bytes
                .get(after)
                .is_none_or(|b| !(b.is_ascii_alphanumeric() || *b == b'-' || *b == b'_'));
            if is_exact_name {
                let mut cursor = after;
                while cursor < bytes.len() && bytes[cursor].is_ascii_whitespace() {
                    cursor += 1;
                }
                // `--token:` is a definition, not the usage we want to point at.
                if bytes.get(cursor) != Some(&b':') {
                    let (line, column) = diagnostic::line_column(css, index);
                    return Some((line, column, css_line_snippet(css, index)));
                }
            }
        }
        index += 1;
    }
    None
}

/// Extract the trimmed line containing byte `offset`, capped so a minified
/// stylesheet does not produce a wall-of-text snippet.
fn css_line_snippet(css: &str, offset: usize) -> String {
    const MAX_SNIPPET: usize = 80;
    let start = css[..offset].rfind('\n').map_or(0, |n| n + 1);
    let end = css[offset..].find('\n').map_or(css.len(), |n| offset + n);
    let line = css[start..end].trim();
    if line.chars().count() > MAX_SNIPPET {
        let truncated: String = line.chars().take(MAX_SNIPPET).collect();
        format!("{truncated}…")
    } else {
        line.to_string()
    }
}

/// File-like owner label for a component's standalone CSS
/// (`my-card` → `my-card.css`), used as the `--> owner:line:column` prefix.
fn css_owner_label(tag_name: &str) -> String {
    let mut label = String::with_capacity(tag_name.len() + 4);
    label.push_str(tag_name);
    label.push_str(".css");
    label
}

impl CssTokenAnalysis {
    /// Validate the required CSS tokens against a loaded design-token theme.
    ///
    /// A token is *required* in every theme only when it appears in at least one
    /// unresolved `var()` chain with **no** literal CSS fallback. A chain such as
    /// `var(--x, 16px)` provides its own value, so `--x` stays in
    /// [`protocol_tokens`](Self::protocol_tokens) for runtime resolution (the
    /// theme value still wins when present) but does not fail the build when the
    /// theme omits it. The chain/theme policy lives in
    /// [`webui_tokens::validate_chain_tokens`]; this only adapts its
    /// [`webui_tokens::TokenError`] into a structured [`Diagnostic`].
    ///
    /// # Errors
    ///
    /// Returns [`ParserError::Template`] with a stable diagnostic code when a
    /// required token is missing from a theme. Theme token values are trusted and
    /// their transitive references are left to browser CSS semantics.
    pub fn validate_theme_tokens(&self, theme: &webui_tokens::TokenFile) -> Result<()> {
        webui_tokens::validate_chain_tokens(&self.fallback_chains, theme)
            .map_err(|source| self.theme_token_error(source, theme))
    }

    /// CSS tokens used **only** with a literal `var()` fallback and defined in no
    /// theme — likely typos that callers may surface as non-fatal advisories.
    ///
    /// Thin wrapper over [`webui_tokens::unthemed_literal_fallback_tokens`].
    #[must_use]
    pub fn unthemed_literal_fallback_tokens(&self, theme: &webui_tokens::TokenFile) -> Vec<String> {
        webui_tokens::unthemed_literal_fallback_tokens(&self.fallback_chains, theme)
    }

    /// Structured, color-free advisories for CSS tokens used only with a
    /// literal `var()` fallback and defined in no theme — almost always typos.
    ///
    /// Returned as warning-severity [`Diagnostic`]s so hosts render them with
    /// the same location/snippet/`help:` layout as errors (the CLI colorizes;
    /// other hosts use the plain [`Diagnostic::body`]). Each carries the token's
    /// source location and a `did you mean --…?` suggestion.
    #[must_use]
    pub fn theme_token_warnings(&self, theme: &webui_tokens::TokenFile) -> Vec<Diagnostic> {
        let mut warnings = Vec::new();
        for token in webui_tokens::unthemed_literal_fallback_tokens(&self.fallback_chains, theme) {
            let help = match closest_theme_token(&token, theme.themes.values()) {
                Some(suggestion) => {
                    format!("did you mean --{suggestion}? otherwise the literal fallback is used")
                }
                None => format!(
                    "define --{token} in the theme, or keep its literal fallback if intentional"
                ),
            };
            let diag = self.locate_diagnostic(
                Diagnostic::warning(format!("unthemed CSS token --{token}"))
                    .code(codes::UNTHEMED_TOKEN)
                    .help(help),
                &token,
            );
            warnings.push(diag);
        }
        warnings
    }

    /// Adapt a [`webui_tokens::TokenError`] into a structured [`Diagnostic`],
    /// enriching it with the token's source location and a `did you mean …?`
    /// suggestion drawn from the theme's own tokens.
    #[cold]
    #[inline(never)]
    fn theme_token_error(
        &self,
        source: webui_tokens::TokenError,
        theme: &webui_tokens::TokenFile,
    ) -> ParserError {
        match source {
            webui_tokens::TokenError::MissingToken {
                theme: theme_name,
                token,
            } => {
                // The error title + snippet already say the token is missing
                // from the theme, so the help only adds what isn't obvious: the
                // likely-typo suggestion and the local-definition escape hatch.
                let help =
                    match closest_theme_token(&token, theme.themes.get(&theme_name).into_iter()) {
                        Some(suggestion) => {
                            format!("did you mean --{suggestion}? otherwise define it locally")
                        }
                        None => {
                            "define it locally if it should not come from the theme".to_string()
                        }
                    };
                self.locate_diagnostic(
                    Diagnostic::error("missing theme token")
                        .code(codes::MISSING_THEME_TOKEN)
                        .help(help),
                    &token,
                )
                .into()
            }
            other => ParserError::Generic(format!("Theme token validation failed: {other}")),
        }
    }

    /// Attach `token`'s recorded source location (owner, `line:column`, snippet)
    /// to `diag`. Falls back to a `--token` snippet when no location is known.
    fn locate_diagnostic(&self, diag: Diagnostic, token: &str) -> Diagnostic {
        let Some(site) = self.token_sites.get(token) else {
            return diag.snippet(format!("--{token}"));
        };
        let mut diag = diag.component(site.owner.clone());
        if let Some((line, column)) = site.position {
            diag = diag.position(line, column);
        }
        match &site.snippet {
            Some(snippet) => diag.snippet(snippet.clone()),
            None => diag.snippet(format!("--{token}")),
        }
    }
}

/// Closest theme token to `target` by Levenshtein distance across the given
/// theme token maps, considering each theme's keys. Sorted for determinism.
#[cold]
#[inline(never)]
fn closest_theme_token<'a>(
    target: &str,
    themes: impl Iterator<Item = &'a std::collections::HashMap<String, String>>,
) -> Option<String> {
    let mut names: Vec<&str> = themes
        .flat_map(|theme| theme.keys().map(String::as_str))
        .collect();
    names.sort_unstable();
    names.dedup();
    crate::suggest::closest_match(target, names.into_iter()).map(ToOwned::to_owned)
}

impl HtmlParser {
    /// Create a new directive parser with default parser options.
    #[must_use]
    pub fn new() -> Self {
        Self::with_options(ParserOptions::default())
    }

    /// Create a new directive parser with explicit parser options.
    #[must_use]
    pub fn with_options(options: impl Into<ParserOptions>) -> Self {
        let options = options.into();

        Self {
            component_registry: ComponentRegistry::with_legal_comments(options.legal_comments),
            css_parser: CssParser::new(),
            id_counter: FragmentIdCounter::new(),
            condition_parser: ConditionParser::new(),
            handlebars_parser: HandlebarsParser::new(),
            raw_buffer: String::new(),
            fragment_records: WebUIFragmentRecords::new(),
            options,
            plugin: None,
            token_roots: Vec::new(),
            fragment_css_tokens: HashMap::new(),
            in_progress_fragments: HashSet::new(),
            current_fragment_id: String::new(),
            boundary_sequence: 0,
            boundary_names: HashSet::new(),
            boundary_names_in_order: Vec::new(),
            in_boundary: false,
            body_depth: 0,
            structural_scopes: Vec::new(),
            foster_context_depth: 0,
            boundary_parent_scope: None,
            module_entry_srcs: Vec::new(),
        }
    }

    /// Create a new parser with a plugin and default parser options.
    #[must_use]
    pub fn with_plugin(plugin: Box<dyn ParserPlugin>) -> Self {
        Self::with_plugin_options(plugin, ParserOptions::default())
    }

    /// Create a new parser with a plugin and explicit parser options.
    #[must_use]
    pub fn with_plugin_options(
        plugin: Box<dyn ParserPlugin>,
        options: impl Into<ParserOptions>,
    ) -> Self {
        let mut p = Self::with_options(options);
        p.component_registry
            .set_component_source_transform(plugin.component_source_transform());
        p.plugin = Some(plugin);
        p.configure_plugin();
        p
    }

    fn configure_plugin(&mut self) {
        if let Some(ref mut plugin) = self.plugin {
            plugin.configure(&self.options);
        }
    }

    /// Get a mutable reference to the component registry.
    pub fn component_registry_mut(&mut self) -> &mut ComponentRegistry {
        &mut self.component_registry
    }

    /// Get a shared reference to the component registry.
    #[must_use]
    pub fn component_registry(&self) -> &ComponentRegistry {
        &self.component_registry
    }

    pub fn into_fragment_records(mut self) -> WebUIFragmentRecords {
        std::mem::take(&mut self.fragment_records)
    }

    /// Check if a fragment ID has been parsed (exists in the fragment records).
    pub fn has_fragment(&self, fragment_id: &str) -> bool {
        self.fragment_records.contains_key(fragment_id)
    }

    /// Number of `<boundary>` streaming checkpoints compiled by the most
    /// recent top-level [`HtmlParser::parse`] call.
    ///
    /// Returns `0` for an ordinary, non-streaming entry. Callers use this to
    /// decide whether streaming-specific build advisories apply.
    #[must_use]
    pub fn boundary_count(&self) -> usize {
        self.boundary_sequence as usize
    }

    /// Free-form `<boundary name>` values in declaration order from the most
    /// recent top-level [`HtmlParser::parse`] call.
    #[must_use]
    pub fn boundary_names(&self) -> &[String] {
        &self.boundary_names_in_order
    }

    /// `src` of every authored `<script type="module">` outside any
    /// `<boundary>`, in document order, from the most recent top-level
    /// [`HtmlParser::parse`] call.
    ///
    /// These are the page's critical module entries — the ones whose shared
    /// chunks belong in `<link rel="modulepreload">`. Island loaders placed
    /// inside a boundary are excluded, because deferring them is the point.
    ///
    /// A `src` containing `{{` is skipped: it is a binding resolved per
    /// request, so no build-time artifact can be matched to it.
    #[must_use]
    pub fn module_entry_srcs(&self) -> &[String] {
        &self.module_entry_srcs
    }

    /// Take any post-parse artifacts captured by the parser plugin.
    ///
    /// # Errors
    ///
    /// Returns [`ParserError::Template`] if a tracked component contains an
    /// invalid `@event` handler or a non-braced `w-ref` binding.
    pub fn take_plugin_artifacts(&mut self) -> Result<ParserPluginArtifacts> {
        self.plugin
            .take()
            .map_or(Ok(ParserPluginArtifacts::None), |plugin| {
                plugin.into_artifacts()
            })
    }

    /// Take the accumulated CSS tokens as a sorted, deduplicated `Vec`.
    ///
    /// Convenience wrapper around [`HtmlParser::token_analysis`].
    #[must_use]
    pub fn take_tokens(&mut self) -> Vec<String> {
        self.token_analysis().protocol_tokens
    }

    /// Analyze CSS token requirements from the parsed fragment/component graph.
    ///
    /// Each token candidate in a `var()` fallback chain is removed when that
    /// token is defined by CSS in the current fragment or an ancestor
    /// component/root. The returned protocol token list is sorted and
    /// deduplicated from the remaining unresolved fallback-chain candidates.
    #[must_use]
    pub fn token_analysis(&self) -> CssTokenAnalysis {
        let (fallback_chains, token_sites) = self.collect_unresolved_fallback_chains();
        let mut protocol_token_set = HashSet::new();
        for chain in &fallback_chains {
            for token in &chain.tokens {
                protocol_token_set.insert(token.clone());
            }
        }
        let mut protocol_tokens: Vec<String> = protocol_token_set.into_iter().collect();
        protocol_tokens.sort();
        CssTokenAnalysis {
            protocol_tokens,
            fallback_chains,
            token_sites,
        }
    }

    fn collect_unresolved_fallback_chains(
        &self,
    ) -> (Vec<CssFallbackChain>, HashMap<String, TokenSite>) {
        let mut out = UnresolvedTokens::default();
        let mut available_counts: HashMap<String, usize> = HashMap::new();
        let mut ops: Vec<TokenGraphOp<'_>> = Vec::with_capacity(self.token_roots.len());
        for root in self.token_roots.iter().rev() {
            ops.push(TokenGraphOp::EnterFragment(root.as_str()));
        }

        while let Some(op) = ops.pop() {
            match op {
                TokenGraphOp::EnterFragment(fragment_id) => {
                    self.enter_token_fragment(
                        fragment_id,
                        &mut available_counts,
                        &mut out,
                        &mut ops,
                    );
                }
                TokenGraphOp::EnterComponent(tag_name) => {
                    self.enter_token_component(tag_name, &mut available_counts, &mut out, &mut ops);
                }
                TokenGraphOp::EnterRoute(route) => {
                    self.enter_token_route(route, &mut available_counts, &mut out, &mut ops);
                }
                TokenGraphOp::ExitDefinitions(definitions) => {
                    remove_token_definitions(definitions, &mut available_counts);
                }
            }
        }

        (out.chains, out.sites)
    }

    fn enter_token_fragment<'a>(
        &'a self,
        fragment_id: &'a str,
        available_counts: &mut HashMap<String, usize>,
        out: &mut UnresolvedTokens,
        ops: &mut Vec<TokenGraphOp<'a>>,
    ) {
        if let Some(css) = self.fragment_css_tokens.get(fragment_id) {
            add_token_definitions(&css.definitions, available_counts);
            // Inline `<style>` tokens record their owning fragment (the entry
            // file). A fragment may carry several `<style>` blocks, so an
            // offset into one body is ambiguous — record the owner only.
            record_unresolved_requirements(
                &css.fallback_chains,
                available_counts,
                fragment_id,
                None,
                out,
            );
            ops.push(TokenGraphOp::ExitDefinitions(&css.definitions));
        }

        let Some(fragments) = self.fragment_records.get(fragment_id) else {
            return;
        };
        for fragment in fragments.fragments.iter().rev() {
            match fragment.fragment.as_ref() {
                Some(web_ui_fragment::Fragment::Component(component)) => {
                    ops.push(TokenGraphOp::EnterComponent(component.fragment_id.as_str()));
                }
                Some(web_ui_fragment::Fragment::ForLoop(for_loop)) => {
                    ops.push(TokenGraphOp::EnterFragment(for_loop.fragment_id.as_str()));
                }
                Some(web_ui_fragment::Fragment::IfCond(if_cond)) => {
                    ops.push(TokenGraphOp::EnterFragment(if_cond.fragment_id.as_str()));
                }
                Some(web_ui_fragment::Fragment::Route(route)) => {
                    ops.push(TokenGraphOp::EnterRoute(route));
                }
                _ => {}
            }
        }
    }

    fn enter_token_component<'a>(
        &'a self,
        tag_name: &'a str,
        available_counts: &mut HashMap<String, usize>,
        out: &mut UnresolvedTokens,
        ops: &mut Vec<TokenGraphOp<'a>>,
    ) {
        let Some(component) = self.component_registry.get(tag_name) else {
            return;
        };
        add_token_definitions(&component.css_definitions, available_counts);
        record_unresolved_requirements(
            &component.css_fallback_chains,
            available_counts,
            &css_owner_label(tag_name),
            component.css_content.as_deref(),
            out,
        );
        ops.push(TokenGraphOp::ExitDefinitions(&component.css_definitions));
        ops.push(TokenGraphOp::EnterFragment(tag_name));
    }

    fn enter_token_route<'a>(
        &'a self,
        route: &'a WebUiFragmentRoute,
        available_counts: &mut HashMap<String, usize>,
        out: &mut UnresolvedTokens,
        ops: &mut Vec<TokenGraphOp<'a>>,
    ) {
        let Some(component) = self.component_registry.get(&route.fragment_id) else {
            return;
        };
        add_token_definitions(&component.css_definitions, available_counts);
        record_unresolved_requirements(
            &component.css_fallback_chains,
            available_counts,
            &css_owner_label(&route.fragment_id),
            component.css_content.as_deref(),
            out,
        );
        ops.push(TokenGraphOp::ExitDefinitions(&component.css_definitions));
        if !route.error_component.is_empty() {
            ops.push(TokenGraphOp::EnterComponent(route.error_component.as_str()));
        }
        if !route.pending_component.is_empty() {
            ops.push(TokenGraphOp::EnterComponent(
                route.pending_component.as_str(),
            ));
        }
        for child in route.children.iter().rev() {
            ops.push(TokenGraphOp::EnterRoute(child));
        }
        ops.push(TokenGraphOp::EnterFragment(route.fragment_id.as_str()));
    }

    fn record_fragment_css_tokens(
        &mut self,
        definitions: HashSet<String>,
        fallback_chains: Vec<CssFallbackChain>,
    ) {
        let css = self
            .fragment_css_tokens
            .entry(self.current_fragment_id.clone())
            .or_default();
        css.definitions.extend(definitions);
        css.definitions.sort();
        css.definitions.dedup();
        css.fallback_chains.extend(fallback_chains);
    }

    /// Parse HTML content to generate WebUI fragments.
    pub fn parse(&mut self, fragment_id: &str, html_content: &str) -> Result<()> {
        let fragment_key = fragment_id.to_string();
        let is_token_root = self.in_progress_fragments.is_empty();
        if is_token_root {
            // A new top-level entry template starts a fresh boundary
            // namespace: sequence numbers are response-local (spec: "0, 1,
            // …") and names are only required to be unique per entry. Also
            // reset `in_boundary`/`structural_scopes`, which should already
            // be empty/false here but could be left dirty if a *previous*
            // top-level parse on this same `HtmlParser` errored out midway
            // through an open boundary or `<if>`/`<for>` scope.
            self.boundary_sequence = 0;
            self.boundary_names.clear();
            self.boundary_names_in_order.clear();
            self.in_boundary = false;
            self.body_depth = 0;
            self.structural_scopes.clear();
            self.foster_context_depth = 0;
            self.boundary_parent_scope = None;
            self.module_entry_srcs.clear();
        }
        // Save the caller's fragment id and restore it before returning. A
        // component parse recurses through `enter_component_directive`
        // (`self.parse(child, …)`); without restoring, the parent would keep
        // parsing with `current_fragment_id` pointing at the child, so a later
        // diagnostic in the parent would be attributed to the wrong owner.
        let previous_fragment_id =
            std::mem::replace(&mut self.current_fragment_id, fragment_key.clone());
        if !self.in_progress_fragments.insert(fragment_key.clone()) {
            let err = self
                .authoring_error(
                    codes::RECURSIVE_TEMPLATE,
                    format!("recursive template reference while parsing <{fragment_id}>"),
                )
                .help(
                    "move the recursive usage behind runtime data, or split the component graph \
                     so templates do not reference themselves at build time",
                )
                .into();
            self.current_fragment_id = previous_fragment_id;
            return Err(err);
        }
        if is_token_root && !self.token_roots.contains(&fragment_key) {
            self.token_roots.push(fragment_key.clone());
        }

        let result = self.parse_inner(fragment_id, html_content);
        self.in_progress_fragments.remove(&fragment_key);
        self.current_fragment_id = previous_fragment_id;
        result
    }

    fn parse_inner(&mut self, fragment_id: &str, html_content: &str) -> Result<()> {
        if html_content.len() > MAX_TEMPLATE_BYTES {
            return Err(ParserError::Html(format!(
                "Template '{fragment_id}' is {} bytes, which exceeds the {MAX_TEMPLATE_BYTES} byte parser limit. Split very large templates into components or reduce generated markup before build.",
                html_content.len()
            )));
        }

        // Reset sub-fragments for new parse
        self.raw_buffer.clear();
        if let Some(ref mut plugin) = self.plugin {
            plugin.start_fragment(fragment_id);
        }

        let mut entry_fragment: Vec<WebUIFragment> = Vec::new();
        self.parse_range(html_content, 0..html_content.len(), &mut entry_fragment, 0)?;

        self.flush_raw_buffer(&mut entry_fragment);

        self.fragment_records.insert(
            fragment_id.to_string(),
            FragmentList {
                fragments: entry_fragment,
            },
        );

        Ok(())
    }

    /// Add raw content to the buffer
    fn add_raw_fragment(&mut self, content: &str) {
        if !content.is_empty() {
            self.raw_buffer.push_str(content);
        }
    }

    /// Add a for fragment, flushing raw buffer first
    fn add_for_fragment(
        &mut self,
        item: String,
        collection: String,
        fragment_id: String,
        fragments: &mut Vec<WebUIFragment>,
    ) {
        self.flush_raw_buffer(fragments);
        fragments.push(WebUIFragment::for_loop(item, collection, fragment_id));
    }

    /// Add an if fragment, flushing raw buffer first
    fn add_if_fragment(
        &mut self,
        condition: ConditionExpr,
        fragment_id: String,
        fragments: &mut Vec<WebUIFragment>,
    ) {
        self.flush_raw_buffer(fragments);
        fragments.push(WebUIFragment::if_cond(condition, fragment_id));
    }

    /// Add a non-raw fragment, flushing the raw buffer first if needed
    fn add_fragment(&mut self, fragment: WebUIFragment, fragments: &mut Vec<WebUIFragment>) {
        self.flush_raw_buffer(fragments);
        fragments.push(fragment);
    }

    /// Flush the raw buffer into fragments if not empty
    fn flush_raw_buffer(&mut self, fragments: &mut Vec<WebUIFragment>) {
        if !self.raw_buffer.is_empty() {
            fragments.push(WebUIFragment::raw(std::mem::take(&mut self.raw_buffer)));
        }
    }

    /// Returns true when a text node should be emitted into the fragment stream.
    ///
    /// Pure formatting runs that contain line breaks are still dropped, but
    /// inline whitespace-only separators such as the spaces around `&gt;` in
    /// `{{sectionName}} &gt; {{topicName}}` must be preserved.
    fn should_emit_text_content(content: &str) -> bool {
        if content.is_empty() {
            return false;
        }

        if !content.trim().is_empty() {
            return true;
        }

        content.chars().all(char::is_whitespace)
            && !content.contains('\n')
            && !content.contains('\r')
    }

    fn parse_range(
        &mut self,
        source: &str,
        range: Range<usize>,
        fragments: &mut Vec<WebUIFragment>,
        depth: usize,
    ) -> Result<()> {
        let mut ops = vec![ParseOp::Parse { range, depth }];

        while let Some(op) = ops.pop() {
            match op {
                ParseOp::Parse { range, depth } => {
                    if depth > MAX_TEMPLATE_DEPTH {
                        return Err(self
                            .html_error(
                                codes::EXCESSIVE_NESTING,
                                format!(
                                    "template nesting exceeds the {MAX_TEMPLATE_DEPTH}-level limit"
                                ),
                                source,
                                range.start,
                            )
                            .help(
                                "split deeply nested markup into components, or reduce generated nesting",
                            )
                            .into());
                    }

                    let end = range.end;
                    let mut index = range.start;
                    while index < end {
                        let remaining = &source[index..end];
                        if remaining.starts_with("<!--") {
                            let Some(close) = html::find_comment_close(remaining) else {
                                return Err(self
                                    .html_error(
                                        codes::UNTERMINATED_HTML_COMMENT,
                                        "unterminated HTML comment",
                                        source,
                                        index,
                                    )
                                    .help("close the comment with `-->`")
                                    .into());
                            };
                            index += close;
                            continue;
                        }

                        if remaining.starts_with("<!") {
                            let Some(close) = html::find_declaration_close(remaining) else {
                                return Err(self
                                    .html_error(
                                        codes::UNTERMINATED_HTML_DECLARATION,
                                        "unterminated HTML declaration",
                                        source,
                                        index,
                                    )
                                    .help("close the declaration with `>`")
                                    .into());
                            };
                            self.add_raw_fragment(&remaining[..close]);
                            index += close;
                            continue;
                        }

                        if remaining.starts_with('<') {
                            let Some(tag) = html::parse_tag(remaining) else {
                                return Err(self
                                    .html_error(
                                        codes::MALFORMED_HTML_TAG,
                                        "malformed HTML tag",
                                        source,
                                        index,
                                    )
                                    .help(
                                        "close the tag with `>`, or escape a literal `<` as `&lt;`",
                                    )
                                    .into());
                            };
                            if tag.closing {
                                return Err(self
                                    .html_error(
                                        codes::UNEXPECTED_CLOSING_TAG,
                                        format!("unexpected closing tag </{}>", tag.name),
                                        source,
                                        index,
                                    )
                                    .help("remove it, or add the matching opening tag before it")
                                    .into());
                            }

                            let content_start = index + tag.close + 1;
                            let (content_end, close_end) = if tag.self_closing
                                || html::is_void_element(tag.name)
                            {
                                (content_start, content_start)
                            } else if let Some((close_start, close_end)) =
                                html::find_matching_end(remaining, tag.name, tag.close + 1)
                            {
                                (index + close_start, index + close_end)
                            } else {
                                return Err(self
                                    .html_error(
                                        codes::UNCLOSED_HTML_TAG,
                                        format!("unclosed <{}> tag", tag.name),
                                        source,
                                        index,
                                    )
                                    .help(format!(
                                        "add the matching </{}> closing tag, or make the element self-closing",
                                        tag.name
                                    ))
                                    .into());
                            };

                            let element = Element {
                                source,
                                start: index,
                                tag,
                                content_start,
                                content_end,
                                close_end,
                            };

                            if close_end < end {
                                ops.push(ParseOp::Parse {
                                    range: close_end..end,
                                    depth,
                                });
                            }

                            match element.name() {
                                "for" => {
                                    self.enter_for_directive(&element, fragments, depth, &mut ops)?;
                                }
                                "if" => {
                                    self.enter_if_directive(&element, fragments, depth, &mut ops)?;
                                }
                                "route" => {
                                    self.process_route_directive(&element, fragments)?;
                                }
                                "outlet" => {
                                    self.flush_raw_buffer(fragments);
                                    fragments.push(WebUIFragment::outlet());
                                }
                                "boundary" => {
                                    self.enter_boundary_directive(
                                        &element, fragments, depth, &mut ops,
                                    )?;
                                }
                                name if name.eq_ignore_ascii_case("body") => {
                                    self.enter_body_element(&element, fragments, depth, &mut ops)?;
                                }
                                name if name.eq_ignore_ascii_case("head") => {
                                    self.enter_head_element(&element, fragments, depth, &mut ops)?;
                                }
                                name if name.eq_ignore_ascii_case("webui-hydrate") => {
                                    return Err(self.authored_webui_hydrate_error(&element));
                                }
                                name if name.eq_ignore_ascii_case("style") => {
                                    self.process_style_element(&element, fragments)?;
                                }
                                _ if self.component_registry.contains(element.name()) => {
                                    self.enter_component_directive(
                                        &element, fragments, depth, &mut ops,
                                    )?;
                                }
                                _ => {
                                    self.check_component_typo(&element)?;
                                    self.enter_regular_element(
                                        &element, fragments, depth, &mut ops,
                                    )?;
                                }
                            }

                            break;
                        }

                        let next = remaining.find('<').unwrap_or(remaining.len());
                        let text_end = if next == 0 {
                            remaining
                                .chars()
                                .next()
                                .map_or(remaining.len(), char::len_utf8)
                        } else {
                            next
                        };
                        self.process_text(&remaining[..text_end], fragments)?;
                        index += text_end;
                    }
                }
                ParseOp::EmitClose(name) => {
                    self.add_raw_fragment("</");
                    self.add_raw_fragment(name);
                    self.add_raw_fragment(">");
                }
                ParseOp::EndHead(name) => {
                    self.flush_raw_buffer(fragments);
                    fragments.push(structural_signal("head_end"));
                    self.add_raw_fragment("</");
                    self.add_raw_fragment(name);
                    self.add_raw_fragment(">");
                }
                ParseOp::EndBody(name) => {
                    self.flush_raw_buffer(fragments);
                    fragments.push(structural_signal("body_end"));
                    self.add_raw_fragment("</");
                    self.add_raw_fragment(name);
                    self.add_raw_fragment(">");
                    self.body_depth = self.body_depth.saturating_sub(1);
                }
                ParseOp::EndBoundary(sequence) => {
                    self.flush_raw_buffer(fragments);
                    fragments.push(structural_signal(format!("boundary_end:{sequence}")));
                    self.in_boundary = false;
                }
                ParseOp::RestoreFosterDepth(previous) => {
                    self.foster_context_depth = previous;
                }
                ParseOp::RestoreBoundaryParentScope(previous) => {
                    self.boundary_parent_scope = previous;
                }
                ParseOp::CompleteFor {
                    parent,
                    item,
                    collection,
                    fragment_id,
                    keep_empty,
                } => {
                    self.structural_scopes.pop();
                    self.flush_raw_buffer(fragments);
                    let for_fragment = std::mem::take(fragments);
                    *fragments = parent.fragments;
                    self.raw_buffer = parent.raw_buffer;

                    if !for_fragment.is_empty() {
                        self.fragment_records.insert(
                            fragment_id.clone(),
                            FragmentList {
                                fragments: for_fragment,
                            },
                        );
                    } else if !keep_empty {
                        continue;
                    }

                    self.add_for_fragment(item, collection, fragment_id, fragments);
                }
                ParseOp::CompleteIf {
                    parent,
                    condition,
                    fragment_id,
                } => {
                    self.structural_scopes.pop();
                    self.flush_raw_buffer(fragments);
                    let if_fragment = std::mem::take(fragments);
                    *fragments = parent.fragments;
                    self.raw_buffer = parent.raw_buffer;

                    self.fragment_records.insert(
                        fragment_id.clone(),
                        FragmentList {
                            fragments: if_fragment,
                        },
                    );
                    self.add_if_fragment(condition, fragment_id, fragments);
                }
            }
        }

        Ok(())
    }

    fn enter_regular_element<'a>(
        &mut self,
        element: &Element<'a>,
        fragments: &mut Vec<WebUIFragment>,
        depth: usize,
        ops: &mut Vec<ParseOp<'a>>,
    ) -> Result<()> {
        if !self.in_boundary && element.name().eq_ignore_ascii_case("script") {
            self.record_module_entry(element);
        }
        self.add_raw_fragment("<");
        self.add_raw_fragment(element.name());

        let binding_count = self.process_tag_attributes(element.attrs(), fragments, false)?;
        if let Some(ref mut p) = self.plugin {
            if let Some(data) = p.finish_element(binding_count) {
                self.add_fragment(WebUIFragment::plugin(data), fragments);
            }
        }

        if element.self_closing() {
            self.add_raw_fragment("/>");
            return Ok(());
        }

        self.add_raw_fragment(">");
        if !element.is_void() {
            if element.close_end() > element.content_end() {
                ops.push(ParseOp::EmitClose(element.name()));
            }
            self.enter_foster_context(element.name(), ops);
            self.enter_boundary_parent_scope(element.name(), ops);
            ops.push(ParseOp::Parse {
                range: element.inner(),
                depth: depth + 1,
            });
        }
        Ok(())
    }

    /// Record an authored `<script type="module" src="...">` as a critical
    /// module entry, so the build can preload the chunks it statically imports.
    ///
    /// Only bare, static `src` values qualify. A `{{binding}}` resolves per
    /// request and cannot be matched to a build artifact, and a script without
    /// `type="module"` has no ES module graph to preload. Both are skipped
    /// silently rather than diagnosed, because either is a perfectly valid
    /// thing to author — they just carry no preload information.
    ///
    /// The caller has already excluded scripts inside a `<boundary>`.
    fn record_module_entry(&mut self, element: &Element<'_>) {
        let Some(src) = element.attr("src") else {
            return;
        };
        if src.is_empty() || src.contains("{{") {
            return;
        }
        if !element
            .attr("type")
            .is_some_and(|kind| kind.trim().eq_ignore_ascii_case("module"))
        {
            return;
        }
        // Document order is meaningful and duplicates are near-impossible, so
        // this stays a `Vec` scan rather than paying for a set.
        if self.module_entry_srcs.iter().any(|seen| seen == src) {
            return;
        }
        self.module_entry_srcs.push(src.to_string());
    }

    fn enter_head_element<'a>(
        &mut self,
        element: &Element<'a>,
        fragments: &mut Vec<WebUIFragment>,
        depth: usize,
        ops: &mut Vec<ParseOp<'a>>,
    ) -> Result<()> {
        if element.name() == "head" {
            // Preserve the lowercase legacy protocol shape exactly: authored
            // attributes on `<head>` were ignored and the opening was emitted
            // as the literal `<head>`.
            self.add_raw_fragment("<head>");
        } else {
            // origin/main treated mixed-case native tags as ordinary elements.
            // Keep their authored name and attributes while adding only the
            // separate case-insensitive streaming structure.
            self.add_raw_fragment("<");
            self.add_raw_fragment(element.name());
            let binding_count = self.process_tag_attributes(element.attrs(), fragments, false)?;
            if let Some(ref mut p) = self.plugin {
                if let Some(data) = p.finish_element(binding_count) {
                    self.add_fragment(WebUIFragment::plugin(data), fragments);
                }
            }
            self.add_raw_fragment(">");
        }
        self.flush_raw_buffer(fragments);
        // Mirrors `body_start`: emitted immediately after the raw opening
        // `<head>` fragment and before any child content (including
        // authored async scripts), so `render_streaming` can preflight on it.
        // Raw signals unknown to the legacy (non-streaming) handler are
        // ignored there, so this is a no-op for existing output.
        fragments.push(structural_signal("head_start"));
        ops.push(ParseOp::EndHead(element.name()));
        ops.push(ParseOp::Parse {
            range: element.inner(),
            depth: depth + 1,
        });
        Ok(())
    }

    fn enter_body_element<'a>(
        &mut self,
        element: &Element<'a>,
        fragments: &mut Vec<WebUIFragment>,
        depth: usize,
        ops: &mut Vec<ParseOp<'a>>,
    ) -> Result<()> {
        self.add_raw_fragment("<");
        self.add_raw_fragment(element.name());
        let binding_count = self.process_tag_attributes(element.attrs(), fragments, false)?;
        if let Some(ref mut p) = self.plugin {
            if let Some(data) = p.finish_element(binding_count) {
                self.add_fragment(WebUIFragment::plugin(data), fragments);
            }
        }
        self.add_raw_fragment(">");
        self.flush_raw_buffer(fragments);
        fragments.push(structural_signal("body_start"));
        self.body_depth += 1;
        ops.push(ParseOp::EndBody(element.name()));
        ops.push(ParseOp::Parse {
            range: element.inner(),
            depth: depth + 1,
        });
        Ok(())
    }

    /// Start an authoring [`Diagnostic`] for the template currently being
    /// parsed, naming the owning fragment (entry file or component tag) when
    /// known. `code` is the stable machine-readable [`code`](crate::diagnostic::codes).
    ///
    /// Marked `#[cold]`/`#[inline(never)]`: this only runs while *building* a
    /// build error, so keeping it out-of-line preserves hot parse-path layout.
    #[cold]
    #[inline(never)]
    fn authoring_error(&self, code: &'static str, title: impl Into<String>) -> Diagnostic {
        let diag = Diagnostic::error(title).code(code);
        if self.current_fragment_id.is_empty() {
            diag
        } else {
            diag.component(self.current_fragment_id.clone())
        }
    }

    /// Like [`HtmlParser::authoring_error`], but also records the source
    /// position (line/column) of `element` so the diagnostic can point at the
    /// exact spot in the template.
    #[cold]
    #[inline(never)]
    fn authoring_error_at(
        &self,
        code: &'static str,
        title: impl Into<String>,
        element: &Element<'_>,
    ) -> Diagnostic {
        self.authoring_error(code, title)
            .at_offset(element.source(), element.start)
    }

    /// Build a positioned [`Diagnostic`] for a structural HTML well-formedness
    /// error (unclosed/malformed tag, unterminated comment, …), naming the
    /// owning fragment and the source position from a byte `offset`.
    #[cold]
    #[inline(never)]
    fn html_error(
        &self,
        code: &'static str,
        title: impl Into<String>,
        source: &str,
        offset: usize,
    ) -> Diagnostic {
        self.authoring_error(code, title).at_offset(source, offset)
    }

    /// Build the `help:` line for an unknown component `<name>`.
    ///
    /// Offers the closest registered component as a "did you mean …?" fix when
    /// one is a near typo; otherwise falls back to the generic registration
    /// hint.
    #[cold]
    #[inline(never)]
    fn unknown_component_help(&self, name: &str) -> String {
        match suggest::closest_match(name, self.component_registry.names()) {
            Some(suggestion) => format!(
                "did you mean <{suggestion}>? otherwise register the component by adding a \
                 matching .html file"
            ),
            None => "register the component (add a matching .html file) or check the tag name \
                     for a typo"
                .to_string(),
        }
    }

    /// Suggest a registered component that an unregistered custom-element `tag`
    /// likely mistypes.
    ///
    /// Only same-namespace candidates are considered — the text before the
    /// first `-` must match exactly — so a genuine third-party custom element
    /// (`<md-button>` when `<mp-button>` is registered) is never flagged, while
    /// an in-namespace slip (`<mp-buton>` → `<mp-button>`) is. Returns `None`
    /// for non-hyphenated (native) tags and when no near match exists.
    fn suggest_component(&self, tag: &str) -> Option<&str> {
        let (prefix, _) = tag.split_once('-')?;
        let same_namespace = self
            .component_registry
            .names()
            .filter(|name| name.split_once('-').map(|(p, _)| p) == Some(prefix));
        suggest::closest_match(tag, same_namespace)
    }

    /// Error with a "did you mean …?" hint when `element` is an unregistered
    /// custom-element tag that closely matches a registered component in the
    /// same namespace. Genuine external custom elements pass through as raw
    /// HTML (returns `Ok`).
    fn check_component_typo(&self, element: &Element<'_>) -> Result<()> {
        if let Some(suggestion) = self.suggest_component(element.name()) {
            return Err(self
                .authoring_error_at(
                    codes::UNKNOWN_COMPONENT,
                    format!("unknown component <{}>", element.name()),
                    element,
                )
                .help(format!(
                    "did you mean <{suggestion}>? otherwise register <{}> by adding a matching \
                     .html file",
                    element.name()
                ))
                .into());
        }
        Ok(())
    }

    /// Name of an attribute on `element` that looks like a typo of `expected`
    /// (close edit distance, not an exact match), if any. Used to suggest the
    /// intended directive attribute (e.g. `eahc` → `each`).
    #[cold]
    #[inline(never)]
    fn attr_typo_suggestion<'a>(element: &Element<'a>, expected: &str) -> Option<&'a str> {
        suggest::closest_match(expected, element.attrs().map(|attr| attr.name))
            .filter(|&name| name != expected)
    }

    /// Promote a standalone [`ParserError::Css`] into a structured authoring
    /// [`Diagnostic`] (code [`codes::INVALID_CSS`], owning fragment), so CSS
    /// mistakes render and serialize like every other authoring error. The
    /// message already carries the in-`<style>` line/column. Non-CSS errors
    /// pass through unchanged.
    #[cold]
    #[inline(never)]
    fn css_diagnostic(&self, err: ParserError) -> ParserError {
        match err {
            ParserError::Css(message) => self.authoring_error(codes::INVALID_CSS, message).into(),
            other => other,
        }
    }

    /// Build the error for a `<for>` missing its `each` attribute (cold path).
    #[cold]
    #[inline(never)]
    fn for_each_missing_error(&self, element: &Element<'_>) -> ParserError {
        let diag = self
            .authoring_error_at(
                codes::MISSING_FOR_EACH,
                "missing each attribute on <for>",
                element,
            )
            .element("for");
        match Self::attr_typo_suggestion(element, "each") {
            Some(typo) => diag.help(format!(
                "found `{typo}` \u{2014} did you mean each=\"item in collection\"?"
            )),
            None => diag.help("add each=\"item in collection\", e.g. <for each=\"todo in todos\">"),
        }
        .into()
    }

    /// Build the error for a malformed `<for each>` expression (cold path).
    #[cold]
    #[inline(never)]
    fn for_each_invalid_error(&self, element: &Element<'_>, each: &str) -> ParserError {
        self.authoring_error_at(
            codes::INVALID_FOR_EACH,
            "invalid <for> each expression",
            element,
        )
        .element("for")
        .snippet(format!("each=\"{each}\""))
        .help("use the form each=\"item in collection\", e.g. each=\"todo in todos\"")
        .into()
    }

    /// Build the error for a `<for each>` with disallowed identifier characters
    /// (cold path).
    #[cold]
    #[inline(never)]
    fn for_identifier_error(&self, element: &Element<'_>, each: &str) -> ParserError {
        self.authoring_error_at(
            codes::INVALID_FOR_IDENTIFIER,
            "invalid identifier in <for> each expression",
            element,
        )
        .element("for")
        .snippet(format!("each=\"{each}\""))
        .help("item and collection names may use only letters, digits, '_', '-', and '.'")
        .into()
    }

    fn enter_for_directive<'a>(
        &mut self,
        element: &Element<'a>,
        fragments: &mut Vec<WebUIFragment>,
        depth: usize,
        ops: &mut Vec<ParseOp<'a>>,
    ) -> Result<()> {
        let each = element
            .attr("each")
            .map(ToString::to_string)
            .ok_or_else(|| self.for_each_missing_error(element))?;

        let mut parts = each.split_whitespace();
        let (Some(item), Some(in_kw), Some(collection), None) =
            (parts.next(), parts.next(), parts.next(), parts.next())
        else {
            return Err(self.for_each_invalid_error(element, &each));
        };
        if in_kw != "in" {
            return Err(self.for_each_invalid_error(element, &each));
        }

        let allowed = |s: &str| {
            s.chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-' || c == '.')
        };
        if !allowed(item) || !allowed(collection) {
            return Err(self.for_identifier_error(element, &each));
        }

        let custom_fragment_id = element.attr("template").map(ToString::to_string);
        let keep_empty = custom_fragment_id.is_some();
        let fragment_id = custom_fragment_id.unwrap_or_else(|| self.id_counter.next_id("for"));
        let parent = ParseContext {
            fragments: std::mem::take(fragments),
            raw_buffer: std::mem::take(&mut self.raw_buffer),
        };

        self.structural_scopes.push("a <for> loop");
        ops.push(ParseOp::CompleteFor {
            parent,
            item: item.to_string(),
            collection: collection.to_string(),
            fragment_id,
            keep_empty,
        });
        ops.push(ParseOp::Parse {
            range: element.inner(),
            depth: depth + 1,
        });
        Ok(())
    }

    /// Build the error for an `<if>` missing its `condition` attribute (cold
    /// path).
    #[cold]
    #[inline(never)]
    fn if_condition_missing_error(&self, element: &Element<'_>) -> ParserError {
        let diag = self
            .authoring_error_at(
                codes::MISSING_IF_CONDITION,
                "missing condition attribute on <if>",
                element,
            )
            .element("if");
        match Self::attr_typo_suggestion(element, "condition") {
            Some(typo) => diag.help(format!(
                "found `{typo}` \u{2014} did you mean condition=\"expression\"?"
            )),
            None => diag.help("add condition=\"expression\", e.g. <if condition=\"isActive\">"),
        }
        .into()
    }

    /// Build the error for a malformed `<if condition>` expression (cold path).
    #[cold]
    #[inline(never)]
    fn if_condition_invalid_error(&self, element: &Element<'_>, condition: &str) -> ParserError {
        self.authoring_error_at(
            codes::INVALID_IF_CONDITION,
            "invalid <if> condition expression",
            element,
        )
        .element("if")
        .snippet(format!("condition=\"{condition}\""))
        .help("use a simple expression like \"isActive\", \"count > 0\", or \"!hidden\"")
        .into()
    }

    fn enter_if_directive<'a>(
        &mut self,
        element: &Element<'a>,
        fragments: &mut Vec<WebUIFragment>,
        depth: usize,
        ops: &mut Vec<ParseOp<'a>>,
    ) -> Result<()> {
        let condition_str = element
            .attr("condition")
            .map(ToString::to_string)
            .ok_or_else(|| self.if_condition_missing_error(element))?;

        let condition = self
            .condition_parser
            .parse(&condition_str)
            .map_err(|_| self.if_condition_invalid_error(element, &condition_str))?;

        self.flush_raw_buffer(fragments);
        let parent = ParseContext {
            fragments: std::mem::take(fragments),
            raw_buffer: std::mem::take(&mut self.raw_buffer),
        };
        let fragment_id = self.id_counter.next_id("if");

        self.structural_scopes.push("an <if> block");
        ops.push(ParseOp::CompleteIf {
            parent,
            condition,
            fragment_id,
        });
        ops.push(ParseOp::Parse {
            range: element.inner(),
            depth: depth + 1,
        });
        Ok(())
    }

    /// Track whether `name` opens (or shields from) an HTML insertion mode
    /// that foster-parents unknown elements, pushing a restore op when the
    /// depth changes.
    ///
    /// Called for every regular element, so the common `Transparent` case
    /// does no work beyond the classification compare — no op is pushed and
    /// no field is written.
    #[inline]
    fn enter_foster_context<'a>(&mut self, name: &str, ops: &mut Vec<ParseOp<'a>>) {
        match foster_context_of(name) {
            FosterContext::Transparent => {}
            FosterContext::Hostile => {
                ops.push(ParseOp::RestoreFosterDepth(self.foster_context_depth));
                self.foster_context_depth += 1;
            }
            FosterContext::Barrier => {
                if self.foster_context_depth != 0 {
                    ops.push(ParseOp::RestoreFosterDepth(self.foster_context_depth));
                    self.foster_context_depth = 0;
                }
            }
        }
    }

    #[inline]
    fn enter_boundary_parent_scope<'a>(&mut self, name: &str, ops: &mut Vec<ParseOp<'a>>) {
        let Some(scope) = boundary_parent_scope(name) else {
            return;
        };
        ops.push(ParseOp::RestoreBoundaryParentScope(
            self.boundary_parent_scope,
        ));
        self.boundary_parent_scope = Some(scope);
    }

    #[inline]
    fn enter_component_content_scope<'a>(&mut self, ops: &mut Vec<ParseOp<'a>>) {
        ops.push(ParseOp::RestoreBoundaryParentScope(
            self.boundary_parent_scope,
        ));
        self.boundary_parent_scope = Some("component host content");
    }

    /// Enter a `<boundary name="…">` directive.
    ///
    /// `<boundary>` is a reserved, compile-time-only directive (see
    /// "Progressive Streaming Hydration" in `DESIGN.md`): it emits
    /// no wrapper element, only `boundary_start:<seq>` /
    /// `boundary_end:<seq>` raw signal fragments around its children, mirroring
    /// how [`Self::enter_body_element`] brackets its children with
    /// `body_start`/`body_end`.
    ///
    /// # Errors
    ///
    /// Returns [`ParserError::Template`] when:
    /// - the boundary is nested inside another open boundary
    ///   ([`codes::NESTED_BOUNDARY`]);
    /// - the boundary is outside an open `<body>`
    ///   ([`codes::BOUNDARY_OUTSIDE_BODY`]);
    /// - the boundary sits in an HTML foster-parenting context such as
    ///   `<table>`/`<tbody>`/`<tr>`/`<select>`
    ///   ([`codes::BOUNDARY_IN_FOSTER_CONTEXT`]);
    /// - the boundary would cut through a reusable component template, an
    ///   `<if>`, a `<for>`, or a `<route>` scope instead of independently
    ///   wrapping it ([`codes::BOUNDARY_CROSSES_SCOPE`] — see the "Known
    ///   limitation" note on [`Self::boundary_scope_error`]);
    /// - `name` is missing/empty ([`codes::MISSING_BOUNDARY_NAME`]), dynamic
    ///   ([`codes::INVALID_BOUNDARY_NAME`]), or a duplicate within the entry
    ///   ([`codes::DUPLICATE_BOUNDARY_NAME`]).
    fn enter_boundary_directive<'a>(
        &mut self,
        element: &Element<'a>,
        fragments: &mut Vec<WebUIFragment>,
        depth: usize,
        ops: &mut Vec<ParseOp<'a>>,
    ) -> Result<()> {
        // `in_progress_fragments.len() > 1` means parsing has recursed into a
        // reusable template (a component usage or a route-referenced
        // component) via `self.parse(..)`, as opposed to the outermost entry
        // template. The design intentionally permits boundaries only in that
        // outermost entry.
        if self.in_progress_fragments.len() > 1 {
            return Err(self.boundary_scope_error(element, "a reusable component template"));
        }
        if let Some(scope) = self.structural_scopes.last() {
            return Err(self.boundary_scope_error(element, scope));
        }
        if let Some(scope) = self.boundary_parent_scope {
            return Err(self.boundary_scope_error(element, scope));
        }
        if self.in_boundary {
            return Err(self.nested_boundary_error(element));
        }
        if self.body_depth == 0 {
            return Err(self.boundary_outside_body_error(element));
        }
        if self.foster_context_depth != 0 {
            return Err(self.boundary_in_foster_context_error(element));
        }

        let name = self.validate_boundary_name(element)?;
        if !self.boundary_names.insert(name.clone()) {
            return Err(self.duplicate_boundary_name_error(element, &name));
        }
        self.boundary_names_in_order.push(name);

        let sequence = self.boundary_sequence;
        self.boundary_sequence += 1;
        self.in_boundary = true;

        self.flush_raw_buffer(fragments);
        fragments.push(structural_signal(format!("boundary_start:{sequence}")));

        ops.push(ParseOp::EndBoundary(sequence));
        ops.push(ParseOp::Parse {
            range: element.inner(),
            depth: depth + 1,
        });
        Ok(())
    }

    /// Validate `<boundary name>`: required, non-empty, and static (no
    /// `{{binding}}`).
    fn validate_boundary_name(&self, element: &Element<'_>) -> Result<String> {
        let name = element.attr("name").unwrap_or_default();
        if name.is_empty() {
            return Err(self.missing_boundary_name_error(element));
        }
        if name.contains("{{") {
            return Err(self.invalid_boundary_name_error(element, name));
        }
        Ok(name.to_string())
    }

    /// Build the error for a `<boundary>` missing its `name` attribute
    /// (cold path).
    #[cold]
    #[inline(never)]
    fn missing_boundary_name_error(&self, element: &Element<'_>) -> ParserError {
        self.authoring_error_at(
            codes::MISSING_BOUNDARY_NAME,
            "missing name attribute on <boundary>",
            element,
        )
        .element("boundary")
        .help("add a unique static name, e.g. <boundary name=\"counter-ready\">")
        .into()
    }

    /// Build the error for a `<boundary name>` that is not a static
    /// string literal (cold path).
    #[cold]
    #[inline(never)]
    fn invalid_boundary_name_error(&self, element: &Element<'_>, name: &str) -> ParserError {
        self.authoring_error_at(
            codes::INVALID_BOUNDARY_NAME,
            "invalid name attribute on <boundary>",
            element,
        )
        .element("boundary")
        .snippet(format!("name=\"{name}\""))
        .help("boundary names must be a static string literal, e.g. name=\"counter-ready\"")
        .into()
    }

    /// Build the error for a duplicate `<boundary name>` within the
    /// same entry template (cold path).
    #[cold]
    #[inline(never)]
    fn duplicate_boundary_name_error(&self, element: &Element<'_>, name: &str) -> ParserError {
        self.authoring_error_at(
            codes::DUPLICATE_BOUNDARY_NAME,
            format!("duplicate <boundary> name \"{name}\""),
            element,
        )
        .element("boundary")
        .help("boundary names must be unique within the page; rename one of the boundaries")
        .into()
    }

    /// Build the error for a `<boundary>` nested inside another open
    /// boundary (cold path).
    #[cold]
    #[inline(never)]
    fn nested_boundary_error(&self, element: &Element<'_>) -> ParserError {
        self.authoring_error_at(
            codes::NESTED_BOUNDARY,
            "nested <boundary> is not allowed",
            element,
        )
        .element("boundary")
        .help("boundaries cannot be nested; close the enclosing <boundary> before opening another")
        .into()
    }

    /// Build the error for a `<boundary>` that would cut through
    /// `scope` (a reusable component template, an `<if>`/`<for>` block, or a
    /// `<route>` element) instead of independently wrapping it (cold path).
    ///
    /// The parser rejects boundaries in every recursively parsed template,
    /// including route shells: only the outermost entry passed to
    /// [`Self::parse`] may contain one.
    #[cold]
    #[inline(never)]
    fn boundary_scope_error(&self, element: &Element<'_>, scope: &str) -> ParserError {
        self.authoring_error_at(
            codes::BOUNDARY_CROSSES_SCOPE,
            format!("<boundary> cannot appear inside {scope}"),
            element,
        )
        .element("boundary")
        .help("move the <boundary> so it fully wraps this scope from the entry template, instead of appearing inside it")
        .into()
    }

    /// Build the error for a `<boundary>` outside an open body.
    #[cold]
    #[inline(never)]
    fn boundary_outside_body_error(&self, element: &Element<'_>) -> ParserError {
        self.authoring_error_at(
            codes::BOUNDARY_OUTSIDE_BODY,
            "<boundary> must appear inside <body>",
            element,
        )
        .element("boundary")
        .help("move the <boundary> between the opening <body> and its matching </body>")
        .into()
    }

    /// Build the error for a `<boundary>` inside an HTML
    /// foster-parenting insertion mode (cold path).
    ///
    /// The browser would move the generated `<webui-hydrate>` sentinel out of
    /// the table while leaving its payload `<script>` behind, so the
    /// coordinator could not pair them and would halt hydration for the whole
    /// page. Rejecting at build time keeps that failure impossible.
    #[cold]
    #[inline(never)]
    fn boundary_in_foster_context_error(&self, element: &Element<'_>) -> ParserError {
        self.authoring_error_at(
            codes::BOUNDARY_IN_FOSTER_CONTEXT,
            "<boundary> cannot appear inside a table or select context",
            element,
        )
        .element("boundary")
        .help(
            "the HTML parser moves unknown elements out of <table>/<tbody>/<tr>/<select>, which would split the hydration sentinel from its payload; wrap the whole <table> in the boundary, or move it inside a <td>, <th>, or <caption>",
        )
        .into()
    }

    /// Build the error for an authored `<webui-hydrate>` (cold path).
    /// `<webui-hydrate>` is reserved for the compiler/handler-generated
    /// runtime hydration sentinel and must never appear in authored markup.
    #[cold]
    #[inline(never)]
    fn authored_webui_hydrate_error(&self, element: &Element<'_>) -> ParserError {
        self.authoring_error_at(
            codes::AUTHORED_WEBUI_HYDRATE,
            "<webui-hydrate> is reserved for the generated runtime sentinel",
            element,
        )
        .element("webui-hydrate")
        .help("remove <webui-hydrate>; it is emitted only by the compiler/handler, never authored")
        .into()
    }

    fn enter_component_directive<'a>(
        &mut self,
        element: &Element<'a>,
        fragments: &mut Vec<WebUIFragment>,
        depth: usize,
        ops: &mut Vec<ParseOp<'a>>,
    ) -> Result<()> {
        self.add_raw_fragment("<");
        self.add_raw_fragment(element.name());

        let binding_count = self.process_tag_attributes(element.attrs(), fragments, true)?;
        if let Some(ref mut p) = self.plugin {
            if let Some(data) = p.finish_element(binding_count) {
                self.add_fragment(WebUIFragment::plugin(data), fragments);
            }
        }

        // Emit a compiler-owned streamed SSR root signal immediately before the
        // component opening tag closes. Ordinary rendering ignores it byte-for-byte;
        // streaming rendering consumes it to inject ` data-ws` inside the opening tag
        // (so the marker exists before custom-element upgrade). Placing the signal
        // between the attribute run and the closing `>` is required because an
        // attribute cannot be injected once `>` has already been flushed.
        self.flush_raw_buffer(fragments);
        fragments.push(structural_signal(format!(
            "streaming_root:{}",
            element.name()
        )));

        if element.self_closing() {
            self.add_raw_fragment("/>");
        } else {
            self.add_raw_fragment(">");
        }

        self.flush_raw_buffer(fragments);

        if !self.fragment_records.contains_key(element.name()) {
            let component_data = self
                .component_registry
                .get(element.name())
                .ok_or_else(|| {
                    self.authoring_error_at(
                        codes::UNKNOWN_COMPONENT,
                        format!("unknown component <{}>", element.name()),
                        element,
                    )
                    .help(self.unknown_component_help(element.name()))
                })?
                .clone();
            let artifact_html = self
                .component_registry
                .component_artifact_source(element.name())
                .map(str::to_string);
            let built = self.build_component_templates(ComponentTemplateInput {
                tag_name: element.name(),
                html: &component_data.html_content,
                artifact_html: artifact_html.as_deref(),
                css_content: component_data.css_content.as_deref(),
                artifact_needed: self.plugin.is_some(),
            })?;

            if let Some(ref mut p) = self.plugin {
                p.register_component_template(element.name(), &component_data, built.artifact())?;
            }

            self.parse(element.name(), &built.ssr)?;
        }

        fragments.push(WebUIFragment::component(element.name().to_string()));

        if !element.self_closing() {
            ops.push(ParseOp::EmitClose(element.name()));
            self.enter_component_content_scope(ops);
            ops.push(ParseOp::Parse {
                range: element.inner(),
                depth: depth + 1,
            });
        }

        Ok(())
    }

    fn process_text(&mut self, content: &str, fragments: &mut Vec<WebUIFragment>) -> Result<()> {
        if !Self::should_emit_text_content(content) {
            return Ok(());
        }

        let parsed_fragments = self.handlebars_parser.parse(content)?;
        for fragment in parsed_fragments {
            if let Some(Fragment::Raw(raw)) = fragment.fragment.as_ref() {
                if Self::should_emit_text_content(&raw.value) {
                    self.add_raw_fragment(&raw.value);
                }
            } else {
                self.add_fragment(fragment, fragments);
            }
        }
        Ok(())
    }

    fn process_style_content(
        &mut self,
        css: &str,
        comments: &[crate::css_parser::CssComment],
        fragments: &mut Vec<WebUIFragment>,
    ) {
        let mut last_end = 0usize;

        for comment in comments {
            if comment.start_byte < last_end {
                if comment.end_byte > last_end {
                    last_end = comment.end_byte;
                }
                continue;
            }

            self.add_raw_fragment(&css[last_end..comment.start_byte]);
            let comment_text = &css[comment.start_byte..comment.end_byte];
            if let Some(fragment) = self.css_signal_comment_fragment(comment_text) {
                self.add_fragment(fragment, fragments);
            } else if comment.preserve {
                self.add_raw_fragment(comment_text);
            }
            last_end = comment.end_byte;
        }

        self.add_raw_fragment(&css[last_end..]);
    }

    fn css_signal_comment_fragment(&self, comment: &str) -> Option<WebUIFragment> {
        let signal = comment_policy::parse_css_signal_comment(comment)?;
        Some(WebUIFragment::signal(signal.path, signal.raw))
    }

    /// Check if an attribute value is a pure handlebars expression (e.g., "{{name}}" or
    /// "{{name}}" with quotes). Returns the inner signal name (borrowed) if so.
    fn extract_single_handlebars(value: &str) -> Option<&str> {
        let trimmed = value.trim();
        if trimmed.starts_with("{{") && trimmed.ends_with("}}") && !trimmed.starts_with("{{{") {
            let inner = trimmed[2..trimmed.len() - 2].trim();
            // Verify there's no other {{ in the middle (i.e., it's truly a single expression)
            if !inner.contains("{{") && !inner.is_empty() {
                return Some(inner);
            }
        }
        None
    }

    /// Check if an attribute value contains any handlebars expressions.
    fn contains_handlebars(value: &str) -> bool {
        value.contains("{{")
    }

    fn process_tag_attributes(
        &mut self,
        attrs: Attrs<'_>,
        fragments: &mut Vec<WebUIFragment>,
        is_component: bool,
    ) -> Result<u32> {
        let mut first_dynamic_emitted = false;
        let mut binding_count: u32 = 0;
        for attr in attrs {
            let attr_name = attr.name;

            if let Some(ref mut p) = self.plugin {
                match p.classify_attribute(attr_name) {
                    AttributeAction::Keep => {}
                    AttributeAction::Skip => continue,
                    AttributeAction::SkipAndCountBinding => {
                        binding_count += 1;
                        continue;
                    }
                }
            }

            let attr_value = attr.value;

            if let Some(bool_name) = attr_name.strip_prefix('?') {
                if is_component {
                    if let Some(condition) = self.parse_boolean_condition(attr_value) {
                        let frag = Self::maybe_mark_attr_start(
                            WebUIFragment::attribute_boolean(bool_name, condition),
                            &mut first_dynamic_emitted,
                        );
                        self.add_fragment(frag, fragments);
                        binding_count += 1;
                    }
                } else if self.process_boolean_attribute(bool_name, attr_value, fragments)? {
                    binding_count += 1;
                }
            } else if let Some(prop_name) = attr_name.strip_prefix(':') {
                if !is_component {
                    return Err(ParserError::Parse(format!(
                        ":{prop_name} complex binding is only allowed on custom elements. \
                         Use {prop_name}=\"{{{{expr}}}}\" for native HTML elements."
                    )));
                }
                if Self::is_blocked_complex_property(prop_name) {
                    return Err(ParserError::Parse(format!(
                        ":{prop_name} is not allowed as a complex attribute binding \
                         because it enables arbitrary HTML injection. \
                         Use {{{{{{expr}}}}}} (triple-brace) syntax for raw HTML."
                    )));
                }
                if let Some(val) = attr_value {
                    if let Some(signal_name) = Self::extract_single_handlebars(val) {
                        let frag = Self::maybe_mark_attr_start(
                            WebUIFragment::attribute_complex(attr_name, signal_name),
                            &mut first_dynamic_emitted,
                        );
                        self.add_fragment(frag, fragments);
                        binding_count += 1;
                    }
                }
            } else if is_component && Self::is_skipped_attribute(attr_name) {
                if let Some(val) = attr_value {
                    if let Some(signal_name) = Self::extract_single_handlebars(val) {
                        let frag = WebUIFragment {
                            fragment: Some(web_ui_fragment::Fragment::Attribute(
                                WebUIFragmentAttribute {
                                    name: attr_name.to_string(),
                                    value: signal_name.to_string(),
                                    attr_skip: true,
                                    ..Default::default()
                                },
                            )),
                        };
                        self.add_fragment(frag, fragments);
                        binding_count += 1;
                    } else if Self::contains_handlebars(val) {
                        let template_id = self.id_counter.next_id("attr");
                        let parsed = self.handlebars_parser.parse(val)?;
                        self.fragment_records
                            .insert(template_id.clone(), FragmentList { fragments: parsed });
                        let frag = WebUIFragment {
                            fragment: Some(web_ui_fragment::Fragment::Attribute(
                                WebUIFragmentAttribute {
                                    name: attr_name.to_string(),
                                    template: template_id,
                                    attr_skip: true,
                                    ..Default::default()
                                },
                            )),
                        };
                        self.add_fragment(frag, fragments);
                        binding_count += 1;
                    } else {
                        let frag = WebUIFragment {
                            fragment: Some(web_ui_fragment::Fragment::Attribute(
                                WebUIFragmentAttribute {
                                    name: attr_name.to_string(),
                                    value: val.to_string(),
                                    raw_value: true,
                                    attr_skip: true,
                                    ..Default::default()
                                },
                            )),
                        };
                        self.add_fragment(frag, fragments);
                    }
                }
            } else if let Some(val) = attr_value {
                if Self::contains_handlebars(val) {
                    if is_component {
                        if let Some(signal_name) = Self::extract_single_handlebars(val) {
                            let frag = Self::maybe_mark_attr_start(
                                WebUIFragment::attribute(attr_name, signal_name),
                                &mut first_dynamic_emitted,
                            );
                            self.add_fragment(frag, fragments);
                            binding_count += 1;
                        } else {
                            let template_id = self.id_counter.next_id("attr");
                            let parsed = self.handlebars_parser.parse(val)?;
                            self.fragment_records
                                .insert(template_id.clone(), FragmentList { fragments: parsed });
                            let frag = Self::maybe_mark_attr_start(
                                WebUIFragment::attribute_template(attr_name, template_id),
                                &mut first_dynamic_emitted,
                            );
                            self.add_fragment(frag, fragments);
                            binding_count += 1;
                        }
                    } else {
                        self.process_dynamic_attribute(attr_name, val, fragments)?;
                        binding_count += 1;
                    }
                } else if is_component {
                    let frag = Self::maybe_mark_attr_start(
                        WebUIFragment {
                            fragment: Some(web_ui_fragment::Fragment::Attribute(
                                WebUIFragmentAttribute {
                                    name: attr_name.to_string(),
                                    value: val.to_string(),
                                    raw_value: true,
                                    ..Default::default()
                                },
                            )),
                        },
                        &mut first_dynamic_emitted,
                    );
                    self.add_fragment(frag, fragments);
                } else {
                    self.add_raw_fragment(" ");
                    self.add_raw_fragment(attr.raw);
                }
            } else {
                self.add_raw_fragment(" ");
                self.add_raw_fragment(attr_name);
            }
        }
        Ok(binding_count)
    }

    /// Set `attr_start = true` on the first non-skipped attribute fragment for
    /// a component element.
    fn maybe_mark_attr_start(
        mut frag: WebUIFragment,
        first_dynamic_emitted: &mut bool,
    ) -> WebUIFragment {
        if !*first_dynamic_emitted {
            if let Some(web_ui_fragment::Fragment::Attribute(ref mut a)) = frag.fragment {
                a.attr_start = true;
            }
            *first_dynamic_emitted = true;
        }
        frag
    }

    /// Process a boolean attribute (?prefix). Silently drops if value is not a
    /// pure handlebars expression.
    fn parse_boolean_condition(&self, value: Option<&str>) -> Option<ConditionExpr> {
        if let Some(val) = value {
            if let Some(expr_str) = Self::extract_single_handlebars(val) {
                return Some(
                    self.condition_parser
                        .parse(expr_str)
                        .unwrap_or_else(|_| ConditionExpr::identifier(expr_str)),
                );
            }
        }

        None
    }

    fn process_boolean_attribute(
        &mut self,
        name: &str,
        value: Option<&str>,
        fragments: &mut Vec<WebUIFragment>,
    ) -> Result<bool> {
        if let Some(condition) = self.parse_boolean_condition(value) {
            self.add_fragment(WebUIFragment::attribute_boolean(name, condition), fragments);
            return Ok(true);
        }
        // Invalid boolean attribute — silently drop (no output at all)
        Ok(false)
    }

    /// Process a dynamic attribute (regular name with handlebars in value).
    fn process_dynamic_attribute(
        &mut self,
        name: &str,
        value: &str,
        fragments: &mut Vec<WebUIFragment>,
    ) -> Result<()> {
        if let Some(signal_name) = Self::extract_single_handlebars(value) {
            // Pure handlebars — simple attribute fragment
            self.add_fragment(WebUIFragment::attribute(name, signal_name), fragments);
        } else {
            // Mixed static + dynamic — create a template sub-stream
            let template_id = self.id_counter.next_id("attr");
            let parsed = self.handlebars_parser.parse(value)?;

            self.fragment_records
                .insert(template_id.clone(), FragmentList { fragments: parsed });

            self.add_fragment(
                WebUIFragment::attribute_template(name, template_id),
                fragments,
            );
        }
        Ok(())
    }

    fn process_style_element(
        &mut self,
        element: &Element<'_>,
        fragments: &mut Vec<WebUIFragment>,
    ) -> Result<()> {
        self.add_raw_fragment(element.opening());
        let inner = element.inner();
        let style_content = &element.source()[inner.start..inner.end];
        let (_tokens, defs, requirements, comments) = self
            .css_parser
            .extract_tokens_definitions_requirements_and_comments(
                style_content,
                self.options.legal_comments,
            )
            .map_err(|e| self.css_diagnostic(e))?;
        self.record_fragment_css_tokens(defs, requirements);
        self.process_style_content(style_content, &comments, fragments);
        if element.close_end() > element.content_end() {
            // Reconstruct the closing tag from the parsed name so the emitted
            // case mirrors the source (e.g. `<STYLE>` closes with `</STYLE>`),
            // matching how regular elements are closed via `ParseOp::EmitClose`.
            self.add_raw_fragment("</");
            self.add_raw_fragment(element.name());
            self.add_raw_fragment(">");
        }
        Ok(())
    }

    /// Process a `<route>` directive.
    ///
    /// Emits a `Fragment::Route` protocol fragment. The handler renders
    /// `<webui-route>` elements with server-side route matching — matched
    /// routes get `active` + component content, non-matched get `display:none`.
    fn process_route_directive(
        &mut self,
        element: &Element<'_>,
        fragments: &mut Vec<WebUIFragment>,
    ) -> Result<()> {
        let attrs = Self::route_attrs_from_element(element);
        let path = attrs.path.clone();
        let component = attrs.component.clone();
        route_parser::validate_attributes(&attrs)?;

        let route_params: std::collections::HashSet<String> =
            route_parser::extract_params(&path)?.into_iter().collect();
        if !attrs.cache_tags.is_empty() {
            route_parser::validate_tag_placeholders(
                &attrs.cache_tags,
                &route_params,
                "cache-tags",
                &path,
            )?;
        }
        if !attrs.invalidates.is_empty() {
            route_parser::validate_tag_placeholders(
                &attrs.invalidates,
                &route_params,
                "invalidates",
                &path,
            )?;
        }

        if !attrs.pending_component.is_empty() {
            self.ensure_route_component_parsed(&attrs.pending_component)?;
        }
        if !attrs.error_component.is_empty() {
            self.ensure_route_component_parsed(&attrs.error_component)?;
        }
        self.ensure_route_component_parsed(&component)?;

        let mut all_params = std::collections::HashSet::new();
        all_params.extend(route_params);
        self.validate_route_nesting_depth(element.source(), element.inner(), 1)?;
        let children =
            self.parse_child_routes(element.source(), element.inner(), &all_params, 1)?;

        self.flush_raw_buffer(fragments);
        let route_fragment =
            route_parser::build_route_fragment(&attrs, component.clone(), children);
        fragments.push(WebUIFragment::route_from(route_fragment));

        Ok(())
    }

    /// Parse nested `<route>` children into route fragments.
    ///
    /// `depth` is the nesting level of the routes produced here (top-level
    /// `<route>` children are depth 1). Nested routes are validated against
    /// [`MAX_TEMPLATE_DEPTH`] so pathological route trees cannot exhaust the
    /// call stack via `parse_route_as_fragment` recursion.
    fn parse_child_routes(
        &mut self,
        source: &str,
        range: Range<usize>,
        ancestor_params: &std::collections::HashSet<String>,
        depth: usize,
    ) -> Result<Vec<WebUiFragmentRoute>> {
        if depth > MAX_TEMPLATE_DEPTH {
            return Err(self
                .html_error(
                    codes::EXCESSIVE_NESTING,
                    format!("route nesting exceeds the {MAX_TEMPLATE_DEPTH}-level limit"),
                    source,
                    range.start,
                )
                .help("flatten deeply nested <route> trees before building")
                .into());
        }

        let mut children = Vec::new();
        for event in Walker::new_range(source, range.start, range.end) {
            match event {
                Event::Element(element) => {
                    self.validate_closed_element(&element)?;
                    self.reject_boundary_or_hydrate_in_route(&element)?;
                    if element.name() == "route" {
                        children.push(self.parse_route_as_fragment(
                            &element,
                            ancestor_params,
                            depth,
                        )?);
                    } else if element.name().eq_ignore_ascii_case("style") {
                        self.validate_style_element(&element)?;
                    } else if !element.self_closing() && !element.is_void() {
                        self.validate_ignored_route_html(element.source(), element.inner(), depth)?;
                    }
                }
                Event::Text(text) => {
                    if text.contains('<') {
                        return Err(self
                            .authoring_error(
                                codes::MALFORMED_HTML_TAG,
                                "malformed HTML tag in <route>",
                            )
                            .help("close the tag with `>`, or escape a literal `<` as `&lt;`")
                            .into());
                    }
                }
                Event::Comment(comment_range) => {
                    self.validate_comment_range(source, comment_range)?;
                }
                Event::Declaration(declaration_range) => {
                    self.validate_declaration_range(source, declaration_range)?;
                }
                Event::ClosingTag(closing_range) => {
                    return Err(self.unexpected_closing_tag_error(source, closing_range));
                }
            }
        }

        Ok(children)
    }

    fn validate_route_nesting_depth(
        &self,
        source: &str,
        range: Range<usize>,
        depth: usize,
    ) -> Result<()> {
        let mut stack = vec![(range, depth)];
        while let Some((range, depth)) = stack.pop() {
            if depth > MAX_TEMPLATE_DEPTH {
                return Err(self
                    .html_error(
                        codes::EXCESSIVE_NESTING,
                        format!("route nesting exceeds the {MAX_TEMPLATE_DEPTH}-level limit"),
                        source,
                        range.start,
                    )
                    .help("flatten deeply nested <route> trees before building")
                    .into());
            }

            for event in Walker::new_range(source, range.start, range.end) {
                if let Event::Element(element) = event {
                    self.validate_closed_element(&element)?;
                    if element.name() == "route" && !element.self_closing() {
                        stack.push((element.inner(), depth + 1));
                    }
                }
            }
        }
        Ok(())
    }

    fn validate_ignored_route_html(
        &mut self,
        source: &str,
        range: Range<usize>,
        depth: usize,
    ) -> Result<()> {
        let mut stack = vec![(range, depth)];
        while let Some((range, depth)) = stack.pop() {
            if depth > MAX_TEMPLATE_DEPTH {
                return Err(self
                    .html_error(
                        codes::EXCESSIVE_NESTING,
                        format!("template nesting exceeds the {MAX_TEMPLATE_DEPTH}-level limit"),
                        source,
                        range.start,
                    )
                    .help("split deeply nested markup into components, or reduce generated nesting")
                    .into());
            }

            for event in Walker::new_range(source, range.start, range.end) {
                match event {
                    Event::Element(element) => {
                        self.validate_closed_element(&element)?;
                        self.reject_boundary_or_hydrate_in_route(&element)?;
                        if element.name().eq_ignore_ascii_case("style") {
                            self.validate_style_element(&element)?;
                        } else if !element.self_closing() && !element.is_void() {
                            stack.push((element.inner(), depth + 1));
                        }
                    }
                    Event::Text(text) => {
                        if text.contains('<') {
                            return Err(self
                                .authoring_error(
                                    codes::MALFORMED_HTML_TAG,
                                    "malformed HTML tag in <route>",
                                )
                                .help("close the tag with `>`, or escape a literal `<` as `&lt;`")
                                .into());
                        }
                    }
                    Event::Comment(comment_range) => {
                        self.validate_comment_range(source, comment_range)?;
                    }
                    Event::Declaration(declaration_range) => {
                        self.validate_declaration_range(source, declaration_range)?;
                    }
                    Event::ClosingTag(closing_range) => {
                        return Err(self.unexpected_closing_tag_error(source, closing_range));
                    }
                }
            }
        }
        Ok(())
    }

    /// Reject `<boundary>`/`<webui-hydrate>` written inside `<route>`
    /// markup.
    ///
    /// `<route>` children are validated by a separate well-formedness walker
    /// ([`Self::parse_child_routes`]/[`Self::validate_ignored_route_html`]),
    /// not the main fragment dispatch in [`Self::parse_range`], so both
    /// reserved directives need an explicit check here to produce the same
    /// structured diagnostics as everywhere else instead of silently passing
    /// through as ignored HTML.
    fn reject_boundary_or_hydrate_in_route(&self, element: &Element<'_>) -> Result<()> {
        if element.name() == "boundary" {
            Err(self.boundary_scope_error(element, "a <route> element"))
        } else if element.name().eq_ignore_ascii_case("webui-hydrate") {
            Err(self.authored_webui_hydrate_error(element))
        } else {
            Ok(())
        }
    }

    fn validate_closed_element(&self, element: &Element<'_>) -> Result<()> {
        if !element.self_closing()
            && !element.is_void()
            && element.close_end() == element.content_end()
        {
            return Err(self
                .html_error(
                    codes::UNCLOSED_HTML_TAG,
                    format!("unclosed <{}> tag", element.name()),
                    element.source(),
                    element.start,
                )
                .help(format!(
                    "add the matching </{}> closing tag, or make the element self-closing",
                    element.name()
                ))
                .into());
        }
        Ok(())
    }

    fn validate_style_element(&mut self, element: &Element<'_>) -> Result<()> {
        let inner = element.inner();
        let style_content = &element.source()[inner.start..inner.end];
        self.css_parser
            .extract_tokens_definitions_requirements_and_comments(
                style_content,
                self.options.legal_comments,
            )
            .map_err(|e| self.css_diagnostic(e))?;
        Ok(())
    }

    fn validate_comment_range(&self, source: &str, range: Range<usize>) -> Result<()> {
        let start = range.start;
        if source[range].ends_with("-->") {
            return Ok(());
        }
        Err(self
            .html_error(
                codes::UNTERMINATED_HTML_COMMENT,
                "unterminated HTML comment in <route>",
                source,
                start,
            )
            .help("close the comment with `-->`")
            .into())
    }

    fn validate_declaration_range(&self, source: &str, range: Range<usize>) -> Result<()> {
        let start = range.start;
        if source[range].ends_with('>') {
            return Ok(());
        }
        Err(self
            .html_error(
                codes::UNTERMINATED_HTML_DECLARATION,
                "unterminated HTML declaration in <route>",
                source,
                start,
            )
            .help("close the declaration with `>`")
            .into())
    }

    fn unexpected_closing_tag_error(&self, source: &str, range: Range<usize>) -> ParserError {
        let start = range.start;
        let title = match html::parse_tag(&source[range]) {
            Some(tag) => format!("unexpected closing tag </{}> in <route>", tag.name),
            None => "unexpected closing tag in <route>".to_string(),
        };
        self.html_error(codes::UNEXPECTED_CLOSING_TAG, title, source, start)
            .help("remove it, or add the matching opening tag before it")
            .into()
    }

    fn route_attrs_from_element(element: &Element<'_>) -> route_parser::RouteAttributes {
        route_parser::RouteAttributes {
            path: element.attr("path").unwrap_or_default().to_string(),
            component: element.attr("component").unwrap_or_default().to_string(),
            exact: element.has_attr("exact"),
            query: element.attr("query").unwrap_or_default().to_string(),
            keep_alive: element.has_attr("keep-alive"),
            cache_tags: element
                .attr("cache-tags")
                .map(route_parser::parse_tag_list)
                .unwrap_or_default(),
            invalidates: element
                .attr("invalidates")
                .map(route_parser::parse_tag_list)
                .unwrap_or_default(),
            pending_component: element.attr("pending").unwrap_or_default().to_string(),
            error_component: element.attr("error").unwrap_or_default().to_string(),
        }
    }

    fn parse_route_as_fragment(
        &mut self,
        element: &Element<'_>,
        ancestor_params: &std::collections::HashSet<String>,
        depth: usize,
    ) -> Result<WebUiFragmentRoute> {
        let attrs = Self::route_attrs_from_element(element);
        let path = attrs.path.clone();
        let component = attrs.component.clone();

        route_parser::validate_attributes(&attrs)?;

        let own_params: std::collections::HashSet<String> =
            route_parser::extract_params(&path)?.into_iter().collect();
        let mut all_params = ancestor_params.clone();
        all_params.extend(own_params.iter().cloned());
        if !attrs.cache_tags.is_empty() {
            route_parser::validate_tag_placeholders(
                &attrs.cache_tags,
                &all_params,
                "cache-tags",
                &path,
            )?;
        }
        if !attrs.invalidates.is_empty() {
            route_parser::validate_tag_placeholders(
                &attrs.invalidates,
                &all_params,
                "invalidates",
                &path,
            )?;
        }

        if !attrs.pending_component.is_empty() {
            self.ensure_route_component_parsed(&attrs.pending_component)?;
        }
        if !attrs.error_component.is_empty() {
            self.ensure_route_component_parsed(&attrs.error_component)?;
        }

        self.ensure_route_component_parsed(&component)?;
        let children =
            self.parse_child_routes(element.source(), element.inner(), &all_params, depth + 1)?;

        Ok(route_parser::build_route_fragment(
            &attrs, component, children,
        ))
    }

    /// Ensure a route-referenced component is parsed and registered.
    fn ensure_route_component_parsed(&mut self, component: &str) -> Result<()> {
        if component.is_empty()
            || !self.component_registry.contains(component)
            || self.fragment_records.contains_key(component)
        {
            return Ok(());
        }

        let component_data = self
            .component_registry
            .get(component)
            .ok_or_else(|| {
                self.authoring_error(
                    codes::UNKNOWN_COMPONENT,
                    format!("unknown component <{component}>"),
                )
                .help(self.unknown_component_help(component))
            })?
            .clone();
        let artifact_html = self
            .component_registry
            .component_artifact_source(component)
            .map(str::to_string);

        let built = self.build_component_templates(ComponentTemplateInput {
            tag_name: component,
            html: &component_data.html_content,
            artifact_html: artifact_html.as_deref(),
            css_content: component_data.css_content.as_deref(),
            artifact_needed: self.plugin.is_some(),
        })?;

        if let Some(ref mut p) = self.plugin {
            p.register_component_template(component, &component_data, built.artifact())?;
        }

        let saved_buffer = std::mem::take(&mut self.raw_buffer);
        self.parse(component, &built.ssr)?;
        self.raw_buffer = saved_buffer;

        Ok(())
    }

    /// Skipped attribute names for components.
    const SKIPPED_ATTRIBUTES: &[&str] = &["class", "style", "role"];
    /// Skipped attribute prefixes for components.
    const SKIPPED_ATTRIBUTE_PREFIXES: &[&str] = &["data-", "aria-"];
    const ADOPTED_STYLESHEETS_ATTR: &str = "shadowrootadoptedstylesheets";

    fn is_skipped_attribute(name: &str) -> bool {
        Self::SKIPPED_ATTRIBUTES.contains(&name)
            || Self::SKIPPED_ATTRIBUTE_PREFIXES
                .iter()
                .any(|prefix| name.starts_with(prefix))
    }

    /// Properties that must never be set via `:attr` complex bindings.
    /// These enable XSS (HTML injection) or arbitrary code execution.
    fn is_blocked_complex_property(name: &str) -> bool {
        matches!(name, "innerHTML" | "outerHTML" | "srcdoc" | "content") || name.starts_with("on")
    }

    /// Build both SSR-facing and plugin-facing component template views.
    fn build_component_templates(
        &mut self,
        input: ComponentTemplateInput<'_>,
    ) -> Result<BuiltComponentTemplate> {
        let tag_name = input.tag_name;
        let html = input.html;
        let css_content = input.css_content;
        let adopted_specifier = match self.options.css_strategy {
            CssStrategy::Module if css_content.is_some() => Some(tag_name),
            _ => None,
        };
        let css_injection = match self.options.css_strategy {
            CssStrategy::Link => {
                // In light DOM mode, CSS links go in <head> (emitted by handler),
                // not inside each component template.
                if let (Some(css), DomStrategy::Shadow) = (css_content, self.options.dom_strategy) {
                    let href = self.options.css_link_options.resolve(tag_name, css);
                    let mut link = String::with_capacity(31 + href.href.len());
                    link.push_str("<link rel=\"stylesheet\" href=\"");
                    link.push_str(&href.href);
                    link.push_str("\">");
                    Some(link)
                } else {
                    None
                }
            }
            CssStrategy::Style => css_content.map(|css| {
                let trimmed = css.trim();
                let mut style = String::with_capacity(15 + trimmed.len());
                style.push_str("<style>");
                style.push_str(trimmed);
                style.push_str("</style>");
                style
            }),
            CssStrategy::Module => None,
        };

        let artifact_source = if input.artifact_needed {
            input
                .artifact_html
                .or_else(|| Self::template_has_stripped_runtime_attrs(html).then_some(html))
        } else {
            None
        };
        let policy_wrapper = parse_component_render_policy(tag_name, html)?.is_authored();
        let ssr = self.process_component_policy_template(
            html,
            css_injection.as_deref(),
            adopted_specifier,
            policy_wrapper,
        )?;
        let artifact = match artifact_source {
            Some(source) => Some(self.process_component_artifact_template(
                source,
                css_injection.as_deref(),
                adopted_specifier,
                policy_wrapper,
            )?),
            None => None,
        };

        Ok(BuiltComponentTemplate { ssr, artifact })
    }

    /// Process component template HTML for SSR output.
    ///
    /// The developer's authored `<template>` wrapper is the source of truth.
    ///
    /// - **Dev supplied `<template ...>`:** preserved verbatim — including
    ///   `shadowrootmode`, `shadowrootadoptedstylesheets`, signal fragments,
    ///   and any other custom attributes. SSR strips runtime-only attributes
    ///   (`@event`, `:bind`, `?cond`) from the opening tag, since those are
    ///   protocol metadata and never appear in HTML output. Plugin-facing
    ///   artifacts preserve them. If a CSS snippet is supplied, it is injected
    ///   immediately inside the opening tag (before the dev's children) so
    ///   styles still apply. For `CssStrategy::Module`, the parser appends
    ///   `shadowrootadoptedstylesheets="<tag>"` when it is missing.
    ///
    /// - **Dev omitted `<template>`:**
    ///   - `DomStrategy::Shadow` wraps the content in a framework-controlled
    ///     `<template shadowrootmode="open">`, optionally adding
    ///     `shadowrootadoptedstylesheets="<tag>"` for the CSS-module strategy.
    ///   - `DomStrategy::Light` emits the content as-is (with the CSS snippet
    ///     prepended, if any).
    ///
    /// Performance: zero recursion, zero regex. The dev-template path uses
    /// quote-aware scanners for opening-tag queries and pre-sizes the output
    /// buffer to avoid reallocation in the hot path.
    #[cfg(test)]
    fn process_component_template(
        &mut self,
        html: &str,
        css_snippet: Option<&str>,
        adopted_specifier: Option<&str>,
    ) -> Result<String> {
        self.process_component_policy_template(html, css_snippet, adopted_specifier, false)
    }

    fn process_component_policy_template(
        &mut self,
        html: &str,
        css_snippet: Option<&str>,
        adopted_specifier: Option<&str>,
        policy_wrapper: bool,
    ) -> Result<String> {
        self.process_component_template_with_mode(
            html,
            css_snippet,
            adopted_specifier,
            ComponentTemplateMode {
                preserve_runtime_attrs: false,
                policy_wrapper,
            },
        )
    }

    fn process_component_artifact_template(
        &mut self,
        html: &str,
        css_snippet: Option<&str>,
        adopted_specifier: Option<&str>,
        policy_wrapper: bool,
    ) -> Result<String> {
        self.process_component_template_with_mode(
            html,
            css_snippet,
            adopted_specifier,
            ComponentTemplateMode {
                preserve_runtime_attrs: true,
                policy_wrapper,
            },
        )
    }

    fn process_component_template_with_mode(
        &mut self,
        html: &str,
        css_snippet: Option<&str>,
        adopted_specifier: Option<&str>,
        mode: ComponentTemplateMode,
    ) -> Result<String> {
        let trimmed_end = html.trim_end();
        let (trimmed, _) = html::leading_content(trimmed_end);
        let snippet = css_snippet.unwrap_or_default();

        let processed = if trimmed.starts_with("<template") {
            let authored_shadow =
                html::parse_tag(trimmed).is_some_and(|tag| tag.has_attr("shadowrootmode"));
            let base = self.strip_template_build_attrs(trimmed, mode.preserve_runtime_attrs);
            let with_shadow = if mode.policy_wrapper
                && matches!(self.options.dom_strategy, DomStrategy::Shadow)
            {
                Self::append_shadow_mode_if_missing(base)
            } else {
                base
            };
            let with_adopted = Self::append_adopted_attr_if_missing(with_shadow, adopted_specifier);
            let injected = Self::inject_css_snippet_into_template(with_adopted, snippet);
            if mode.policy_wrapper
                && matches!(self.options.dom_strategy, DomStrategy::Light)
                && !authored_shadow
            {
                Self::unwrap_component_policy_template(injected)
            } else {
                injected
            }
        } else {
            match self.options.dom_strategy {
                DomStrategy::Shadow => {
                    let adopted = adopted_specifier.unwrap_or_default();
                    let adopted_extra = if adopted.is_empty() {
                        0
                    } else {
                        Self::adopted_attr_len(adopted)
                    };
                    let mut result =
                        String::with_capacity(45 + adopted_extra + snippet.len() + trimmed.len());
                    result.push_str("<template shadowrootmode=\"open\"");
                    if !adopted.is_empty() {
                        Self::push_adopted_attr(&mut result, adopted);
                    }
                    result.push('>');
                    result.push_str(snippet);
                    result.push_str(trimmed);
                    result.push_str("</template>");
                    result
                }
                DomStrategy::Light => {
                    if snippet.is_empty() {
                        trimmed.to_string()
                    } else {
                        let mut result = String::with_capacity(snippet.len() + trimmed.len());
                        result.push_str(snippet);
                        result.push_str(trimmed);
                        result
                    }
                }
            }
        };

        self.strip_template_comments(processed)
    }

    fn inject_css_snippet_into_template(html: String, snippet: &str) -> String {
        if snippet.is_empty() {
            return html;
        }

        match html::find_tag_close(&html) {
            Some(open_end) => {
                let mut result = String::with_capacity(html.len() + snippet.len());
                result.push_str(&html[..=open_end]);
                result.push_str(snippet);
                result.push_str(&html[open_end + 1..]);
                result
            }
            // Malformed opening tag (no closing `>`): emit as-is rather than
            // panic. The downstream parser surfaces the error.
            None => html,
        }
    }

    fn append_adopted_attr_if_missing(html: String, adopted_specifier: Option<&str>) -> String {
        let Some(adopted) = adopted_specifier else {
            return html;
        };
        let Some(tag) = html::parse_tag(&html) else {
            return html;
        };
        if tag.name != "template" || tag.closing || tag.has_attr(Self::ADOPTED_STYLESHEETS_ATTR) {
            return html;
        }

        let mut result = String::with_capacity(html.len() + Self::adopted_attr_len(adopted));
        result.push_str(&html[..tag.close]);
        Self::push_adopted_attr(&mut result, adopted);
        result.push_str(&html[tag.close..]);
        result
    }

    fn append_shadow_mode_if_missing(html: String) -> String {
        let Some(tag) = html::parse_tag(&html) else {
            return html;
        };
        if tag.name != "template" || tag.closing || tag.has_attr("shadowrootmode") {
            return html;
        }

        const SHADOW_MODE: &str = " shadowrootmode=\"open\"";
        let mut result = String::with_capacity(html.len() + SHADOW_MODE.len());
        result.push_str(&html[..tag.close]);
        result.push_str(SHADOW_MODE);
        result.push_str(&html[tag.close..]);
        result
    }

    fn unwrap_component_policy_template(html: String) -> String {
        let Some(tag) = html::parse_tag(&html) else {
            return html;
        };
        let inner_start = tag.close + 1;
        let inner_end = html.rfind("</template>").unwrap_or(html.len());
        if inner_start >= inner_end {
            return String::new();
        }
        html[inner_start..inner_end].to_string()
    }

    fn push_adopted_attr(out: &mut String, adopted: &str) {
        out.push(' ');
        out.push_str(Self::ADOPTED_STYLESHEETS_ATTR);
        out.push_str("=\"");
        out.push_str(adopted);
        out.push('"');
    }

    fn adopted_attr_len(adopted: &str) -> usize {
        4 + Self::ADOPTED_STYLESHEETS_ATTR.len() + adopted.len()
    }

    fn template_has_stripped_runtime_attrs(html: &str) -> bool {
        let (trimmed, _) = html::leading_content(html);
        let Some(tag) = html::parse_tag(trimmed) else {
            return false;
        };
        if tag.name != "template" || tag.closing {
            return false;
        }
        tag.attrs().any(|attr| {
            attr.name.starts_with('@') || attr.name.starts_with(':') || attr.name.starts_with('?')
        })
    }

    fn strip_template_comments(&mut self, html: String) -> Result<String> {
        let mut ranges = Vec::new();
        let mut style_ranges = Vec::new();
        Self::collect_html_comment_and_style_ranges(&html, &mut ranges, &mut style_ranges)?;

        for (style_start, style_end) in style_ranges {
            let css = &html[style_start..style_end];
            let (_tokens, _defs, _requirements, comments) = self
                .css_parser
                .extract_tokens_definitions_requirements_and_comments(
                    css,
                    self.options.legal_comments,
                )
                .map_err(|e| self.css_diagnostic(e))?;
            for comment in comments {
                let comment_text = &css[comment.start_byte..comment.end_byte];
                if comment.preserve || self.css_signal_comment_fragment(comment_text).is_some() {
                    continue;
                }
                ranges.push((
                    style_start + comment.start_byte,
                    style_start + comment.end_byte,
                ));
            }
        }

        if ranges.is_empty() {
            return Ok(html);
        }

        Ok(comment_policy::strip_ranges(&html, ranges.as_mut_slice()).into_owned())
    }

    /// Collect HTML comment ranges and `<style>` raw-text ranges in one scan.
    fn collect_html_comment_and_style_ranges(
        html: &str,
        comment_ranges: &mut Vec<(usize, usize)>,
        style_ranges: &mut Vec<(usize, usize)>,
    ) -> Result<()> {
        let mut index = 0usize;
        while index < html.len() {
            let remaining = &html[index..];
            if remaining.starts_with("<!--") {
                let Some(end) = html::find_comment_close(remaining) else {
                    return Err(Diagnostic::error("unterminated HTML comment")
                        .code(codes::UNTERMINATED_HTML_COMMENT)
                        .at_offset(html, index)
                        .help("close the comment with `-->`")
                        .into());
                };
                comment_ranges.push((index, index + end));
                index += end;
                continue;
            }

            if remaining.starts_with('<') {
                if let Some((style_start, style_end, close_end)) =
                    html::style_element_bounds(remaining)
                {
                    style_ranges.push((index + style_start, index + style_end));
                    index += close_end;
                    continue;
                }

                if let Some(tag) = html::parse_tag(remaining) {
                    index += tag.close + 1;
                    continue;
                }
            }

            let Some(ch) = remaining.chars().next() else {
                break;
            };
            index += ch.len_utf8();
        }
        Ok(())
    }

    /// Strip compiler-owned component policy attributes and, for the SSR view,
    /// runtime attributes starting with `@`, `:`, or `?` from the opening
    /// `<template>` tag.
    ///
    /// Uses the same quote-aware tag scanner as the main HTML pipeline.
    fn strip_template_build_attrs(&mut self, html: &str, preserve_runtime_attrs: bool) -> String {
        let Some(tag) = html::parse_tag(html) else {
            return html.to_string();
        };

        let mut removals: Vec<(usize, usize)> = Vec::new();
        for attr in tag.attrs() {
            let runtime_attr = attr.name.starts_with('@')
                || attr.name.starts_with(':')
                || attr.name.starts_with('?');
            let policy_attr = matches!(
                attr.name,
                COMPONENT_RENDER_ATTR | COMPONENT_HYDRATE_ATTR | COMPONENT_RESERVE_BLOCK_SIZE_ATTR
            );
            if policy_attr || (!preserve_runtime_attrs && runtime_attr) {
                let mut start = attr.raw_range.start;
                while start > 0 && html.as_bytes()[start - 1].is_ascii_whitespace() {
                    start -= 1;
                }
                removals.push((start, attr.raw_range.end));
            }
        }

        if removals.is_empty() {
            return html.to_string();
        }

        // Rebuild the string, skipping removed ranges
        let mut result = String::with_capacity(html.len());
        let mut pos = 0;
        for (start, end) in &removals {
            result.push_str(&html[pos..*start]);
            pos = *end;
        }
        result.push_str(&html[pos..]);
        result
    }
}

#[cfg(test)]
mod tests {
    use webui_test_utils::*;

    use super::*;

    fn structural_matcher(value: &str) -> FragmentMatcher {
        signal_raw(&format!("{STRUCTURAL_SIGNAL_PREFIX}{value}"))
    }

    #[test]
    fn test_plugin_display_names() {
        assert_eq!(Plugin::Fast.to_string(), "fast");
        assert_eq!(Plugin::FastV2.to_string(), "fast-v2");
        assert_eq!(Plugin::FastV3.to_string(), "fast-v3");
        assert_eq!(Plugin::WebUI.to_string(), "webui");
    }

    #[test]
    fn test_plugin_from_str_names() {
        assert_eq!("fast".parse::<Plugin>(), Ok(Plugin::Fast));
        assert_eq!("fast-v2".parse::<Plugin>(), Ok(Plugin::FastV2));
        assert_eq!("fast-v3".parse::<Plugin>(), Ok(Plugin::FastV3));
        assert_eq!("webui".parse::<Plugin>(), Ok(Plugin::WebUI));
    }

    #[test]
    fn test_plugin_from_str_unknown_mentions_supported_names() {
        let err = "legacy".parse::<Plugin>().unwrap_err();
        assert!(
            err.contains("fast-v3"),
            "error should mention fast-v3: {err}"
        );
        assert!(
            err.contains("fast-v2"),
            "error should mention fast-v2: {err}"
        );
        assert!(err.contains("fast"), "error should mention fast: {err}");
    }

    #[test]
    fn test_parse_signal() {
        let mut parser = HtmlParser::new();
        let html = "Hello, {{name}}!";
        let result = parser.parse("test.html", html);

        assert!(result.is_ok());
        let fragment_records = parser.into_fragment_records();

        assert_stream!(
            fragment_records,
            "test.html",
            [raw("Hello, "), signal("name"), raw("!"),]
        );
    }

    #[test]
    fn test_parse_raw_signal() {
        let mut parser = HtmlParser::new();
        let html = "Hello, {{{html_content}}}!";
        let result = parser.parse("test.html", html);

        assert!(result.is_ok());
        let fragment_records = parser.into_fragment_records();

        assert_stream!(
            fragment_records,
            "test.html",
            [raw("Hello, "), signal_raw("html_content"), raw("!"),]
        );
    }

    #[test]
    fn test_parse_preserves_inline_spaces_around_entity_between_bindings() {
        let mut parser = HtmlParser::new();
        let html = "<nav>{{sectionName}} &gt; {{topicName}}</nav>";
        let result = parser.parse("test.html", html);

        assert!(result.is_ok(), "Parse error: {:?}", result.err());
        let fragment_records = parser.into_fragment_records();

        assert_stream!(
            fragment_records,
            "test.html",
            [
                raw("<nav>"),
                signal("sectionName"),
                raw(" &gt; "),
                signal("topicName"),
                raw("</nav>"),
            ]
        );
    }

    #[test]
    fn test_invalid_for_each_is_template_diagnostic() {
        let mut parser = HtmlParser::new();
        let err = parser
            .parse(
                "index.html",
                "<div>\n  <for each=\"person inpeople\"><p>x</p></for>\n</div>",
            )
            .expect_err("invalid for-each must error");
        assert!(matches!(err, ParserError::Template(_)));
        let msg = err.to_string();
        assert!(msg.contains("invalid <for> each expression"), "{msg}");
        assert!(msg.contains(r#"each="person inpeople""#), "{msg}");
        // The <for> sits on line 2, column 3 (after the two-space indent),
        // reported rustc-style as `--> index.html:2:3`.
        assert!(
            msg.contains("--> index.html:2:3"),
            "missing line:column — {msg}"
        );
        assert!(msg.contains("help:"), "{msg}");
    }

    #[test]
    fn test_invalid_if_condition_is_template_diagnostic() {
        let mut parser = HtmlParser::new();
        let err = parser
            .parse("index.html", r#"<if condition="a +"><p>x</p></if>"#)
            .expect_err("invalid condition must error");
        assert!(matches!(err, ParserError::Template(_)));
        let msg = err.to_string();
        assert!(msg.contains("invalid <if> condition expression"), "{msg}");
        assert!(
            msg.contains("--> index.html:1:1"),
            "missing line:column — {msg}"
        );
    }

    #[test]
    fn test_parse_for_directive() {
        let mut parser = HtmlParser::new();
        let html = r#"<for each="item in items"><div class="item">{{item.name}}</div></for>"#;

        let result = parser.parse("test.html", html);
        assert!(result.is_ok(), "Parse error: {:?}", result.err());
        let fragment_records = parser.into_fragment_records();

        assert_stream!(
            fragment_records,
            "test.html",
            [for_loop("item", "items", "for-1"),]
        );

        // Verify the sub-fragment contains our item content
        assert_stream!(
            fragment_records,
            "for-1",
            [
                raw("<div class=\"item\">"),
                signal("item.name"),
                raw("</div>"),
            ]
        );
    }

    #[test]
    fn test_parse_if_directive() {
        let mut parser = HtmlParser::new();
        let html = r#"<if condition="isLoggedIn"><div>Welcome back, {{username}}!</div></if>"#;

        let result = parser.parse("test.html", html);
        assert!(result.is_ok(), "Parse error: {:?}", result.err());
        let fragment_records = parser.into_fragment_records();

        assert_stream!(fragment_records, "test.html", [if_cond("if-1"),]);

        // Verify the sub-fragment contains our content
        assert_stream!(
            fragment_records,
            "if-1",
            [
                raw("<div>Welcome back, "),
                signal("username"),
                raw("!</div>"),
            ]
        );
    }

    #[test]
    fn test_component_directive() {
        let mut parser = HtmlParser::with_options(DomStrategy::Light);
        parser
            .component_registry
            .register_component(ComponentRegistration::new(
                "my-component",
                "<div>My Component</div>",
                Some("div { color: blue; }"),
                true,
            ))
            .expect("Failed to register component");

        let result = parser.parse("test.html", "<my-component></my-component>");
        assert!(result.is_ok(), "Parse error: {:?}", result.err());
        let records = parser.into_fragment_records();

        assert_stream!(
            records,
            "test.html",
            [
                raw("<my-component"),
                structural_matcher("streaming_root:my-component"),
                raw(">"),
                component("my-component"),
                raw("</my-component>"),
            ]
        );

        // Component template stream should contain the component content (no shadow DOM wrapper)
        let comp = &records["my-component"].fragments;
        assert_eq!(comp.len(), 1);
        assert!(
            matches!(comp[0].fragment.as_ref(), Some(Fragment::Raw(raw)) if
                !raw.value.contains("<template shadowrootmode") && raw.value.contains("<div>My Component</div>"))
        );
    }

    #[test]
    fn streaming_root_signal_emitted_before_component_opening_tag_closes() {
        // The compiler-owned streamed SSR root signal must sit between the
        // component's attribute run and the closing `>`, carrying the tag,
        // so the streaming handler can inject ` data-ws` inside the tag before
        // custom-element upgrade. Ordinary rendering ignores the signal.
        let mut parser = HtmlParser::with_options(DomStrategy::Light);
        parser
            .component_registry
            .register_component(ComponentRegistration::new(
                "my-widget",
                "<div>w</div>",
                None,
                true,
            ))
            .expect("register");

        let result = parser.parse("test.html", "<my-widget></my-widget>");
        assert!(result.is_ok(), "Parse error: {:?}", result.err());
        let records = parser.into_fragment_records();

        assert_stream!(
            records,
            "test.html",
            [
                raw("<my-widget"),
                structural_matcher("streaming_root:my-widget"),
                raw(">"),
                component("my-widget"),
                raw("</my-widget>"),
            ]
        );
    }

    #[test]
    fn streaming_root_signal_emitted_per_component_when_nested_and_repeated() {
        // Each distinct component host emits its own streaming_root signal,
        // including nested hosts and hosts repeated across sibling positions.
        use webui_protocol::web_ui_fragment::Fragment;
        let mut parser = HtmlParser::with_options(DomStrategy::Light);
        for name in ["outer-box", "inner-pill"] {
            parser
                .component_registry
                .register_component(ComponentRegistration::new(
                    name,
                    "<slot></slot>",
                    None,
                    true,
                ))
                .expect("register");
        }

        let result = parser.parse(
            "test.html",
            "<outer-box><inner-pill></inner-pill></outer-box><inner-pill></inner-pill>",
        );
        assert!(result.is_ok(), "Parse error: {:?}", result.err());
        let records = parser.into_fragment_records();
        let fragments = &records["test.html"].fragments;

        let signals: Vec<&str> = fragments
            .iter()
            .filter_map(|f| match f.fragment.as_ref() {
                Some(Fragment::Signal(s)) if s.raw => Some(s.value.as_str()),
                _ => None,
            })
            .collect();
        assert_eq!(
            signals,
            [
                "}}}webui:streaming_root:outer-box",
                "}}}webui:streaming_root:inner-pill",
                "}}}webui:streaming_root:inner-pill",
            ]
        );

        // Every streaming_root signal is immediately followed by a `>` raw and
        // preceded by a raw ending in the host tag name (never a bare `<tag>`).
        for (i, frag) in fragments.iter().enumerate() {
            let Some(Fragment::Signal(s)) = frag.fragment.as_ref() else {
                continue;
            };
            if !s.raw || !s.value.starts_with("}}}webui:streaming_root:") {
                continue;
            }
            match fragments[i + 1].fragment.as_ref() {
                Some(Fragment::Raw(r)) => assert_eq!(r.value, ">"),
                other => panic!("expected raw(\">\") after signal, got {other:?}"),
            }
        }
    }

    #[test]
    fn unknown_component_typo_in_same_namespace_errors_with_suggestion() {
        let mut parser = HtmlParser::with_options(DomStrategy::Light);
        parser
            .component_registry
            .register_component(ComponentRegistration::new(
                "mp-button",
                "<button>b</button>",
                None,
                true,
            ))
            .expect("Failed to register component");

        // `<mp-buton>` is a same-namespace one-character typo of `mp-button`.
        let err = parser
            .parse("test.html", "<mp-buton></mp-buton>")
            .expect_err("a near-typo component tag should error");
        let ParserError::Template(diag) = err else {
            panic!("expected ParserError::Template, got {err:?}");
        };
        assert_eq!(diag.error_code(), Some(codes::UNKNOWN_COMPONENT));
        let help = diag
            .help_text()
            .expect("diagnostic should carry a help line");
        assert!(
            help.contains("did you mean <mp-button>?"),
            "expected a suggestion, got: {help}"
        );
    }

    #[test]
    fn external_custom_element_in_other_namespace_passes_through() {
        let mut parser = HtmlParser::with_options(DomStrategy::Light);
        parser
            .component_registry
            .register_component(ComponentRegistration::new(
                "mp-button",
                "<button>b</button>",
                None,
                true,
            ))
            .expect("Failed to register component");

        // `<md-button>` is a different namespace (md- vs mp-): a genuine
        // third-party custom element must pass through as raw HTML, not error.
        let result = parser.parse("test.html", "<md-button></md-button>");
        assert!(
            result.is_ok(),
            "external custom element should pass through, got: {:?}",
            result.err()
        );
    }

    #[test]
    fn for_missing_each_suggests_typoed_attribute() {
        let mut parser = HtmlParser::with_options(DomStrategy::Light);
        // `eahc` is a transposition of the required `each` attribute.
        let err = parser
            .parse("test.html", "<for eahc=\"todo in todos\"></for>")
            .expect_err("a <for> without `each` should error");
        let ParserError::Template(diag) = err else {
            panic!("expected ParserError::Template, got {err:?}");
        };
        assert_eq!(diag.error_code(), Some(codes::MISSING_FOR_EACH));
        let help = diag
            .help_text()
            .expect("diagnostic should carry a help line");
        assert!(
            help.contains("found `eahc`") && help.contains("did you mean"),
            "expected an attribute-typo suggestion, got: {help}"
        );
    }

    #[test]
    fn parent_diagnostic_owner_survives_nested_component_parse() {
        // Regression: a component parse recurses through `self.parse(child, …)`,
        // which used to leave `current_fragment_id` pointing at the child. A
        // later error in the PARENT template must still be attributed to the
        // parent (here `index.html`), not the nested component.
        let mut parser = HtmlParser::with_options(DomStrategy::Light);
        parser
            .component_registry
            .register_component(ComponentRegistration::new(
                "my-card",
                "<div>card</div>",
                None,
                true,
            ))
            .expect("register");

        // A registered component first, then a broken <for> in the same template.
        let html = r#"<my-card></my-card><for each="bad inval"></for>"#;
        let err = parser
            .parse("index.html", html)
            .expect_err("the malformed <for> should error");
        let ParserError::Template(diag) = err else {
            panic!("expected ParserError::Template, got {err:?}");
        };
        assert_eq!(diag.error_code(), Some(codes::INVALID_FOR_EACH));
        assert_eq!(
            diag.component_name(),
            Some("index.html"),
            "parent diagnostic must be owned by index.html, not the nested component"
        );
    }

    #[test]
    fn test_component_directive_with_slots() {
        let mut parser = HtmlParser::new();
        parser
            .component_registry
            .register_component(ComponentRegistration::new(
                "my-component",
                "<div>My Component</div>",
                Some("div { color: blue; }"),
                true,
            ))
            .expect("Failed to register component");

        let result = parser.parse(
            "test.html",
            "Hello<my-component><p>World</p></my-component>",
        );
        assert!(result.is_ok(), "Parse error: {:?}", result.err());
        let records = parser.into_fragment_records();
        let fragments = &records["test.html"].fragments;

        // Entry: raw(Hello<my-component>) + component + raw(<p>World</p></my-component>)
        assert!(fragments.len() >= 3);
        // First fragment should contain "Hello" and "<my-component" (the opening
        // tag is split by the compiler-owned streaming_root signal before `>`).
        assert!(
            matches!(fragments[0].fragment.as_ref(), Some(Fragment::Raw(raw)) if raw.value.contains("Hello") && raw.value.contains("<my-component"))
        );
        // Should have component fragment
        assert!(fragments.iter().any(|f| matches!(
            f.fragment.as_ref(),
            Some(Fragment::Component(c)) if c.fragment_id == "my-component"
        )));
        // Should end with closing tag
        let last = fragments.last().unwrap();
        assert!(
            matches!(last.fragment.as_ref(), Some(Fragment::Raw(raw)) if raw.value.contains("</my-component>"))
        );
    }

    fn assert_f_template_component_source(plugin: Box<dyn ParserPlugin>) {
        let mut parser =
            HtmlParser::with_plugin_options(plugin, (CssStrategy::Style, DomStrategy::Shadow));
        parser
            .component_registry
            .register_component(ComponentRegistration::new(
                "file-card",
                r#"<f-template name="named-card"><template><f-when value="{{visible}}"><f-repeat value="{{item in items}}"><button @click="{save()}" :config="{config}" ?disabled="{{disabled}}" f-ref="{button}" title="{{title}}">{{item.label}}</button></f-repeat></f-when></template></f-template>"#,
                Some(".root { color: red; }"),
                true,
            ))
            .expect("register component");

        parser
            .parse("index.html", "<named-card></named-card>")
            .expect("parse entry");
        let records = parser.fragment_records.clone();
        assert_stream!(
            records,
            "named-card",
            [
                raw("<template><style>.root { color: red; }</style>"),
                if_cond("if-1"),
                raw("</template>"),
            ]
        );
        assert_stream!(records, "if-1", [for_loop("item", "items", "for-1"),]);
        let for_fragments = &records["for-1"].fragments;
        // Binding count is 5: @click, :config, f-ref (FAST client bindings
        // stripped and counted via classify_attribute) plus ?disabled and
        // title (WebUI dynamic bindings).
        assert!(for_fragments.iter().any(|fragment| {
            matches!(
                fragment.fragment.as_ref(),
                Some(Fragment::Plugin(data)) if data.data == 5u32.to_le_bytes()
            )
        }));
        // No FAST client attribute leaks into the SSR raw stream.
        assert!(!for_fragments.iter().any(|fragment| {
            matches!(
                fragment.fragment.as_ref(),
                Some(Fragment::Raw(raw))
                    if raw.value.contains("@click")
                        || raw.value.contains(":config")
                        || raw.value.contains("f-ref")
            )
        }));

        let artifacts = parser.take_plugin_artifacts().expect("artifacts");
        let ParserPluginArtifacts::ComponentTemplates(templates) = artifacts else {
            panic!("expected component template artifacts");
        };
        assert_eq!(templates.len(), 1);
        assert_eq!(templates[0].tag_name, "named-card");
        let template = &templates[0].template;
        assert!(template.contains(r#"<f-template name="named-card">"#));
        assert!(template.contains("<style>.root { color: red; }</style>"));
        assert!(template.contains(r#"<f-when value="{{visible}}">"#));
        assert!(template.contains(r#"<f-repeat value="{{item in items}}">"#));
        assert!(template.contains(r#"@click="{save()}""#));
        assert!(template.contains(r#":config="{config}""#));
        assert!(template.contains(r#"?disabled="{{disabled}}""#));
        assert!(template.contains(r#"f-ref="{button}""#));
        assert!(template.contains(r#"title="{{title}}""#));
        assert!(!template.contains("file-card"));
    }

    #[test]
    fn fast_v2_plugin_uses_authored_f_template_source() {
        assert_f_template_component_source(Box::new(plugin::fast_v2::FastV2ParserPlugin::new()));
    }

    #[test]
    #[allow(deprecated)]
    fn fast_plugin_uses_authored_f_template_source() {
        assert_f_template_component_source(Box::new(plugin::fast::FastParserPlugin::new()));
    }

    #[test]
    fn fast_v3_plugin_uses_authored_f_template_source() {
        assert_f_template_component_source(Box::new(plugin::fast_v3::FastV3ParserPlugin::new()));
    }

    #[test]
    fn fast_client_attributes_are_counted_in_nested_source_order_without_markers() {
        let mut parser =
            HtmlParser::with_plugin(Box::new(plugin::fast_v3::FastV3ParserPlugin::new()));
        parser
            .component_registry
            .register_component(ComponentRegistration::new(
                "binding-card",
                r#"<f-template name="binding-card"><template><div @click="{save()}" :config="{config}" f-ref="{root}" f-slotted="{slot}" f-children="{children}" title="{{title}}"><span @focus="{focus()}" :value="{value}">{{label}}</span></div></template></f-template>"#,
                None,
                true,
            ))
            .expect("register component");

        parser
            .parse("index.html", "<binding-card></binding-card>")
            .expect("parse entry");
        let fragments = &parser.fragment_records["binding-card"].fragments;
        let binding_counts: Vec<u32> = fragments
            .iter()
            .filter_map(|fragment| {
                let Some(Fragment::Plugin(data)) = fragment.fragment.as_ref() else {
                    return None;
                };
                (data.data.len() == 4).then(|| {
                    u32::from_le_bytes([data.data[0], data.data[1], data.data[2], data.data[3]])
                })
            })
            .collect();
        assert_eq!(binding_counts, vec![6, 2]);

        for fragment in fragments {
            if let Some(Fragment::Raw(raw)) = fragment.fragment.as_ref() {
                for client_attr in [
                    "@click",
                    ":config",
                    "f-ref",
                    "f-slotted",
                    "f-children",
                    "@focus",
                    ":value",
                    "data-webui-internal-",
                ] {
                    assert!(!raw.value.contains(client_attr));
                }
            }
        }

        let ParserPluginArtifacts::ComponentTemplates(templates) =
            parser.take_plugin_artifacts().expect("artifacts")
        else {
            panic!("expected component template artifacts");
        };
        let template = &templates[0].template;
        for client_attr in [
            "@click",
            ":config",
            "f-ref",
            "f-slotted",
            "f-children",
            "@focus",
            ":value",
        ] {
            assert!(template.contains(client_attr));
        }
        assert!(!template.contains("data-webui-internal-"));
    }

    #[test]
    fn route_only_fast_component_uses_retained_authored_source() {
        let mut parser =
            HtmlParser::with_plugin(Box::new(plugin::fast_v2::FastV2ParserPlugin::new()));
        parser
            .component_registry
            .register_component(ComponentRegistration::new(
                "route-card",
                r#"<f-template name="route-card"><template><f-when value="{{visible}}"><span>{{label}}</span></f-when></template></f-template>"#,
                None,
                true,
            ))
            .expect("register component");

        parser
            .parse(
                "index.html",
                r#"<route path="/card" component="route-card" exact />"#,
            )
            .expect("parse route");
        assert!(parser.fragment_records.contains_key("route-card"));

        let ParserPluginArtifacts::ComponentTemplates(templates) =
            parser.take_plugin_artifacts().expect("artifacts")
        else {
            panic!("expected component template artifacts");
        };
        assert_eq!(templates.len(), 1);
        assert!(templates[0]
            .template
            .contains(r#"<f-when value="{{visible}}">"#));
    }

    #[test]
    fn fast_v2_plugin_converts_double_quoted_when_condition_without_malformed_output() {
        // `<f-when value='{{status == "ready"}}'>` must convert to a
        // single-quoted `<if condition='…'>` so the double-quoted literal is not
        // truncated. The stored SSR parser view must then parse cleanly.
        let mut parser =
            HtmlParser::with_plugin(Box::new(plugin::fast_v2::FastV2ParserPlugin::new()));
        parser
            .component_registry
            .register_component(ComponentRegistration::new(
                "status-card",
                r#"<f-template name="status-card"><template><f-when value='{{status == "ready"}}'><span>{{label}}</span></f-when></template></f-template>"#,
                None,
                true,
            ))
            .expect("register component");

        let parser_content = parser
            .component_registry
            .get("status-card")
            .map(|component| component.html_content.clone())
            .expect("registered component");
        assert_eq!(
            parser_content,
            r#"<template><if condition='status == "ready"'><span>{{label}}</span></if></template>"#
        );

        // The generated SSR view parses without a malformed-condition error.
        let mut ssr = HtmlParser::new();
        ssr.parse("status-card.html", &parser_content)
            .expect("converted parser content parses cleanly");
    }

    #[test]
    fn webui_plugin_leaves_f_template_shaped_source_inert() {
        // The WebUI plugin supplies no component-source transform, so
        // `<f-template>`-shaped markup is stored verbatim under the filename
        // tag: no rename, no FAST conversion, and no FAST diagnostics.
        let mut parser = HtmlParser::with_plugin(Box::new(plugin::webui::WebUIParserPlugin::new()));
        parser
            .component_registry
            .register_component(ComponentRegistration::new(
                "file-card",
                r#"<f-template name="named-card"><template><span>{{label}}</span></template></f-template>"#,
                None,
                true,
            ))
            .expect("register component");

        assert!(parser.component_registry.contains("file-card"));
        assert!(!parser.component_registry.contains("named-card"));
        assert_eq!(
            parser
                .component_registry
                .get("file-card")
                .map(|component| component.html_content.as_str()),
            Some(
                r#"<f-template name="named-card"><template><span>{{label}}</span></template></f-template>"#
            )
        );
        assert_eq!(
            parser
                .component_registry
                .component_artifact_source("file-card"),
            None
        );
    }

    #[test]
    fn default_parser_leaves_f_template_shaped_source_inert() {
        // With no plugin selected, `<f-template>`-shaped markup is inert too.
        let mut parser = HtmlParser::new();
        parser
            .component_registry
            .register_component(ComponentRegistration::new(
                "file-card",
                r#"<f-template name="named-card"><template><span>{{label}}</span></template></f-template>"#,
                None,
                true,
            ))
            .expect("register component");

        assert!(parser.component_registry.contains("file-card"));
        assert!(!parser.component_registry.contains("named-card"));
        assert_eq!(
            parser
                .component_registry
                .component_artifact_source("file-card"),
            None
        );
    }

    #[test]
    fn test_shadow_dom_shell_fragment_graph_includes_child_components() {
        // Reproduces commerce app: mp-app has a shadow DOM template containing
        // child components (mp-navbar, mp-cart-panel, mp-footer) plus an <outlet>.
        // The parser must emit Fragment::Component entries for ALL child components
        // so the inventory walk finds them.
        let mut parser = HtmlParser::with_options(DomStrategy::Shadow);

        parser
            .component_registry
            .register_component(ComponentRegistration::new(
                "app-shell",
                r#"<template shadowrootmode="open">
                  <my-navbar></my-navbar>
                  <main><outlet /></main>
                  <cart-panel></cart-panel>
                  <my-footer></my-footer>
                </template>"#,
                Some(":host{display:flex}"),
                true,
            ))
            .expect("register app-shell");
        parser
            .component_registry
            .register_component(ComponentRegistration::new(
                "my-navbar",
                "<nav>Nav</nav>",
                Some("nav{color:red}"),
                true,
            ))
            .expect("register my-navbar");
        parser
            .component_registry
            .register_component(ComponentRegistration::new(
                "cart-panel",
                "<aside>Cart</aside>",
                Some("aside{color:green}"),
                true,
            ))
            .expect("register cart-panel");
        parser
            .component_registry
            .register_component(ComponentRegistration::new(
                "my-footer",
                "<footer>Footer</footer>",
                Some("footer{color:blue}"),
                true,
            ))
            .expect("register my-footer");

        parser
            .parse("index.html", "<app-shell></app-shell>")
            .expect("parse index.html");
        let records = parser.into_fragment_records();

        // app-shell's fragment list must contain Component entries for all children
        let app_frags = &records["app-shell"].fragments;
        let component_ids: Vec<&str> = app_frags
            .iter()
            .filter_map(|f| match f.fragment.as_ref() {
                Some(Fragment::Component(c)) => Some(c.fragment_id.as_str()),
                _ => None,
            })
            .collect();

        assert!(
            component_ids.contains(&"my-navbar"),
            "app-shell fragments should contain my-navbar: {component_ids:?}"
        );
        assert!(
            component_ids.contains(&"cart-panel"),
            "app-shell fragments should contain cart-panel: {component_ids:?}"
        );
        assert!(
            component_ids.contains(&"my-footer"),
            "app-shell fragments should contain my-footer: {component_ids:?}"
        );
    }

    // ── Component template wrapping tests ────────────────────────────

    #[test]
    fn test_component_no_double_wrap_template() {
        // Developer-authored <template foo="bar"> must be preserved verbatim
        // in --dom=light. The framework only strips runtime-only attrs;
        // every other attribute is the developer's responsibility.
        let mut parser = HtmlParser::with_options(DomStrategy::Light);
        parser
            .component_registry
            .register_component(ComponentRegistration::new(
                "custom-element",
                r#"<template foo="bar"><slot></slot></template>"#,
                None,
                true,
            ))
            .expect("register");
        let result = parser.parse("index.html", "<custom-element>Hello</custom-element>");
        assert!(result.is_ok());
        let records = parser.into_fragment_records();

        assert_fragments!(
            records["index.html"].fragments,
            [
                raw("<custom-element"),
                structural_matcher("streaming_root:custom-element"),
                raw(">"),
                component("custom-element"),
                raw("Hello</custom-element>"),
            ]
        );

        assert_stream!(
            records,
            "custom-element",
            [raw(r#"<template foo="bar"><slot></slot></template>"#),]
        );
    }

    #[test]
    fn test_component_styled_no_double_wrap() {
        // --dom=light with a developer-supplied <template> wrapper preserves
        // the wrapper verbatim. CSS is the default `Link` strategy which
        // injects only in shadow DOM, so in light mode there is no CSS
        // snippet to splice into the wrapper.
        let mut parser = HtmlParser::with_options(DomStrategy::Light);
        parser
            .component_registry
            .register_component(ComponentRegistration::new(
                "custom-element",
                r#"<template foo="bar"><slot></slot></template>"#,
                Some("div { color: red; }"),
                true,
            ))
            .expect("register");
        let result = parser.parse("index.html", "<custom-element>Hello</custom-element>");
        assert!(result.is_ok());
        let records = parser.into_fragment_records();

        assert_stream!(
            records,
            "custom-element",
            [raw(r#"<template foo="bar"><slot></slot></template>"#),]
        );
    }

    #[test]
    fn test_component_strip_runtime_attrs() {
        // Runtime-only attributes (`@event`, `:bind`, `?cond`) are stripped
        // from the opening <template> tag, but the wrapper itself is
        // preserved. After stripping in this case the wrapper becomes
        // `<template>` with no attributes.
        let mut parser = HtmlParser::with_options(DomStrategy::Light);
        parser
            .component_registry
            .register_component(ComponentRegistration::new(
                "custom-element",
                r#"<template @click={foo} :bar="baz" ?bool="true"><slot></slot></template>"#,
                None,
                true,
            ))
            .expect("register");
        let result = parser.parse("index.html", "<custom-element>Hello</custom-element>");
        assert!(result.is_ok());
        let records = parser.into_fragment_records();

        assert_stream!(
            records,
            "custom-element",
            [raw("<template><slot></slot></template>"),]
        );
    }

    #[test]
    fn test_component_strip_runtime_attrs_does_not_match_attr_value_text() {
        let mut parser = HtmlParser::with_options(DomStrategy::Light);
        parser
            .component_registry
            .register_component(ComponentRegistration::new(
                "custom-element",
                r#"<template data-note="@click={foo}" @click={foo}><slot></slot></template>"#,
                None,
                true,
            ))
            .expect("register");
        let result = parser.parse("index.html", "<custom-element>Hello</custom-element>");
        assert!(result.is_ok());
        let records = parser.into_fragment_records();

        assert_stream!(
            records,
            "custom-element",
            [raw(
                r#"<template data-note="@click={foo}"><slot></slot></template>"#
            ),]
        );
    }

    #[test]
    fn test_component_strip_runtime_attrs_handles_duplicate_runtime_attrs() {
        let mut parser = HtmlParser::with_options(DomStrategy::Light);
        parser
            .component_registry
            .register_component(ComponentRegistration::new(
                "custom-element",
                r#"<template @click={foo} @click={bar}><slot></slot></template>"#,
                None,
                true,
            ))
            .expect("register");
        let result = parser.parse("index.html", "<custom-element>Hello</custom-element>");
        assert!(result.is_ok());
        let records = parser.into_fragment_records();

        assert_stream!(
            records,
            "custom-element",
            [raw("<template><slot></slot></template>"),]
        );
    }

    #[test]
    fn test_component_with_slots_and_attrs() {
        let mut parser = HtmlParser::new();
        parser
            .component_registry
            .register_component(ComponentRegistration::new(
                "custom-element",
                "<slot></slot>",
                None,
                true,
            ))
            .expect("register");
        let result = parser.parse(
            "index.html",
            r#"<custom-element appearance="subtle">Hello World</custom-element>"#,
        );
        assert!(result.is_ok());
        let records = parser.into_fragment_records();
        assert_fragments!(
            records["index.html"].fragments,
            [
                raw("<custom-element"),
                attr_raw_start("appearance", "subtle"),
                structural_matcher("streaming_root:custom-element"),
                raw(">"),
                component("custom-element"),
                raw("Hello World</custom-element>"),
            ]
        );
    }

    #[test]
    fn test_component_legacy_no_styles() {
        let mut parser = HtmlParser::with_options(DomStrategy::Light);
        parser
            .component_registry
            .register_component(ComponentRegistration::new(
                "custom-element",
                "<div>Custom Element</div>",
                None,
                true,
            ))
            .expect("register");
        let result = parser.parse("index.html", "<custom-element></custom-element>");
        assert!(result.is_ok());
        let records = parser.into_fragment_records();

        assert_fragments!(
            records["index.html"].fragments,
            [
                raw("<custom-element"),
                structural_matcher("streaming_root:custom-element"),
                raw(">"),
                component("custom-element"),
                raw("</custom-element>"),
            ]
        );

        assert_stream!(
            records,
            "custom-element",
            [raw("<div>Custom Element</div>"),]
        );
    }

    #[test]
    fn test_component_self_closing() {
        let mut parser = HtmlParser::new();
        parser
            .component_registry
            .register_component(ComponentRegistration::new(
                "custom-widget",
                "<div>Widget Content</div>",
                None,
                true,
            ))
            .expect("register");
        let result = parser.parse("index.html", r#"<custom-widget config="{{settings}}" />"#);
        assert!(result.is_ok());
        let records = parser.into_fragment_records();

        assert_fragments!(
            records["index.html"].fragments,
            [
                raw("<custom-widget"),
                attr_start("config", "settings"),
                structural_matcher("streaming_root:custom-widget"),
                raw("/>"),
                component("custom-widget"),
            ]
        );
    }

    #[test]
    fn test_component_nested_self_closing_in_slot() {
        let mut parser = HtmlParser::with_options(DomStrategy::Light);
        parser
            .component_registry
            .register_component(ComponentRegistration::new(
                "custom-icon",
                "<svg><slot></slot></svg>",
                None,
                true,
            ))
            .expect("register");
        let result = parser.parse(
            "index.html",
            r##"<custom-icon><use href="#icon-{{iconName}}" /></custom-icon>"##,
        );
        assert!(result.is_ok());
        let records = parser.into_fragment_records();

        assert_fragments!(
            records["index.html"].fragments,
            [
                raw("<custom-icon"),
                structural_matcher("streaming_root:custom-icon"),
                raw(">"),
                component("custom-icon"),
                raw("<use"),
                attr_template("href", "attr-1"),
                raw("/></custom-icon>"),
            ]
        );

        assert_stream!(records, "custom-icon", [raw("<svg><slot></slot></svg>"),]);
    }

    #[test]
    fn test_component_leading_boolean_attr_start() {
        let mut parser = HtmlParser::new();
        parser
            .component_registry
            .register_component(ComponentRegistration::new(
                "custom-element",
                "<slot></slot>",
                None,
                true,
            ))
            .expect("register");
        let result = parser.parse(
            "index.html",
            r#"<custom-element ?disabled="{{isDisabled}}" title="Hello"></custom-element>"#,
        );
        assert!(result.is_ok());
        let records = parser.into_fragment_records();

        assert_fragments!(
            records["index.html"].fragments,
            [
                raw("<custom-element"),
                // First dynamic attr: boolean with attrStart
                bool_attr_start("disabled", "isDisabled"),
                // Static attr after dynamic: rawValue
                attr_raw("title", "Hello"),
                structural_matcher("streaming_root:custom-element"),
                raw(">"),
                component("custom-element"),
                raw("</custom-element>"),
            ]
        );
    }

    #[test]
    fn test_component_boolean_predicate_preserves_condition_tree() {
        let mut parser = HtmlParser::new();
        parser
            .component_registry
            .register_component(ComponentRegistration::new(
                "custom-element",
                "<slot></slot>",
                None,
                true,
            ))
            .expect("register");
        let result = parser.parse(
            "index.html",
            r#"<custom-element ?disabled="{{page == 'dashboard'}}"></custom-element>"#,
        );
        assert!(result.is_ok());

        let records = parser.into_fragment_records();
        let fragments = &records["index.html"].fragments;
        match fragments
            .get(1)
            .and_then(|fragment| fragment.fragment.as_ref())
        {
            Some(webui_protocol::web_ui_fragment::Fragment::Attribute(attr)) => {
                assert_eq!(attr.name, "disabled");
                assert!(attr.attr_start);
                match attr
                    .condition_tree
                    .as_ref()
                    .and_then(|condition| condition.expr.as_ref())
                {
                    Some(webui_protocol::condition_expr::Expr::Predicate(pred)) => {
                        assert_eq!(pred.left, "page");
                        assert_eq!(pred.operator, 3);
                        assert_eq!(pred.right, "'dashboard'");
                    }
                    other => panic!("expected predicate condition tree, got {:?}", other),
                }
            }
            other => panic!("expected attribute fragment, got {:?}", other),
        }
    }

    #[test]
    fn test_component_meta_link_tags() {
        let (fragments, _) = parse_and_get_fragments(
            r#"<head><meta charset="utf-8" /><link rel="stylesheet" href="{{cssFile}}" /></head>"#,
        );
        assert!(fragments.len() >= 7);
        assert!(
            matches!(fragments[0].fragment.as_ref(), Some(Fragment::Raw(raw)) if raw.value == "<head>")
        );
        assert!(
            matches!(fragments[1].fragment.as_ref(), Some(Fragment::Signal(s)) if
                s.raw && s.value.strip_prefix(STRUCTURAL_SIGNAL_PREFIX) == Some("head_start"))
        );
        assert!(
            matches!(fragments[2].fragment.as_ref(), Some(Fragment::Raw(raw)) if raw.value.contains("<meta charset=\"utf-8\"") && raw.value.contains("<link"))
        );
        assert!(
            matches!(fragments[3].fragment.as_ref(), Some(Fragment::Attribute(a)) if a.name == "href" && a.value == "cssFile")
        );
        assert!(
            matches!(fragments[4].fragment.as_ref(), Some(Fragment::Raw(raw)) if raw.value.contains("/>"))
        );
        assert!(
            matches!(fragments[5].fragment.as_ref(), Some(Fragment::Signal(s)) if
                s.raw && s.value.strip_prefix(STRUCTURAL_SIGNAL_PREFIX) == Some("head_end"))
        );
        assert!(
            matches!(fragments[6].fragment.as_ref(), Some(Fragment::Raw(raw)) if raw.value.contains("</head>"))
        );
    }

    #[test]
    fn test_nested_directives() {
        let mut parser = HtmlParser::new();
        let html = r#"<for each="category in categories">
            <if condition="category.hasItems">
                <for each="item in category.items">
                   {{item.title}}
                </for>
            </if>
        </for>"#;

        let result = parser.parse("test.html", html);

        assert!(result.is_ok(), "Parse error: {:?}", result.err());
        let fragment_records = parser.into_fragment_records();

        assert_stream!(
            fragment_records,
            "test.html",
            [for_loop("category", "categories", "for-1"),]
        );

        assert_stream!(fragment_records, "for-1", [if_cond("if-1"),]);

        assert_stream!(
            fragment_records,
            "if-1",
            [for_loop("item", "category.items", "for-2"),]
        );

        assert_stream!(fragment_records, "for-2", [signal("item.title"),]);
    }

    #[test]
    fn test_complex_directives() {
        let mut parser = HtmlParser::new();
        let html = r#"<for each="category in categories">
            <div class="category">
                <h2>{{category.name}}</h2>
                <if condition="category.hasItems">
                    <ul>
                        <for each="item in category.items">
                            <li>{{item.title}}</li>
                        </for>
                    </ul>
                </if>
            </div>
        </for>"#;

        let result = parser.parse("test.html", html);

        assert!(result.is_ok(), "Parse error: {:?}", result.err());
        let fragment_records = parser.into_fragment_records();

        assert_stream!(
            fragment_records,
            "test.html",
            [for_loop("category", "categories", "for-1"),]
        );

        // Verify for fragments contains the category.name signal
        assert_stream!(
            fragment_records,
            "for-1",
            [
                raw("<div class=\"category\"><h2>"),
                signal("category.name"),
                raw("</h2>"),
                if_cond("if-1"),
                raw("</div>"),
            ]
        );

        // Verify nested if condition.
        assert_stream!(
            fragment_records,
            "if-1",
            [
                raw("<ul>"),
                for_loop("item", "category.items", "for-2"),
                raw("</ul>"),
            ]
        );

        // Verify nested for each.
        assert_stream!(
            fragment_records,
            "for-2",
            [raw("<li>"), signal("item.title"), raw("</li>"),]
        );
    }

    // ── Attribute fragment tests ─────────────────────────────────────────

    /// Helper to parse HTML and return the fragments for the entry stream.
    fn parse_and_get_fragments(html: &str) -> (Vec<WebUIFragment>, WebUIFragmentRecords) {
        let mut parser = HtmlParser::new();
        let result = parser.parse("index.html", html);
        assert!(result.is_ok(), "Parse error: {:?}", result.err());
        let records = parser.into_fragment_records();
        let fragments = records
            .get("index.html")
            .expect("Failed to get index.html fragment")
            .fragments
            .clone();
        (fragments, records)
    }

    /// Helper to parse HTML with a pre-registered component.
    fn parse_with_component(tag: &str, html: &str) -> (Vec<WebUIFragment>, WebUIFragmentRecords) {
        let mut parser = HtmlParser::new();
        parser
            .component_registry
            .register_component(ComponentRegistration::new(tag, "<div></div>", None, true))
            .expect("register");
        let result = parser.parse("index.html", html);
        assert!(result.is_ok(), "Parse error: {:?}", result.err());
        let records = parser.into_fragment_records();
        let fragments = records
            .get("index.html")
            .expect("Failed to get index.html fragment")
            .fragments
            .clone();
        (fragments, records)
    }

    #[test]
    fn test_attribute_handlebars_in_href() {
        // Port of: 'should process handlebars from attributes as signals'
        let (fragments, _) = parse_and_get_fragments(r#"<a href="{{url}}">{{name}}</a>"#);
        assert_fragments!(
            fragments,
            [
                raw("<a"),
                attr("href", "url"),
                raw(">"),
                signal("name"),
                raw("</a>"),
            ]
        );
    }

    #[test]
    fn test_attribute_boolean_with_handlebars() {
        // Port of: 'should process boolean attribute with handlebars expression'
        let (fragments, _) =
            parse_and_get_fragments("<button ?disabled={{isDisabled}}>Click</button>");
        assert_fragments!(
            fragments,
            [
                raw("<button"),
                bool_attr("disabled", "isDisabled"),
                raw(">Click</button>"),
            ]
        );
    }

    #[test]
    fn test_attribute_multiple_boolean() {
        // Port of: 'should process multiple boolean attributes'
        // <input ?checked={{isChecked}} ?disabled={{isDisabled}} />
        let (fragments, _) =
            parse_and_get_fragments("<input ?checked={{isChecked}} ?disabled={{isDisabled}} />");

        assert_fragments!(
            fragments,
            [
                raw("<input"),
                bool_attr("checked", "isChecked"),
                bool_attr("disabled", "isDisabled"),
                raw("/>"),
            ]
        );
    }

    #[test]
    fn test_attribute_boolean_and_regular_together() {
        // Port of: 'should process a boolean attribute and a regular attribute together'
        // <button ?disabled="{{isDisabled}}" type="button">Hi</button>
        let (fragments, _) = parse_and_get_fragments(
            r#"<button ?disabled="{{isDisabled}}" type="button">Hi</button>"#,
        );

        assert_fragments!(
            fragments,
            [
                raw("<button"),
                bool_attr("disabled", "isDisabled"),
                raw(" type=\"button\">Hi</button>"),
            ]
        );
    }

    #[test]
    fn test_attribute_boolean_sandwiched() {
        // Port of: 'should process a boolean attribute sandwiched between regular attributes'
        // <button version={{edition}} ?disabled="{{isDisabled}}" type="button">Hi</button>
        let (fragments, _) = parse_and_get_fragments(
            r#"<button version={{edition}} ?disabled="{{isDisabled}}" type="button">Hi</button>"#,
        );

        assert_fragments!(
            fragments,
            [
                raw("<button"),
                attr("version", "edition"),
                bool_attr("disabled", "isDisabled"),
                raw(" type=\"button\">Hi</button>"),
            ]
        );
    }

    #[test]
    fn test_attribute_boolean_ending() {
        // Port of: 'should process html ending with boolean attribute correctly'
        // <button version={{edition}} ?disabled="{{isDisabled}}">Hi</button>
        let (fragments, _) = parse_and_get_fragments(
            r#"<button version={{edition}} ?disabled="{{isDisabled}}">Hi</button>"#,
        );

        assert_fragments!(
            fragments,
            [
                raw("<button"),
                attr("version", "edition"),
                bool_attr("disabled", "isDisabled"),
                raw(">Hi</button>"),
            ]
        );
    }

    #[test]
    fn test_attribute_boolean_dotted_path() {
        // Port of: 'should process boolean attribute with dotted path'
        // <div ?checked={{layout.isPinned}}>Content</div>
        let (fragments, _) =
            parse_and_get_fragments("<div ?checked={{layout.isPinned}}>Content</div>");

        assert_fragments!(
            fragments,
            [
                raw("<div"),
                bool_attr("checked", "layout.isPinned"),
                raw(">Content</div>"),
            ]
        );
    }

    #[test]
    fn test_attribute_colon_prefixed_complex() {
        // Port of: 'should process colon-prefixed attribute with handlebars'
        // <my-component :config="{{settings}}"></my-component>
        let (fragments, _) = parse_with_component(
            "my-component",
            r#"<my-component :config="{{settings}}"></my-component>"#,
        );

        assert_fragments!(
            fragments,
            [
                raw("<my-component"),
                attr_complex_start(":config", "settings"),
                structural_matcher("streaming_root:my-component"),
                raw(">"),
                component("my-component"),
                raw("</my-component>"),
            ]
        );
    }

    #[test]
    fn test_attribute_multiple_colon_prefixed() {
        // Port of: 'should process multiple colon-prefixed complex attributes'
        // <my-component :prop1="{{val1}}" :prop2="{{val2}}"></my-component>
        let (fragments, _) = parse_with_component(
            "my-component",
            r#"<my-component :prop1="{{val1}}" :prop2="{{val2}}"></my-component>"#,
        );

        assert_fragments!(
            fragments,
            [
                raw("<my-component"),
                attr_complex_start(":prop1", "val1"),
                attr_complex(":prop2", "val2"),
                structural_matcher("streaming_root:my-component"),
                raw(">"),
                component("my-component"),
                raw("</my-component>"),
            ]
        );
    }

    #[test]
    fn test_blocked_complex_property_innerhtml() {
        let mut parser = HtmlParser::new();
        let result = parser.parse("index.html", r#"<div :innerHTML="{{content}}"></div>"#);
        assert!(
            result.is_err(),
            "Expected error for :innerHTML on native element"
        );
        let err = result.unwrap_err().to_string();
        assert!(
            err.contains("only allowed on custom elements"),
            "Error: {err}"
        );
    }

    #[test]
    fn test_blocked_complex_on_native_element() {
        let mut parser = HtmlParser::new();
        let result = parser.parse("index.html", r#"<div :data="{{config}}"></div>"#);
        assert!(
            result.is_err(),
            "Expected error for :data on native element"
        );
        let err = result.unwrap_err().to_string();
        assert!(
            err.contains("only allowed on custom elements"),
            "Error: {err}"
        );
    }

    #[test]
    fn test_blocked_complex_property_on_component() {
        let mut parser = HtmlParser::new();
        parser
            .component_registry
            .register_component(ComponentRegistration::new(
                "my-widget",
                "<div></div>",
                None,
                true,
            ))
            .expect("register");
        let result = parser.parse(
            "index.html",
            r#"<my-widget :innerHTML="{{html}}"></my-widget>"#,
        );
        assert!(
            result.is_err(),
            "Expected error for :innerHTML on component"
        );
        let err = result.unwrap_err().to_string();
        assert!(err.contains("HTML injection"), "Error: {err}");
    }

    #[test]
    fn test_allowed_complex_property() {
        // :config on a component should still work
        let (fragments, _) = parse_with_component(
            "my-component",
            r#"<my-component :config="{{settings}}"></my-component>"#,
        );
        assert_fragments!(
            fragments,
            [
                raw("<my-component"),
                attr_complex_start(":config", "settings"),
                structural_matcher("streaming_root:my-component"),
                raw(">"),
                component("my-component"),
                raw("</my-component>"),
            ]
        );
    }

    #[test]
    fn test_attribute_mixed_normal_boolean_colon() {
        // Port of: 'should process mixed normal, boolean, and colon-prefixed attributes'
        // <my-component id="comp" :config="{{settings}}" ?enabled="{{isEnabled}}"></my-component>
        let (fragments, _) = parse_with_component(
            "my-component",
            r#"<my-component id="comp" :config="{{settings}}" ?enabled="{{isEnabled}}"></my-component>"#,
        );

        assert_fragments!(
            fragments,
            [
                raw("<my-component"),
                attr_raw_start("id", "comp"),
                attr_complex(":config", "settings"),
                bool_attr("enabled", "isEnabled"),
                structural_matcher("streaming_root:my-component"),
                raw(">"),
                component("my-component"),
                raw("</my-component>"),
            ]
        );
    }

    #[test]
    fn test_attribute_reject_boolean_without_handlebars() {
        // Port of: 'should reject boolean attribute without handlebars'
        // <button ?checked="name"></button>
        let (fragments, _) = parse_and_get_fragments(r#"<button ?checked="name"></button>"#);

        // Boolean attribute is silently dropped
        assert_fragments!(fragments, [raw("<button></button>"),]);
    }

    #[test]
    fn test_attribute_reject_boolean_with_partial_handlebars() {
        // Port of: 'should reject boolean attribute with partial handlebars'
        // <button ?checked="Hello {{name}}"></button>
        let (fragments, _) =
            parse_and_get_fragments(r#"<button ?checked="Hello {{name}}"></button>"#);

        // Boolean attribute is silently dropped
        assert_fragments!(fragments, [raw("<button></button>"),]);
    }

    #[test]
    fn test_attribute_reject_boolean_with_plain_value() {
        // Port of: 'should reject boolean attribute with plain value'
        // <button ?disabled="true">Click</button>
        let (fragments, _) = parse_and_get_fragments(r#"<button ?disabled="true">Click</button>"#);

        // Boolean attribute is silently dropped
        assert_fragments!(fragments, [raw("<button>Click</button>"),]);
    }

    #[test]
    fn test_attribute_boolean_predicate_equal() {
        // Boolean attribute with == predicate expression
        let (fragments, _) =
            parse_and_get_fragments(r#"<div ?data-active="{{page == 'dashboard'}}">X</div>"#);
        assert_fragments!(
            fragments,
            [
                raw("<div"),
                bool_attr_predicate("data-active", "page", 3, "'dashboard'"),
                raw(">X</div>"),
            ]
        );
    }

    #[test]
    fn test_attribute_boolean_predicate_greater_than() {
        // Boolean attribute with > predicate expression
        let (fragments, _) = parse_and_get_fragments(r#"<span ?hidden="{{num > 9}}">X</span>"#);
        assert_fragments!(
            fragments,
            [
                raw("<span"),
                bool_attr_predicate("hidden", "num", 1, "9"),
                raw(">X</span>"),
            ]
        );
    }

    #[test]
    fn test_attribute_boolean_predicate_not_equal() {
        // Boolean attribute with != predicate expression
        let (fragments, _) =
            parse_and_get_fragments(r#"<a ?data-active="{{status != 'inactive'}}">X</a>"#);
        assert_fragments!(
            fragments,
            [
                raw("<a"),
                bool_attr_predicate("data-active", "status", 4, "'inactive'"),
                raw(">X</a>"),
            ]
        );
    }

    #[test]
    fn test_attribute_boolean_negation() {
        // Boolean attribute with negated expression
        let (fragments, _) =
            parse_and_get_fragments(r#"<button ?disabled="{{!isReady}}">X</button>"#);
        assert_fragments!(
            fragments,
            [
                raw("<button"),
                bool_attr_not("disabled", "isReady"),
                raw(">X</button>"),
            ]
        );
    }

    #[test]
    fn test_attribute_mixed_static_dynamic() {
        // Port of: 'should process mixed attributes correctly'
        // <textarea value="hello {{world}}">Hi</textarea>
        let (fragments, records) =
            parse_and_get_fragments(r#"<textarea value="hello {{world}}">Hi</textarea>"#);

        assert_fragments!(
            fragments,
            [
                raw("<textarea"),
                attr_template("value", "attr-1"),
                raw(">Hi</textarea>"),
            ]
        );

        // Verify the template sub-stream
        assert_stream!(records, "attr-1", [raw("hello "), signal("world"),]);
    }

    // ── Body signal tests ─────────────────────────────────────────────

    #[test]
    fn test_body_signals() {
        let (fragments, _) = parse_and_get_fragments("<body><app-shell></app-shell></body>");
        assert_fragments!(
            fragments,
            [
                raw("<body>"),
                structural_matcher("body_start"),
                raw("<app-shell></app-shell>"),
                structural_matcher("body_end"),
                raw("</body>"),
            ]
        );
    }

    // ── Empty for handling tests ──────────────────────────────────────

    #[test]
    fn test_empty_for_produces_nothing() {
        let (fragments, records) =
            parse_and_get_fragments(r#"<div><for each="item in items"></for></div>"#);
        assert_fragments!(fragments, [raw("<div></div>"),]);
        assert!(!records.contains_key("for-1"));
    }

    // ── Self-closing / void element tests ─────────────────────────────

    #[test]
    fn test_self_closing_svg_path() {
        let (fragments, _) =
            parse_and_get_fragments(r#"<svg width="19"><path d="foo" fill="currentcolor"/></svg>"#);
        assert_fragments!(
            fragments,
            [raw(
                r#"<svg width="19"><path d="foo" fill="currentcolor"/></svg>"#
            ),]
        );
    }

    #[test]
    fn test_html5_void_elements() {
        let (fragments, _) = parse_and_get_fragments(
            r#"<div><img src="test.jpg" alt="test"><br><hr><input type="text"></div>"#,
        );
        assert_fragments!(
            fragments,
            [raw(
                r#"<div><img src="test.jpg" alt="test"><br><hr><input type="text"></div>"#
            ),]
        );
    }

    #[test]
    fn test_self_closing_with_dynamic_attributes() {
        let (fragments, _) =
            parse_and_get_fragments(r#"<img src="{{imageUrl}}" alt="{{imageAlt}}" />"#);
        assert_fragments!(
            fragments,
            [
                raw("<img"),
                attr("src", "imageUrl"),
                attr("alt", "imageAlt"),
                raw("/>"),
            ]
        );
    }

    #[test]
    fn test_self_closing_with_boolean_attributes() {
        let (fragments, _) = parse_and_get_fragments(
            r#"<input type="checkbox" ?checked="{{isSelected}}" ?disabled="{{isDisabled}}" />"#,
        );
        assert_fragments!(
            fragments,
            [
                raw("<input type=\"checkbox\""),
                bool_attr("checked", "isSelected"),
                bool_attr("disabled", "isDisabled"),
                raw("/>"),
            ]
        );
    }

    #[test]
    fn test_multiple_self_closing_in_sequence() {
        let (fragments, _) =
            parse_and_get_fragments(r#"<img src="1.jpg" /><br /><img src="2.jpg" />"#);
        assert_fragments!(
            fragments,
            [raw(r#"<img src="1.jpg"/><br/><img src="2.jpg"/>"#),]
        );
    }

    #[test]
    fn test_self_closing_with_mixed_content() {
        let (fragments, _) =
            parse_and_get_fragments(r#"<div>Text before<img src="{{url}}" />Text after</div>"#);
        assert_fragments!(
            fragments,
            [
                raw("<div>Text before<img"),
                attr("src", "url"),
                raw("/>Text after</div>"),
            ]
        );
    }

    #[test]
    fn test_self_closing_svg_elements() {
        let (fragments, _) = parse_and_get_fragments(
            r#"<svg><circle cx="{{x}}" cy="{{y}}" r="5" /><rect width="10" height="10" /></svg>"#,
        );
        assert_fragments!(
            fragments,
            [
                raw("<svg><circle"),
                attr("cx", "x"),
                attr("cy", "y"),
                raw(r#" r="5"/><rect width="10" height="10"/></svg>"#),
            ]
        );
    }

    #[test]
    fn test_self_closing_inside_for_loop() {
        let (fragments, records) = parse_and_get_fragments(
            r#"<for each="item in items"><img src="{{item.url}}" /></for>"#,
        );
        assert_fragments!(fragments, [for_loop("item", "items", "for-1"),]);
        assert_stream!(
            records,
            "for-1",
            [raw("<img"), attr("src", "item.url"), raw("/>"),]
        );
    }

    #[test]
    fn test_self_closing_whitespace_variations() {
        let (fragments, _) =
            parse_and_get_fragments(r#"<img src="test.jpg"/><input type="text" /><br/>"#);
        assert_fragments!(
            fragments,
            [raw(r#"<img src="test.jpg"/><input type="text"/><br/>"#),]
        );
    }

    #[test]
    fn test_deeply_nested_self_closing() {
        let (fragments, _) = parse_and_get_fragments(
            r#"<div><section><article><img src="deep.jpg" /><br /></article></section></div>"#,
        );
        assert_fragments!(
            fragments,
            [raw(
                r#"<div><section><article><img src="deep.jpg"/><br/></article></section></div>"#
            ),]
        );
    }

    #[test]
    fn test_self_closing_vs_empty_regular_tags() {
        let (fragments, _) =
            parse_and_get_fragments(r#"<div></div><img src="test.jpg" /><span></span>"#);
        assert_fragments!(
            fragments,
            [raw(r#"<div></div><img src="test.jpg"/><span></span>"#),]
        );
    }

    // ── Feature 1: Custom template attribute on <for> ────────────────────

    #[test]
    fn test_for_custom_template_attribute() {
        // Port of: 'should process transient node for with template'
        let (fragments, records) = parse_and_get_fragments(
            r#"<for each="item in items" template="static"><span>Item</span></for>"#,
        );
        assert_fragments!(fragments, [for_loop("item", "items", "static"),]);
        assert_stream!(records, "static", [raw("<span>Item</span>"),]);
    }

    #[test]
    fn test_for_recursive_template() {
        // Port of: 'should process recursive transient nodes'
        let mut parser = HtmlParser::new();
        let html = r#"<for template="static" each="outerItem in outerItems"><div><span>{{outerItem.name}}</span><for template="static" each="innerItem in innerItems" /></div></for>"#;
        let result = parser.parse("index.html", html);
        assert!(result.is_ok(), "Parse error: {:?}", result.err());
        let records = parser.into_fragment_records();

        assert_fragments!(
            records["index.html"].fragments,
            [for_loop("outerItem", "outerItems", "static"),]
        );

        assert_stream!(
            records,
            "static",
            [
                raw("<div><span>"),
                signal("outerItem.name"),
                raw("</span>"),
                for_loop("innerItem", "innerItems", "static"),
                raw("</div>"),
            ]
        );
    }

    // ── Feature 2: <if> / <for> with multiple children ──────────────────

    #[test]
    fn test_if_multiple_children() {
        // Port of: 'should handle <if> with multiple children'
        let (fragments, records) =
            parse_and_get_fragments(r#"<if condition="valid"><p>hello</p><p>world</p></if>"#);
        assert_fragments!(fragments, [if_cond("if-1"),]);
        assert_stream!(records, "if-1", [raw("<p>hello</p><p>world</p>"),]);
    }

    #[test]
    fn if_condition_with_double_quoted_literal_round_trips_through_parser() {
        // The FAST converter emits a single-quoted `<if condition='…'>` when the
        // expression contains a double-quoted literal. The WebUI parser must
        // extract the full expression (no truncation at the embedded quote), so
        // SSR and client see the same predicate.
        let (fragments, _) =
            parse_and_get_fragments(r#"<if condition='status == "ready"'><span>ok</span></if>"#);
        let condition = fragments
            .iter()
            .find_map(|fragment| match fragment.fragment.as_ref() {
                Some(webui_protocol::web_ui_fragment::Fragment::IfCond(if_cond)) => {
                    if_cond.condition.as_ref()
                }
                _ => None,
            })
            .expect("if-condition fragment with a parsed condition");
        match condition.expr.as_ref() {
            Some(webui_protocol::condition_expr::Expr::Predicate(pred)) => {
                assert_eq!(pred.left, "status");
                assert_eq!(pred.right, "\"ready\"");
            }
            other => panic!("expected a predicate condition, got {other:?}"),
        }
    }

    #[test]
    fn test_for_multiple_children() {
        // Port of: 'should handle <for> with multiple children'
        let (fragments, records) =
            parse_and_get_fragments(r#"<for each="item in items"><p>hello</p><p>world</p></for>"#);
        assert_fragments!(fragments, [for_loop("item", "items", "for-1"),]);
        assert_stream!(records, "for-1", [raw("<p>hello</p><p>world</p>"),]);
    }

    // ── Feature 3: Handlebars at beginning/end of text ──────────────────

    #[test]
    fn test_handlebars_at_beginning() {
        // Port of: 'should process handlebars from text at beginning'
        let (fragments, _) = parse_and_get_fragments("{{first}}");
        assert_fragments!(fragments, [signal("first"),]);
    }

    #[test]
    fn test_handlebars_at_beginning_and_raw() {
        // Port of: 'should process handlebars from text at beginning and raw'
        let (fragments, _) = parse_and_get_fragments("{{first}}test");
        assert_fragments!(fragments, [signal("first"), raw("test"),]);
    }

    #[test]
    fn test_handlebars_raw_and_end() {
        // Port of: 'should process handlebars from text at raw and end'
        let (fragments, _) = parse_and_get_fragments("test{{first}}");
        assert_fragments!(fragments, [raw("test"), signal("first"),]);
    }

    // ── Feature 4: Handlebars edge cases ────────────────────────────────

    #[test]
    fn test_handlebars_invalid_triple_open() {
        // Port of: 'should not process handlebars when invalid'
        let (fragments, _) = parse_and_get_fragments("{{{invalid}}");
        assert_fragments!(fragments, [raw("{{{invalid}}"),]);
    }

    #[test]
    fn test_handlebars_four_open_braces() {
        // Port of: 'should not process handlebars when invalid since triple exists'
        let (fragments, _) = parse_and_get_fragments("{{{{invalid}}");
        assert_fragments!(fragments, [raw("{{{{invalid}}"),]);
    }

    #[test]
    fn test_handlebars_five_open_with_valid_double() {
        // Port of: 'should not process handlebars when invalid but with valid triple'
        let (fragments, _) = parse_and_get_fragments("{{{{{invalid}}");
        assert_fragments!(fragments, [raw("{{{"), signal("invalid"),]);
    }

    #[test]
    fn test_entities_preserved() {
        // Port of: 'should process entities correctly'
        let (fragments, _) = parse_and_get_fragments("<p>Hello&#125;World</p>");
        assert_fragments!(fragments, [raw("<p>Hello&#125;World</p>"),]);
    }

    // ── Feature 5: DOCTYPE handling ─────────────────────────────────────

    #[test]
    fn test_doctype_preserved() {
        // DOCTYPE should be preserved as raw content
        let (fragments, _) = parse_and_get_fragments("<!DOCTYPE html><html><head></head></html>");
        assert!(!fragments.is_empty());
        assert!(
            matches!(fragments[0].fragment.as_ref(), Some(Fragment::Raw(raw)) if raw.value.contains("<!DOCTYPE html>"))
        );
    }

    // ── Feature 6: Component attribute skip / multiple nested ───────────

    #[test]
    fn test_component_attr_skip() {
        // Port of: 'should set attrSkip for skipped component attributes'
        let mut parser = HtmlParser::new();
        parser
            .component_registry
            .register_component(ComponentRegistration::new(
                "custom-element",
                "<slot></slot>",
                None,
                true,
            ))
            .expect("register");
        let html = r#"<custom-element :config="{{config}}" class="{{value0}}" style="{{value1}}" role="{{value2}}" data-test="{{value3}}" aria-test="{{value4}}"></custom-element>"#;
        let result = parser.parse("index.html", html);
        assert!(result.is_ok(), "Parse error: {:?}", result.err());
        let records = parser.into_fragment_records();

        // <custom-element, :config(attrStart), class(attrSkip), style(attrSkip),
        // role(attrSkip), data-test(attrSkip), aria-test(attrSkip), >, component, </custom-element>
        assert_fragments!(
            records["index.html"].fragments,
            [
                raw("<custom-element"),
                // :config with attrStart
                attr_complex_start(":config", "config"),
                // Skipped attrs
                attr_skip("class", "value0"),
                attr_skip("style", "value1"),
                attr_skip("role", "value2"),
                attr_skip("data-test", "value3"),
                attr_skip("aria-test", "value4"),
                structural_matcher("streaming_root:custom-element"),
                raw(">"),
                component("custom-element"),
                raw("</custom-element>"),
            ]
        );
    }

    #[test]
    fn test_component_attr_skip_static_and_embedded() {
        // Skipped attrs with static values and embedded bindings should
        // emit fragments (not be silently dropped).
        let mut parser = HtmlParser::new();
        parser
            .component_registry
            .register_component(ComponentRegistration::new(
                "item-group",
                "<slot></slot>",
                None,
                true,
            ))
            .expect("register");

        let html = r#"<item-group role="list" aria-labelledby="group-date-{{group.id}}" data-testid="grp-{{group.id}}" class="fixed-class"></item-group>"#;
        let result = parser.parse("index.html", html);
        assert!(result.is_ok(), "Parse error: {:?}", result.err());
        let records = parser.into_fragment_records();

        assert_fragments!(
            records["index.html"].fragments,
            [
                raw("<item-group"),
                attr_skip_raw("role", "list"),
                attr_skip_template("aria-labelledby", "attr-1"),
                attr_skip_template("data-testid", "attr-2"),
                attr_skip_raw("class", "fixed-class"),
                structural_matcher("streaming_root:item-group"),
                raw(">"),
                component("item-group"),
                raw("</item-group>"),
            ]
        );

        // Verify the embedded-binding template sub-streams exist and
        // contain the expected static + signal fragments.
        assert_stream!(records, "attr-1", [raw("group-date-"), signal("group.id"),]);
        assert_stream!(records, "attr-2", [raw("grp-"), signal("group.id"),]);
    }

    #[test]
    fn test_component_multiple_nested() {
        // Port of: 'handle multiple nested web components'
        let mut parser = HtmlParser::with_options(DomStrategy::Light);
        parser
            .component_registry
            .register_component(ComponentRegistration::new(
                "custom-element",
                "<custom-child></custom-child><slot></slot>",
                None,
                true,
            ))
            .expect("register");
        parser
            .component_registry
            .register_component(ComponentRegistration::new(
                "custom-button",
                "<slot></slot>",
                None,
                true,
            ))
            .expect("register");
        parser
            .component_registry
            .register_component(ComponentRegistration::new(
                "custom-child",
                "<h1>Hello World!</h1>",
                None,
                true,
            ))
            .expect("register");

        let html = r#"<for each="item in items"><custom-element><custom-button>Ok</custom-button></custom-element></for>"#;
        let result = parser.parse("index.html", html);
        assert!(result.is_ok(), "Parse error: {:?}", result.err());
        let records = parser.into_fragment_records();

        // Entry stream
        assert_fragments!(
            records["index.html"].fragments,
            [for_loop("item", "items", "for-1"),]
        );

        // For stream
        assert_stream!(
            records,
            "for-1",
            [
                raw("<custom-element"),
                structural_matcher("streaming_root:custom-element"),
                raw(">"),
                component("custom-element"),
                raw("<custom-button"),
                structural_matcher("streaming_root:custom-button"),
                raw(">"),
                component("custom-button"),
                raw("Ok</custom-button></custom-element>"),
            ]
        );

        // Component streams — custom-element has contains() checks, keep manual
        let ce = &records["custom-element"].fragments;
        assert_eq!(ce.len(), 5);
        assert!(
            matches!(ce[0].fragment.as_ref(), Some(Fragment::Raw(raw)) if raw.value.starts_with("<custom-child"))
        );
        assert!(
            matches!(ce[3].fragment.as_ref(), Some(Fragment::Component(c)) if c.fragment_id == "custom-child")
        );
        assert!(
            matches!(ce[4].fragment.as_ref(), Some(Fragment::Raw(raw)) if raw.value.contains("</custom-child><slot></slot>"))
        );

        assert_stream!(records, "custom-button", [raw("<slot></slot>"),]);

        assert_stream!(records, "custom-child", [raw("<h1>Hello World!</h1>"),]);
    }

    // ── Error handling tests ──────────────────────────────────────────

    #[test]
    fn test_invalid_markup_returns_error() {
        // Port of: 'should fail with invalid markup'
        let mut parser = HtmlParser::new();
        let result = parser.parse("index.html", "<div><span>Unclosed div");
        assert!(matches!(result, Err(ParserError::Template(diag)) if
            diag.to_string().contains("unclosed <div> tag")
        ));
    }

    #[test]
    fn test_unterminated_opening_tag_returns_error() {
        let mut parser = HtmlParser::new();
        let result = parser.parse("index.html", "before <div");
        assert!(matches!(result, Err(ParserError::Template(diag)) if
            diag.to_string().contains("malformed HTML tag")
        ));
    }

    #[test]
    fn test_unterminated_html_comment_returns_error() {
        let mut parser = HtmlParser::new();
        let result = parser.parse("index.html", "<div><!-- missing close</div>");
        assert!(matches!(result, Err(ParserError::Template(diag)) if
            diag.to_string().contains("unclosed <div> tag")
                || diag.to_string().contains("unterminated HTML comment")
        ));
    }

    #[test]
    fn test_unexpected_closing_tag_returns_error() {
        let mut parser = HtmlParser::new();
        let result = parser.parse("index.html", "</span>");
        assert!(matches!(result, Err(ParserError::Template(diag)) if
            diag.to_string().contains("unexpected closing tag </span>")
        ));
    }

    #[test]
    fn test_rejects_templates_over_size_limit() {
        let mut parser = HtmlParser::new();
        let html = "x".repeat(MAX_TEMPLATE_BYTES + 1);
        let result = parser.parse("index.html", &html);

        assert!(matches!(result, Err(ParserError::Html(message)) if
            message.contains("exceeds") && message.contains("parser limit")
        ));
    }

    #[test]
    fn test_rejects_templates_over_depth_limit() {
        let mut parser = HtmlParser::new();
        let mut html = String::with_capacity((MAX_TEMPLATE_DEPTH + 2) * 11);
        for _ in 0..=MAX_TEMPLATE_DEPTH {
            html.push_str("<div>");
        }
        for _ in 0..=MAX_TEMPLATE_DEPTH {
            html.push_str("</div>");
        }

        let result = parser.parse("index.html", &html);

        assert!(matches!(result, Err(ParserError::Template(diag)) if
            diag.to_string().contains("nesting exceeds")
        ));
    }

    #[test]
    fn test_rejects_routes_over_depth_limit() {
        let mut parser = HtmlParser::new();
        parser
            .component_registry
            .register_component(ComponentRegistration::new(
                "route-page",
                "<slot></slot>",
                None,
                true,
            ))
            .expect("register");

        let depth = MAX_TEMPLATE_DEPTH + 2;
        let mut html = String::with_capacity(depth * 48);
        for _ in 0..depth {
            html.push_str(r#"<route path="x" component="route-page">"#);
        }
        for _ in 0..depth {
            html.push_str("</route>");
        }

        let result = parser.parse("index.html", &html);

        assert!(matches!(result, Err(ParserError::Template(diag)) if
            diag.error_code() == Some(codes::EXCESSIVE_NESTING)
                && diag.to_string().contains("route nesting exceeds")
        ));
    }

    #[test]
    fn test_rejects_recursive_component_templates() {
        let mut parser = HtmlParser::new();
        parser
            .component_registry
            .register_component(ComponentRegistration::new(
                "self-card",
                "<self-card></self-card>",
                None,
                true,
            ))
            .expect("register");

        let result = parser.parse("index.html", "<self-card></self-card>");

        assert!(matches!(result, Err(ParserError::Template(diag)) if
            diag.error_code() == Some(codes::RECURSIVE_TEMPLATE)
                && diag.to_string().contains("recursive template reference")
        ));
    }

    // ── Integration tests ─────────────────────────────────────────────

    #[test]
    fn test_complex_raw_text_full_page() {
        // Port of: 'should process a complex raw text'
        let html = r#"<!DOCTYPE HTML><html dir="auto" lang="en"><head><meta charset="utf-8"><title>Test</title><style>html { margin: 0; }</style></head><body><app-shell></app-shell><script type="module" src="./index.js"></script></body></html>"#;
        let (fragments, _) = parse_and_get_fragments(html);

        // DOCTYPE + <head>, head_start, head content, head_end, </head><body>,
        // body_start, body content, body_end, </body></html>
        assert!(fragments.len() >= 9);
        assert!(
            matches!(fragments[0].fragment.as_ref(), Some(Fragment::Raw(raw)) if
                raw.value.contains("<!DOCTYPE HTML>") && raw.value.ends_with("<head>"))
        );
        assert!(
            matches!(fragments[1].fragment.as_ref(), Some(Fragment::Signal(s)) if
                s.raw && s.value.strip_prefix(STRUCTURAL_SIGNAL_PREFIX) == Some("head_start"))
        );
        assert!(
            matches!(fragments[2].fragment.as_ref(), Some(Fragment::Raw(raw)) if
                raw.value.contains("<title>Test</title>"))
        );
        assert!(
            matches!(fragments[3].fragment.as_ref(), Some(Fragment::Signal(s)) if
                s.raw && s.value.strip_prefix(STRUCTURAL_SIGNAL_PREFIX) == Some("head_end"))
        );
        assert!(
            matches!(fragments[4].fragment.as_ref(), Some(Fragment::Raw(raw)) if
                raw.value.contains("</head>") && raw.value.ends_with("<body>"))
        );
        assert!(
            matches!(fragments[5].fragment.as_ref(), Some(Fragment::Signal(s)) if
                s.raw && s.value.strip_prefix(STRUCTURAL_SIGNAL_PREFIX) == Some("body_start"))
        );
        assert!(
            matches!(fragments[6].fragment.as_ref(), Some(Fragment::Raw(raw)) if
                raw.value.contains("<app-shell>"))
        );
        assert!(
            matches!(fragments[7].fragment.as_ref(), Some(Fragment::Signal(s)) if
                s.raw && s.value.strip_prefix(STRUCTURAL_SIGNAL_PREFIX) == Some("body_end"))
        );
        assert!(
            matches!(fragments[8].fragment.as_ref(), Some(Fragment::Raw(raw)) if
                raw.value.contains("</body>") && raw.value.contains("</html>"))
        );
    }

    #[test]
    fn test_css_strategy_external_emits_link_tag() {
        let mut parser = HtmlParser::new();
        parser
            .component_registry_mut()
            .register_component(ComponentRegistration::new(
                "my-card",
                "<p><slot></slot></p>",
                Some("p { color: red; }"),
                true,
            ))
            .ok();
        parser.parse("index.html", "<my-card>Hello</my-card>").ok();
        let records = parser.into_fragment_records();
        let my_card = &records["my-card"].fragments;
        let raw_text: String = my_card
            .iter()
            .filter_map(|f| match &f.fragment {
                Some(Fragment::Raw(r)) => Some(r.value.as_str()),
                _ => None,
            })
            .collect();
        assert!(
            raw_text.contains(r#"<link rel="stylesheet" href="my-card.css">"#),
            "Expected external <link> tag in: {}",
            raw_text
        );
    }

    #[test]
    fn test_uppercase_style_element_is_processed_as_css() {
        // HTML tag names are ASCII case-insensitive: an uppercase `<STYLE>`
        // block must be CSS-processed exactly like `<style>`, so a CSS signal
        // comment becomes a Signal fragment (not inert raw text) and the
        // reconstructed closing tag mirrors the source case.
        let (fragments, _) = parse_and_get_fragments("<STYLE>/*{{tokens}}*/.a{color:red}</STYLE>");
        assert!(
            fragments.iter().any(|f| matches!(
                f.fragment.as_ref(),
                Some(Fragment::Signal(s)) if s.value == "tokens" && !s.raw
            )),
            "uppercase <STYLE> should be CSS-processed, got: {:?}",
            fragments
        );
        let raw: String = fragments
            .iter()
            .filter_map(|f| match f.fragment.as_ref() {
                Some(Fragment::Raw(r)) => Some(r.value.as_str()),
                _ => None,
            })
            .collect();
        assert!(
            raw.contains("</STYLE>"),
            "closing tag should preserve source case, got: {raw}"
        );
    }

    #[test]
    fn test_css_strategy_inline_emits_style_tag() {
        let mut parser = HtmlParser::with_options(CssStrategy::Style);
        parser
            .component_registry_mut()
            .register_component(ComponentRegistration::new(
                "my-card",
                "<p><slot></slot></p>",
                Some("p { color: red; }"),
                true,
            ))
            .ok();
        parser.parse("index.html", "<my-card>Hello</my-card>").ok();
        let records = parser.into_fragment_records();
        let my_card = &records["my-card"].fragments;
        let raw_text: String = my_card
            .iter()
            .filter_map(|f| match &f.fragment {
                Some(Fragment::Raw(r)) => Some(r.value.as_str()),
                _ => None,
            })
            .collect();
        assert!(
            raw_text.contains("<style>p { color: red; }</style>"),
            "Expected inline <style> tag in: {}",
            raw_text
        );
        assert!(
            !raw_text.contains("<link"),
            "Should not have <link> tag in inline mode: {}",
            raw_text
        );
    }

    #[test]
    fn test_css_strategy_module_emits_adopted_stylesheets() {
        let mut parser = HtmlParser::with_options((CssStrategy::Module, DomStrategy::Light));
        parser
            .component_registry_mut()
            .register_component(ComponentRegistration::new(
                "my-card",
                "<p><slot></slot></p>",
                Some("p { color: red; }"),
                true,
            ))
            .ok();
        parser.parse("index.html", "<my-card>Hello</my-card>").ok();
        let records = parser.into_fragment_records();

        // Component template should have shadowrootadoptedstylesheets, no CSS
        // module in raw fragments (CSS lives on the component fragment's css field)
        let my_card = &records["my-card"].fragments;
        let template_text: String = my_card
            .iter()
            .filter_map(|f| match &f.fragment {
                Some(Fragment::Raw(r)) => Some(r.value.as_str()),
                _ => None,
            })
            .collect();
        assert!(
            !template_text.contains("shadowrootadoptedstylesheets"),
            "shadowrootadoptedstylesheets should NOT be in HTML output (now in metadata only): {template_text}"
        );
        assert!(
            !template_text.contains("<link"),
            "Should not have <link> in module mode: {template_text}"
        );
        // No CSS module baked into raw fragments — CSS is stored in
        // protocol.components, populated by the build system, and emitted
        // by the handler (SSR inline + SPA partials) as importmap scripts.
        assert!(
            !template_text.contains(r#"<style type="module""#),
            "CSS module should NOT be in raw fragments (legacy shape): {template_text}"
        );
        assert!(
            !template_text.contains(r#"<script type="importmap""#),
            "CSS module importmap should NOT be in raw fragments: {template_text}"
        );
    }

    #[test]
    fn test_css_strategy_module_no_css_no_adopted_attr() {
        let mut parser = HtmlParser::with_options(CssStrategy::Module);
        parser
            .component_registry_mut()
            .register_component(ComponentRegistration::new(
                "my-card",
                "<p><slot></slot></p>",
                None,
                true,
            ))
            .ok();
        parser.parse("index.html", "<my-card>Hello</my-card>").ok();
        let records = parser.into_fragment_records();
        let my_card = &records["my-card"].fragments;
        let template_text: String = my_card
            .iter()
            .filter_map(|f| match &f.fragment {
                Some(Fragment::Raw(r)) => Some(r.value.as_str()),
                _ => None,
            })
            .collect();
        // No CSS → no shadowrootadoptedstylesheets attribute
        assert!(
            !template_text.contains("shadowrootadoptedstylesheets"),
            "Should not have adopted attr without CSS: {template_text}"
        );
    }

    // ── Dev-authored <template> wrapper handling ────────────────────
    //
    // These tests verify that when a developer includes a `<template>`
    // wrapper in their component HTML, the framework respects it instead
    // of stripping/normalizing it:
    //
    //   --dom=light  : dev `<template ...>` preserved verbatim (including
    //                  signal-fragment attrs like `foo="{{foo}}"`); no
    //                  wrapper added when dev omits one.
    //   --dom=shadow : dev `<template ...>` preserved verbatim (framework
    //                  does NOT inject `shadowrootmode="open"` or overwrite
    //                  a dev-supplied `shadowrootmode="closed"`); wrapper
    //                  added only when dev omits one.
    //
    // Calls `process_component_template` directly so the assertions observe
    // the exact HTML string the framework emits for the component template
    // (this is what gets handed to the inner parse + plugin hooks).

    #[test]
    fn light_preserves_dev_template_with_static_attrs() {
        let mut parser = HtmlParser::with_options(DomStrategy::Light);
        let processed = parser
            .process_component_template(
                r#"<template foo="bar"><div>hi</div></template>"#,
                None,
                None,
            )
            .expect("process failed");
        assert!(
            processed.contains(r#"<template foo="bar">"#),
            "[--dom=light] expected dev <template foo=\"bar\"> preserved verbatim, got: {processed}"
        );
        assert!(
            processed.contains("</template>"),
            "[--dom=light] expected closing </template> preserved, got: {processed}"
        );
    }

    #[test]
    fn light_preserves_dev_template_with_signal_fragment_attrs() {
        let mut parser = HtmlParser::with_options(DomStrategy::Light);
        let processed = parser
            .process_component_template(
                r#"<template foo="{{foo}}"><div>hi</div></template>"#,
                None,
                None,
            )
            .expect("process failed");
        assert!(
            processed.contains(r#"<template foo="{{foo}}">"#),
            "[--dom=light] expected dev <template foo=\"{{{{foo}}}}\"> with signal preserved, got: {processed}"
        );
    }

    #[test]
    fn light_preserves_dev_template_with_multiple_attrs() {
        let mut parser = HtmlParser::with_options(DomStrategy::Light);
        let processed = parser.process_component_template(
            r#"<template autofocus tabindex="0" role="region" data-x="y"><div>hi</div></template>"#,
            None,
            None,
        )
        .expect("process failed");
        assert!(
            processed.contains("autofocus")
                && processed.contains(r#"tabindex="0""#)
                && processed.contains(r#"role="region""#)
                && processed.contains(r#"data-x="y""#),
            "[--dom=light] expected ALL dev template attrs preserved, got: {processed}"
        );
    }

    #[test]
    fn light_no_template_no_wrapper_added() {
        let mut parser = HtmlParser::with_options(DomStrategy::Light);
        let processed = parser
            .process_component_template("<div>hi</div>", None, None)
            .expect("process failed");
        assert!(
            !processed.contains("<template"),
            "[--dom=light] framework must NOT add <template> wrapper when dev omits one, got: {processed}"
        );
        assert!(
            processed.contains("<div>hi</div>"),
            "[--dom=light] expected inner content emitted as-is, got: {processed}"
        );
    }

    #[test]
    fn shadow_preserves_dev_template_with_static_attrs() {
        let mut parser = HtmlParser::with_options(DomStrategy::Shadow);
        let processed = parser
            .process_component_template(
                r#"<template foo="bar"><div>hi</div></template>"#,
                None,
                None,
            )
            .expect("process failed");
        assert!(
            processed.contains(r#"<template foo="bar">"#),
            "[--dom=shadow] dev <template foo=\"bar\"> must be preserved verbatim, got: {processed}"
        );
        assert!(
            !processed.contains(r#"shadowrootmode="open""#),
            "[--dom=shadow] framework must NOT inject shadowrootmode when dev already supplied a <template>, got: {processed}"
        );
    }

    #[test]
    fn shadow_preserves_dev_template_with_shadowrootmode_closed() {
        let mut parser = HtmlParser::with_options(DomStrategy::Shadow);
        let processed = parser
            .process_component_template(
                r#"<template shadowrootmode="closed"><div>hi</div></template>"#,
                None,
                None,
            )
            .expect("process failed");
        assert!(
            processed.contains(r#"shadowrootmode="closed""#),
            "[--dom=shadow] framework must respect dev's shadowrootmode=\"closed\" (developer is managing), got: {processed}"
        );
        assert!(
            !processed.contains(r#"shadowrootmode="open""#),
            "[--dom=shadow] framework must not overwrite dev's shadowrootmode with \"open\", got: {processed}"
        );
    }

    #[test]
    fn shadow_preserves_dev_template_with_signal_fragment_attrs() {
        let mut parser = HtmlParser::with_options(DomStrategy::Shadow);
        let processed = parser
            .process_component_template(
                r#"<template foo="{{foo}}"><div>hi</div></template>"#,
                None,
                None,
            )
            .expect("process failed");
        assert!(
            processed.contains(r#"<template foo="{{foo}}">"#),
            "[--dom=shadow] dev <template> with signal-fragment attr must be preserved, got: {processed}"
        );
    }

    #[test]
    fn shadow_adds_template_wrapper_when_dev_omits_it() {
        let mut parser = HtmlParser::with_options(DomStrategy::Shadow);
        let processed = parser
            .process_component_template("<div>hi</div>", None, None)
            .expect("process failed");
        assert!(
            processed.contains(r#"<template shadowrootmode="open""#),
            "[--dom=shadow] framework MUST add <template shadowrootmode=\"open\"> when dev omits a wrapper, got: {processed}"
        );
        assert!(
            processed.contains("<div>hi</div>"),
            "[--dom=shadow] inner content must survive wrapping, got: {processed}"
        );
        assert!(
            processed.contains("</template>"),
            "[--dom=shadow] framework-added wrapper must be closed, got: {processed}"
        );
    }

    #[test]
    fn leading_comments_do_not_change_root_template_detection() {
        const COMMENT: &str = "<!-- Copyright (C) Corporation. All rights reserved. -->";
        let fixtures = [
            (
                format!(
                    "{COMMENT}\n<template shadowrootmode=\"open\">\n  <h1>Hello</h1>\n</template>"
                ),
                "<template shadowrootmode=\"open\">\n  <h1>Hello</h1>\n</template>",
            ),
            (
                format!("{COMMENT}\n<h1>Hello</h1>"),
                "<template shadowrootmode=\"open\"><h1>Hello</h1></template>",
            ),
            (
                format!("{COMMENT}\nHello"),
                "<template shadowrootmode=\"open\">Hello</template>",
            ),
        ];

        for (input, expected) in fixtures {
            let mut parser = HtmlParser::with_options(DomStrategy::Shadow);
            let processed = parser
                .process_component_template(&input, None, None)
                .expect("leading-comment fixture should process");
            assert_eq!(processed, expected);
        }
    }

    #[test]
    fn component_policy_wrapper_uses_selected_dom_strategy_and_is_build_only() {
        let html = r#"<template w-hydrate="lazy"><div>hi</div></template>"#;

        let mut shadow = HtmlParser::with_options(DomStrategy::Shadow);
        let shadow_built = shadow
            .build_component_templates(ComponentTemplateInput {
                tag_name: "my-comp",
                html,
                artifact_html: None,
                css_content: None,
                artifact_needed: true,
            })
            .expect("shadow policy wrapper should compile");
        assert_eq!(
            shadow_built.ssr,
            r#"<template shadowrootmode="open"><div>hi</div></template>"#
        );
        assert!(!shadow_built.artifact().contains("w-hydrate"));

        let mut light = HtmlParser::with_options(DomStrategy::Light);
        let light_built = light
            .build_component_templates(ComponentTemplateInput {
                tag_name: "my-comp",
                html,
                artifact_html: None,
                css_content: None,
                artifact_needed: true,
            })
            .expect("light policy wrapper should compile");
        assert_eq!(light_built.ssr, "<div>hi</div>");
        assert_eq!(light_built.artifact(), "<div>hi</div>");
    }

    // ── CSS-module adoption on dev-authored <template> wrappers ────────
    //
    // Regression guard: under `CssStrategy::Module`, the
    // `shadowrootadoptedstylesheets` attribute is the only wire between
    // the framework-emitted importmap/module-script and the shadow root.
    // When the developer supplies their own `<template>` wrapper (e.g.
    // to attach `@event` handlers), the framework preserves that wrapper
    // and appends the CSS-module adoption attribute when it is missing.

    #[test]
    fn dev_template_module_strategy_appends_adopted_attr_when_missing() {
        let mut parser = HtmlParser::with_options(DomStrategy::Shadow);
        let processed = parser
            .process_component_template(
                r#"<template shadowrootmode="open"><div>hi</div></template>"#,
                None,
                Some("my-comp"),
            )
            .expect("process failed");
        assert!(
            processed.contains(r#"shadowrootmode="open""#),
            "dev-authored shadowrootmode must be preserved, got: {processed}"
        );
        assert!(
            processed.contains(r#"shadowrootadoptedstylesheets="my-comp""#),
            "module CSS should append adopted stylesheets, got: {processed}"
        );
    }

    #[test]
    fn dev_template_module_strategy_ok_when_dev_supplies_adopted_attr() {
        let mut parser = HtmlParser::with_options(DomStrategy::Shadow);
        let processed = parser
            .process_component_template(
                r#"<template shadowrootmode="open" shadowrootadoptedstylesheets="my-comp"><div>hi</div></template>"#,
                None,
                Some("my-comp"),
            )
            .unwrap();
        assert!(
            processed.contains(r#"shadowrootadoptedstylesheets="my-comp""#),
            "dev-supplied adopted-stylesheets attr must be preserved verbatim, got: {processed}"
        );
        assert_eq!(
            processed.matches("shadowrootadoptedstylesheets").count(),
            1,
            "framework must not duplicate dev-supplied adopted-stylesheets attr, got: {processed}"
        );
    }

    #[test]
    fn dev_template_module_strategy_appends_adopted_attr_and_preserves_root_attrs() {
        for dom_strategy in [DomStrategy::Shadow, DomStrategy::Light] {
            let mut parser = HtmlParser::with_options((CssStrategy::Module, dom_strategy));
            let built = match parser.build_component_templates(ComponentTemplateInput {
                tag_name: "my-comp",
                html: r#"<template shadowrootmode="open" @click="{onClick()}">Hello</template>"#,
                artifact_html: None,
                css_content: Some(":host { color: red; }"),
                artifact_needed: true,
            }) {
                Ok(built) => built,
                Err(err) => panic!(
                    "dev-authored <template> should be accepted under {dom_strategy:?} with module CSS, got: {err}"
                ),
            };
            let artifact = built.artifact();

            assert!(
                artifact.contains(r#"shadowrootmode="open""#),
                "dev-authored shadowrootmode must be preserved under {dom_strategy:?}, got: {artifact}"
            );
            assert!(
                artifact.contains(r#"@click="{onClick()}""#),
                "dev-authored root event must be preserved under {dom_strategy:?}, got: {artifact}"
            );
            assert!(
                artifact.contains(r#"shadowrootadoptedstylesheets="my-comp""#),
                "module CSS should append adopted stylesheets under {dom_strategy:?}, got: {artifact}"
            );
            assert_eq!(
                artifact.matches("shadowrootadoptedstylesheets").count(),
                1,
                "module CSS should append adopted stylesheets once under {dom_strategy:?}, got: {artifact}"
            );
            assert!(
                !built.ssr.contains("@click"),
                "SSR template must still strip runtime attrs, got: {}",
                built.ssr
            );
        }
    }

    #[test]
    fn dev_template_module_strategy_ok_with_multi_specifier_attr() {
        // Multi-specifier scenarios: the dev wants to adopt extra sheets
        // alongside their own component's module. Honored verbatim — the
        // framework's only job is to validate that *some*
        // `shadowrootadoptedstylesheets` is present.
        let mut parser = HtmlParser::with_options(DomStrategy::Shadow);
        let processed = parser
            .process_component_template(
                r#"<template shadowrootmode="open" shadowrootadoptedstylesheets="my-comp other-sheet"><div>hi</div></template>"#,
                None,
                Some("my-comp"),
            )
            .unwrap();
        assert!(
            processed.contains(r#"shadowrootadoptedstylesheets="my-comp other-sheet""#),
            "dev's multi-specifier list must be preserved verbatim, got: {processed}"
        );
    }

    #[test]
    fn dev_template_no_adopted_specifier_does_not_validate() {
        // CssStrategy::Link or CssStrategy::Style pass `adopted_specifier=None`.
        // Dev's <template> must be preserved verbatim and the validation
        // must not fire.
        let mut parser = HtmlParser::with_options(DomStrategy::Shadow);
        let processed = parser
            .process_component_template(
                r#"<template shadowrootmode="open"><div>hi</div></template>"#,
                None,
                None,
            )
            .unwrap();
        assert!(
            !processed.contains("shadowrootadoptedstylesheets"),
            "framework must NOT inject adopted-stylesheets attr without CssStrategy::Module signal, got: {processed}"
        );
    }

    // test_signal_with_default_value — SKIPPED
    // The NodeJS `<f-signal value="testSignal">Default Text</f-signal>` feature
    // is not supported in the Rust parser. There is no `f-signal` element
    // handling in HtmlParser and no corresponding fragment type in
    // webui_protocol.

    // test_estimated_buffer_size — SKIPPED
    // The NodeJS `estimatedBufferSize` field does not exist in the Rust
    // WebUIFragmentRecords / WebUIProtocol types. Buffer size estimation is
    // not part of the Rust parser output.

    #[test]
    fn test_body_start_end_injection() {
        // Port of: 'should inject body_start and body_end signals around body content'
        // Verifies body_start appears immediately after <body> and body_end
        // appears immediately before </body> in a full HTML page.
        let html = r#"<html><head><title>Test</title></head><body><div>Content</div><p>More</p></body></html>"#;
        let (fragments, _) = parse_and_get_fragments(html);

        assert_fragments!(
            fragments,
            [
                raw("<html><head>"),
                structural_matcher("head_start"),
                raw("<title>Test</title>"),
                structural_matcher("head_end"),
                raw("</head><body>"),
                structural_matcher("body_start"),
                raw("<div>Content</div><p>More</p>"),
                structural_matcher("body_end"),
                raw("</body></html>"),
            ]
        );
    }

    #[test]
    fn test_body_preserves_static_attributes() {
        // The <body> tag's attributes must be carried through to the rendered
        // HTML, otherwise host frames (e.g. webui-press's `<body data-layout="…">`)
        // lose their styling hooks. Earlier versions of `process_body_with_signals`
        // emitted a bare `<body>` and dropped every attribute on the element.
        let html = r#"<html><head><title>T</title></head><body data-layout="doc" class="page"><p>x</p></body></html>"#;
        let (fragments, _) = parse_and_get_fragments(html);

        assert_fragments!(
            fragments,
            [
                raw("<html><head>"),
                structural_matcher("head_start"),
                raw("<title>T</title>"),
                structural_matcher("head_end"),
                raw(r#"</head><body data-layout="doc" class="page">"#),
                structural_matcher("body_start"),
                raw("<p>x</p>"),
                structural_matcher("body_end"),
                raw("</body></html>"),
            ]
        );
    }

    #[test]
    fn test_body_preserves_dynamic_attributes() {
        // Dynamic attributes on <body> (e.g. `data-layout="{{layout}}"`) must
        // emit attribute fragments rather than being inlined as literal text,
        // so the handler binds them to runtime state at render time.
        let html = r#"<html><head><title>T</title></head><body data-layout="{{layout}}"><p>x</p></body></html>"#;
        let (fragments, _) = parse_and_get_fragments(html);

        assert_fragments!(
            fragments,
            [
                raw("<html><head>"),
                structural_matcher("head_start"),
                raw("<title>T</title>"),
                structural_matcher("head_end"),
                raw("</head><body"),
                attr("data-layout", "layout"),
                raw(">"),
                structural_matcher("body_start"),
                raw("<p>x</p>"),
                structural_matcher("body_end"),
                raw("</body></html>"),
            ]
        );
    }

    #[test]
    fn test_head_start_signal_ordering_with_authored_script() {
        // `render_streaming` preflights on `head_start` before authored async
        // scripts in <head>, mirroring `body_start`. `head_start` must appear
        // immediately after the raw `<head>` opening fragment and before any
        // child content, including an authored `<script>`.
        let html = r#"<html><head><script async src="./analytics.js"></script></head><body></body></html>"#;
        let (fragments, _) = parse_and_get_fragments(html);

        assert_fragments!(
            fragments,
            [
                raw("<html><head>"),
                structural_matcher("head_start"),
                raw(r#"<script async src="./analytics.js"></script>"#),
                structural_matcher("head_end"),
                raw("</head><body>"),
                structural_matcher("body_start"),
                structural_matcher("body_end"),
                raw("</body></html>"),
            ]
        );
    }

    #[test]
    fn test_head_static_attributes_preserve_legacy_opening_bytes() {
        // Compatibility: origin/main emitted a literal `<head>` and ignored
        // authored head attributes. The structural signal must not alter that
        // ordinary protocol/output shape.
        let html = r#"<html><head data-theme="dark" class="app-head"><title>T</title></head><body></body></html>"#;
        let (fragments, _) = parse_and_get_fragments(html);

        assert_fragments!(
            fragments,
            [
                raw("<html><head>"),
                structural_matcher("head_start"),
                raw("<title>T</title>"),
                structural_matcher("head_end"),
                raw("</head><body>"),
                structural_matcher("body_start"),
                structural_matcher("body_end"),
                raw("</body></html>"),
            ]
        );
    }

    #[test]
    fn test_head_bound_attributes_preserve_legacy_opening_bytes() {
        // Bound head attributes were also ignored by origin/main. In
        // particular, do not introduce an attribute fragment before
        // `head_start`, because ordinary rendering must remain byte-identical.
        let html =
            r#"<html><head data-theme="{{theme}}"><title>T</title></head><body></body></html>"#;
        let (fragments, _) = parse_and_get_fragments(html);

        assert_fragments!(
            fragments,
            [
                raw("<html><head>"),
                structural_matcher("head_start"),
                raw("<title>T</title>"),
                structural_matcher("head_end"),
                raw("</head><body>"),
                structural_matcher("body_start"),
                structural_matcher("body_end"),
                raw("</body></html>"),
            ]
        );
    }

    #[test]
    fn native_head_and_body_are_ascii_case_insensitive() {
        let html = r#"<html><HEAD data-theme="ignored"><title>T</title></HEAD><BODY data-layout="{{layout}}"><p>x</p></BODY></html>"#;
        let (fragments, _) = parse_and_get_fragments(html);

        assert_fragments!(
            fragments,
            [
                raw(r#"<html><HEAD data-theme="ignored">"#),
                structural_matcher("head_start"),
                raw("<title>T</title>"),
                structural_matcher("head_end"),
                raw("</HEAD><BODY"),
                attr("data-layout", "layout"),
                raw(">"),
                structural_matcher("body_start"),
                raw("<p>x</p>"),
                structural_matcher("body_end"),
                raw("</BODY></html>"),
            ]
        );
    }

    // ── `<boundary>` / `<webui-hydrate>` tests (Progressive Streaming
    // Hydration) ───────────────────────────────────────────────────────────

    #[test]
    fn boundary_emits_no_wrapper_and_brackets_children_with_signals() {
        let html = r#"<body><div><boundary name="counter-ready"><my-counter></my-counter></boundary></div></body>"#;
        let (fragments, _) = parse_and_get_fragments(html);

        // No `<boundary>`/`</boundary>` markup is emitted at
        // all — only the raw boundary_start/boundary_end signals bracket
        // the untouched children.
        assert_fragments!(
            fragments,
            [
                raw("<body>"),
                structural_matcher("body_start"),
                raw("<div>"),
                structural_matcher("boundary_start:0"),
                raw("<my-counter></my-counter>"),
                structural_matcher("boundary_end:0"),
                raw("</div>"),
                structural_matcher("body_end"),
                raw("</body>"),
            ]
        );
    }

    #[test]
    fn boundary_sequence_increments_in_document_order_and_resets_per_entry() {
        let html = concat!(
            "<body>",
            r#"<boundary name="a"><p>1</p></boundary>"#,
            r#"<boundary name="b"><p>2</p></boundary>"#,
            "</body>",
        );
        let (fragments, _) = parse_and_get_fragments(html);

        assert_fragments!(
            fragments,
            [
                raw("<body>"),
                structural_matcher("body_start"),
                structural_matcher("boundary_start:0"),
                raw("<p>1</p>"),
                structural_matcher("boundary_end:0"),
                structural_matcher("boundary_start:1"),
                raw("<p>2</p>"),
                structural_matcher("boundary_end:1"),
                structural_matcher("body_end"),
                raw("</body>"),
            ]
        );

        // A separate top-level entry parsed on the same `HtmlParser` starts
        // its own boundary sequence back at 0 and does not reject "a" as a
        // duplicate name — sequence/name state is per top-level entry.
        let mut parser = HtmlParser::new();
        parser
            .parse(
                "first.html",
                r#"<body><boundary name="a"><p>1</p></boundary></body>"#,
            )
            .expect("first entry parses");
        parser
            .parse(
                "second.html",
                r#"<body><boundary name="a"><p>2</p></boundary></body>"#,
            )
            .expect("second entry reuses the name and sequence 0");
        let records = parser.into_fragment_records();
        assert_stream!(
            records,
            "second.html",
            [
                raw("<body>"),
                structural_matcher("body_start"),
                structural_matcher("boundary_start:0"),
                raw("<p>2</p>"),
                structural_matcher("boundary_end:0"),
                structural_matcher("body_end"),
                raw("</body>"),
            ]
        );
    }

    #[test]
    fn boundary_missing_name_errors() {
        let mut parser = HtmlParser::new();
        let err = parser
            .parse("index.html", "<body><boundary><p>x</p></boundary></body>")
            .expect_err("a boundary without a name must error");
        let ParserError::Template(diag) = err else {
            panic!("expected ParserError::Template, got {err:?}");
        };
        assert_eq!(diag.error_code(), Some(codes::MISSING_BOUNDARY_NAME));
    }

    #[test]
    fn boundary_dynamic_name_errors() {
        let mut parser = HtmlParser::new();
        let err = parser
            .parse(
                "index.html",
                r#"<body><boundary name="{{sectionName}}"><p>x</p></boundary></body>"#,
            )
            .expect_err("a dynamic boundary name must error");
        let ParserError::Template(diag) = err else {
            panic!("expected ParserError::Template, got {err:?}");
        };
        assert_eq!(diag.error_code(), Some(codes::INVALID_BOUNDARY_NAME));
    }

    /// Boundary names stay free-form and are exposed separately for the
    /// protocol's cold name-to-handle table. Render fragments still carry only
    /// integer IDs, so response bytes never repeat author names.
    #[test]
    fn boundary_names_are_free_form_ordered_and_absent_from_fragments() {
        let mut parser = HtmlParser::new();
        let html = concat!(
            "<body>",
            r#"<boundary name="above the fold"><p>1</p></boundary>"#,
            r#"<boundary name="feed/items #2"><p>2</p></boundary>"#,
            r#"<boundary name="ダッシュボード"><p>3</p></boundary>"#,
            "</body>",
        );
        parser
            .parse("index.html", html)
            .expect("free-form boundary names must be accepted");
        assert_eq!(parser.boundary_count(), 3);
        assert_eq!(
            parser.boundary_names(),
            ["above the fold", "feed/items #2", "ダッシュボード"]
        );

        let records = parser.into_fragment_records();
        let fragments = &records
            .get("index.html")
            .expect("index.html fragment record")
            .fragments;

        let mut starts = Vec::new();
        for fragment in fragments {
            let Some(Fragment::Signal(signal)) = fragment.fragment.as_ref() else {
                continue;
            };
            let value = signal.value.as_str();
            assert!(
                !value.contains("above the fold")
                    && !value.contains("feed/items")
                    && !value.contains("ダッシュボード"),
                "boundary names must never reach a fragment: {value}"
            );
            if let Some(sequence) = value
                .strip_prefix(STRUCTURAL_SIGNAL_PREFIX)
                .and_then(|signal| signal.strip_prefix("boundary_start:"))
            {
                starts.push(sequence.to_string());
            }
        }
        assert_eq!(
            starts,
            ["0", "1", "2"],
            "only integer sequences are emitted"
        );
    }

    #[test]
    fn module_entry_srcs_records_only_preloadable_critical_entries() {
        let mut parser = HtmlParser::new();
        let html = concat!(
            "<head>",
            // Recorded: a static, module-typed, non-boundary entry.
            r#"<script type="module" async src="/index.js"></script>"#,
            // Recorded: mixed case on the tag, and a padded/mixed-case `type`
            // value, still identify a module. (Attribute *names* are matched
            // case-sensitively here, as everywhere else in this parser.)
            r#"<SCRIPT type=" Module " src="/late.js"></SCRIPT>"#,
            // Skipped: classic scripts have no ES module graph to preload.
            r#"<script src="/legacy.js"></script>"#,
            // Skipped: no `src` means nothing to resolve.
            r#"<script type="module">import "./x.js";</script>"#,
            // Skipped: a per-request binding matches no build artifact.
            r#"<script type="module" src="/{{bundle}}.js"></script>"#,
            "</head>",
            "<body>",
            // Skipped: an island loader is deferred on purpose.
            r#"<boundary name="weather">"#,
            r#"<script type="module" async src="/weather-panel.js"></script>"#,
            "</boundary>",
            // Recorded once: a bottom-of-body entry is still critical, and a
            // repeat of an already-seen src must not duplicate a hint.
            r#"<script type="module" src="/index.js"></script>"#,
            "</body>",
        );
        parser.parse("index.html", html).expect("parse");

        assert_eq!(
            parser.module_entry_srcs(),
            ["/index.js", "/late.js"],
            "only static module entries outside boundaries are preloadable"
        );
    }

    #[test]
    fn module_entry_srcs_reset_between_top_level_parses() {
        let mut parser = HtmlParser::new();
        parser
            .parse(
                "a.html",
                r#"<head><script type="module" src="/a.js"></script></head>"#,
            )
            .expect("parse a");
        assert_eq!(parser.module_entry_srcs(), ["/a.js"]);

        parser
            .parse(
                "b.html",
                r#"<head><script type="module" src="/b.js"></script></head>"#,
            )
            .expect("parse b");
        assert_eq!(
            parser.module_entry_srcs(),
            ["/b.js"],
            "each entry owns its own critical modules"
        );
    }

    #[test]
    fn boundary_duplicate_name_errors() {
        let mut parser = HtmlParser::new();
        let html = concat!(
            "<body>",
            r#"<boundary name="dup"><p>1</p></boundary>"#,
            r#"<boundary name="dup"><p>2</p></boundary>"#,
            "</body>",
        );
        let err = parser
            .parse("index.html", html)
            .expect_err("a duplicate boundary name must error");
        let ParserError::Template(diag) = err else {
            panic!("expected ParserError::Template, got {err:?}");
        };
        assert_eq!(diag.error_code(), Some(codes::DUPLICATE_BOUNDARY_NAME));
    }

    #[test]
    fn boundary_nested_boundary_errors() {
        let mut parser = HtmlParser::new();
        let html = r#"<body><boundary name="outer"><boundary name="inner"><p>x</p></boundary></boundary></body>"#;
        let err = parser
            .parse("index.html", html)
            .expect_err("a nested boundary must error");
        let ParserError::Template(diag) = err else {
            panic!("expected ParserError::Template, got {err:?}");
        };
        assert_eq!(diag.error_code(), Some(codes::NESTED_BOUNDARY));
    }

    #[test]
    fn boundary_inside_if_errors() {
        let mut parser = HtmlParser::new();
        let html =
            r#"<body><if condition="ready"><boundary name="x"><p>x</p></boundary></if></body>"#;
        let err = parser
            .parse("index.html", html)
            .expect_err("a boundary inside <if> must error");
        let ParserError::Template(diag) = err else {
            panic!("expected ParserError::Template, got {err:?}");
        };
        assert_eq!(diag.error_code(), Some(codes::BOUNDARY_CROSSES_SCOPE));
        assert!(diag.to_string().contains("<if>"), "{diag}");
    }

    #[test]
    fn boundary_inside_for_errors() {
        let mut parser = HtmlParser::new();
        let html = r#"<body><for each="item in items"><boundary name="x"><p>x</p></boundary></for></body>"#;
        let err = parser
            .parse("index.html", html)
            .expect_err("a boundary inside <for> must error");
        let ParserError::Template(diag) = err else {
            panic!("expected ParserError::Template, got {err:?}");
        };
        assert_eq!(diag.error_code(), Some(codes::BOUNDARY_CROSSES_SCOPE));
        assert!(diag.to_string().contains("<for>"), "{diag}");
    }

    #[test]
    fn boundary_inside_reusable_component_template_errors() {
        // The boundary lives inside the component's own `.html` template, not
        // the entry template — disallowed even though the entry's usage of
        // `<my-widget>` looks unremarkable.
        let mut parser = HtmlParser::with_options(DomStrategy::Light);
        parser
            .component_registry
            .register_component(ComponentRegistration::new(
                "my-widget",
                r#"<boundary name="x"><p>x</p></boundary>"#,
                None,
                true,
            ))
            .expect("register");

        let err = parser
            .parse("index.html", "<body><my-widget></my-widget></body>")
            .expect_err("a boundary inside a component template must error");
        let ParserError::Template(diag) = err else {
            panic!("expected ParserError::Template, got {err:?}");
        };
        assert_eq!(diag.error_code(), Some(codes::BOUNDARY_CROSSES_SCOPE));
    }

    #[test]
    fn boundary_inside_component_host_content_errors() {
        let mut parser = HtmlParser::with_options(DomStrategy::Light);
        parser
            .component_registry
            .register_component(ComponentRegistration::new(
                "my-widget",
                "<slot></slot>",
                None,
                true,
            ))
            .expect("register");

        let err = parser
            .parse(
                "index.html",
                r#"<body><my-widget><boundary name="x"><p>x</p></boundary></my-widget></body>"#,
            )
            .expect_err("a boundary inside component host content must error");
        let ParserError::Template(diag) = err else {
            panic!("expected ParserError::Template, got {err:?}");
        };
        assert_eq!(diag.error_code(), Some(codes::BOUNDARY_CROSSES_SCOPE));
        assert!(
            diag.to_string().contains("component host content"),
            "{diag}"
        );
    }

    #[test]
    fn boundary_inside_raw_or_inert_html_context_errors() {
        for (element, expected_scope) in [
            ("textarea", "<textarea> text content"),
            ("title", "<title> text content"),
            ("script", "<script> raw-text content"),
            ("xmp", "<xmp> raw-text content"),
            ("iframe", "<iframe> raw-text content"),
            ("noembed", "<noembed> raw-text content"),
            ("noframes", "<noframes> raw-text content"),
            ("noscript", "<noscript> inert content"),
            ("plaintext", "<plaintext> text content"),
            ("template", "<template> inert content"),
        ] {
            let mut parser = HtmlParser::new();
            let html = format!(
                r#"<body><{element}><boundary name="x"><p>x</p></boundary></{element}></body>"#
            );
            let err = parser
                .parse("index.html", &html)
                .expect_err("a boundary in raw or inert content must error");
            let ParserError::Template(diag) = err else {
                panic!("expected ParserError::Template, got {err:?}");
            };
            assert_eq!(
                diag.error_code(),
                Some(codes::BOUNDARY_CROSSES_SCOPE),
                "unexpected code for <{element}>"
            );
            assert!(
                diag.to_string().contains(expected_scope),
                "unexpected scope for <{element}>: {diag}"
            );
        }
    }

    #[test]
    fn boundary_inside_route_element_errors() {
        let mut parser = HtmlParser::new();
        // Directly inside <route> children.
        let err = parser
            .parse(
                "index.html",
                r#"<body><route path="/" component="home"><boundary name="x"><p>x</p></boundary></route></body>"#,
            )
            .expect_err("a boundary directly inside <route> must error");
        let ParserError::Template(diag) = err else {
            panic!("expected ParserError::Template, got {err:?}");
        };
        assert_eq!(diag.error_code(), Some(codes::BOUNDARY_CROSSES_SCOPE));

        // Nested deeper inside <route> ignored markup.
        let mut parser = HtmlParser::new();
        let err = parser
            .parse(
                "index.html",
                r#"<body><route path="/" component="home"><div><boundary name="x"><p>x</p></boundary></div></route></body>"#,
            )
            .expect_err("a boundary nested inside <route> markup must error");
        let ParserError::Template(diag) = err else {
            panic!("expected ParserError::Template, got {err:?}");
        };
        assert_eq!(diag.error_code(), Some(codes::BOUNDARY_CROSSES_SCOPE));
    }

    #[test]
    fn boundary_must_be_inside_an_open_body() {
        for html in [
            r#"<boundary name="before"><p>x</p></boundary><body></body>"#,
            r#"<body></body><boundary name="after"><p>x</p></boundary>"#,
        ] {
            let mut parser = HtmlParser::new();
            let err = parser
                .parse("index.html", html)
                .expect_err("a boundary outside body must error");
            let ParserError::Template(diag) = err else {
                panic!("expected ParserError::Template, got {err:?}");
            };
            assert_eq!(diag.error_code(), Some(codes::BOUNDARY_OUTSIDE_BODY));
            assert_eq!(
                diag.help_text(),
                Some("move the <boundary> between the opening <body> and its matching </body>")
            );
        }

        let mut parser = HtmlParser::new();
        parser
            .parse(
                "index.html",
                r#"<BODY><boundary name="inside"></boundary></BODY>"#,
            )
            .expect("mixed-case native body keeps the boundary inside an open body");
    }

    #[test]
    fn boundary_inside_foster_parenting_context_errors() {
        // Each of these insertion modes moves an unknown element (the
        // generated <webui-hydrate> sentinel) out of the subtree, which would
        // split it from its payload script and halt hydration at runtime.
        for html in [
            r#"<body><table><boundary name="t"><p>x</p></boundary></table></body>"#,
            r#"<body><table><tbody><boundary name="t"><p>x</p></boundary></tbody></table></body>"#,
            r#"<body><table><thead><boundary name="t"><p>x</p></boundary></thead></table></body>"#,
            r#"<body><table><tfoot><boundary name="t"><p>x</p></boundary></tfoot></table></body>"#,
            r#"<body><table><tbody><tr><boundary name="t"><p>x</p></boundary></tr></tbody></table></body>"#,
            r#"<body><table><colgroup><boundary name="t"><p>x</p></boundary></colgroup></table></body>"#,
            r#"<body><select><boundary name="t"><option>x</option></boundary></select></body>"#,
            r#"<body><select><optgroup><boundary name="t"><option>x</option></boundary></optgroup></select></body>"#,
            // Mixed case must be rejected identically: HTML tag names are
            // case-insensitive, so the browser foster-parents these too.
            r#"<body><TABLE><TBODY><boundary name="t"><p>x</p></boundary></TBODY></TABLE></body>"#,
        ] {
            let mut parser = HtmlParser::new();
            let err = parser
                .parse("index.html", html)
                .expect_err("a boundary in a foster-parenting context must error");
            let ParserError::Template(diag) = err else {
                panic!("expected ParserError::Template, got {err:?}");
            };
            assert_eq!(
                diag.error_code(),
                Some(codes::BOUNDARY_IN_FOSTER_CONTEXT),
                "unexpected code for {html}"
            );
            assert!(
                diag.help_text()
                    .is_some_and(|help| help.contains("<td>, <th>, or <caption>")),
                "help should suggest a safe placement for {html}"
            );
        }
    }

    #[test]
    fn boundary_is_allowed_where_insertion_mode_returns_to_in_body() {
        // <td>/<th>/<caption> switch back to "in body" rules, so an unknown
        // element is *not* foster-parented and the boundary is safe even
        // though <table>/<tr> ancestors are hostile.
        for html in [
            r#"<body><table><tbody><tr><td><boundary name="cell"><p>x</p></boundary></td></tr></tbody></table></body>"#,
            r#"<body><table><tbody><tr><th><boundary name="head"><p>x</p></boundary></th></tr></tbody></table></body>"#,
            r#"<body><table><caption><boundary name="cap"><p>x</p></boundary></caption></table></body>"#,
            // Wrapping the whole table is the other documented fix.
            r#"<body><boundary name="whole"><table><tbody><tr><td>x</td></tr></tbody></table></boundary></body>"#,
            // A <td> barrier resets the depth, an inner table raises it again,
            // and closing that inner table must restore the barrier's zero —
            // not the outer table's non-zero depth.
            concat!(
                "<body><table><tbody><tr><td>",
                "<table><tbody><tr><td>inner</td></tr></tbody></table>",
                r#"<boundary name="after-inner"><p>x</p></boundary>"#,
                "</td></tr></tbody></table></body>",
            ),
            // Void and self-closing elements must not leak restore ops.
            r#"<body><table><tbody><tr><td><img src="a"><br><input></td></tr></tbody></table><boundary name="v"><p>x</p></boundary></body>"#,
        ] {
            let mut parser = HtmlParser::new();
            parser
                .parse("index.html", html)
                .unwrap_or_else(|err| panic!("{html} should parse, got {err:?}"));
        }
    }

    #[test]
    fn foster_context_depth_is_restored_after_leaving_a_table() {
        // Regression guard for the save/restore op: a boundary *after* a table
        // must not inherit the table's hostile depth.
        let mut parser = HtmlParser::new();
        parser
            .parse(
                "index.html",
                concat!(
                    "<body>",
                    "<table><tbody><tr><td>x</td></tr></tbody></table>",
                    r#"<boundary name="after-table"><p>y</p></boundary>"#,
                    "</body>",
                ),
            )
            .expect("a boundary after a closed table must be accepted");
    }

    #[test]
    fn authored_raw_bindings_cannot_forge_structural_signals() {
        let html = concat!(
            "<html><head></head><body>",
            "{{{head_start}}}{{{head_end}}}{{{body_start}}}{{{body_end}}}",
            "{{{boundary_start:0}}}{{{boundary_end:0}}}",
            "{{{streaming_root:forged}}}",
            "</body></html>",
        );
        let (fragments, _) = parse_and_get_fragments(html);
        let raw_signals: Vec<&str> = fragments
            .iter()
            .filter_map(|fragment| match fragment.fragment.as_ref() {
                Some(Fragment::Signal(signal)) if signal.raw => Some(signal.value.as_str()),
                _ => None,
            })
            .collect();

        assert_eq!(
            raw_signals,
            [
                "}}}webui:head_start",
                "}}}webui:head_end",
                "}}}webui:body_start",
                "head_start",
                "head_end",
                "body_start",
                "body_end",
                "boundary_start:0",
                "boundary_end:0",
                "streaming_root:forged",
                "}}}webui:body_end",
            ]
        );
    }

    #[test]
    fn authored_webui_hydrate_errors_at_top_level() {
        for tag in ["webui-hydrate", "WEBUI-HYDRATE", "WebUi-HyDrAtE"] {
            let mut parser = HtmlParser::new();
            let html = format!("<div><{tag}></{tag}></div>");
            let err = parser
                .parse("index.html", &html)
                .expect_err("an authored <webui-hydrate> must error in any ASCII casing");
            let ParserError::Template(diag) = err else {
                panic!("expected ParserError::Template, got {err:?}");
            };
            assert_eq!(diag.error_code(), Some(codes::AUTHORED_WEBUI_HYDRATE));
        }
    }

    #[test]
    fn authored_webui_hydrate_errors_inside_route() {
        for tag in ["webui-hydrate", "WEBUI-HYDRATE", "WebUi-HyDrAtE"] {
            let mut parser = HtmlParser::new();
            let html = format!(r#"<route path="/" component="home"><{tag}></{tag}></route>"#);
            let err = parser
                .parse("index.html", &html)
                .expect_err("an authored <webui-hydrate> inside <route> must error");
            let ParserError::Template(diag) = err else {
                panic!("expected ParserError::Template, got {err:?}");
            };
            assert_eq!(diag.error_code(), Some(codes::AUTHORED_WEBUI_HYDRATE));
        }
    }

    #[test]
    fn boundary_directive_remains_case_sensitive() {
        let (fragments, _) =
            parse_and_get_fragments(r#"<WEBUI-BOUNDARY name="ordinary"><p>x</p></WEBUI-BOUNDARY>"#);

        assert_fragments!(
            fragments,
            [raw(
                r#"<WEBUI-BOUNDARY name="ordinary"><p>x</p></WEBUI-BOUNDARY>"#
            ),]
        );
    }

    #[test]
    fn templates_without_webui_boundary_are_unaffected() {
        // Regression: ordinary templates with no `<boundary>` directive
        // must parse exactly as before — no boundary signals, no behavior
        // change for the non-streaming path.
        let html = r#"<div><my-counter count="{{count}}"></my-counter></div>"#;
        let (fragments, _) = parse_and_get_fragments(html);

        assert_fragments!(
            fragments,
            [
                raw("<div><my-counter"),
                attr("count", "count"),
                raw("></my-counter></div>"),
            ]
        );
    }

    #[test]
    fn test_fail_with_invalid_markup() {
        // Port of: 'should fail with invalid markup'
        let mut parser = HtmlParser::new();
        let result = parser.parse("index.html", "<div><span></div></span>");

        assert!(matches!(result, Err(ParserError::Template(diag)) if
            diag.to_string().contains("unclosed <span> tag")
        ));
    }

    #[test]
    fn test_complex_raw_text_page() {
        // Port of: 'should process a complex raw text page with DOCTYPE,
        // meta tags, styles, and scripts'
        let html = concat!(
            "<!DOCTYPE html>",
            "<html lang=\"en\">",
            "<head>",
            "<meta charset=\"utf-8\">",
            "<meta name=\"viewport\" content=\"width=device-width, initial-scale=1.0\">",
            "<title>Complex Page</title>",
            "<style>body { margin: 0; padding: 0; } h1 { color: blue; }</style>",
            "<link rel=\"stylesheet\" href=\"styles.css\">",
            "</head>",
            "<body>",
            "<h1>Hello World</h1>",
            "<script type=\"module\" src=\"./app.js\"></script>",
            "</body>",
            "</html>",
        );
        let (fragments, _) = parse_and_get_fragments(html);

        // Should have: raw(DOCTYPE+<head>), head_start, raw(head content),
        // head_end, raw(</head><body>), body_start, raw(body content),
        // body_end, raw(</body></html>)
        assert!(
            fragments.len() >= 9,
            "Expected at least 9 fragments, got {}",
            fragments.len()
        );

        // First fragment: DOCTYPE through the opening <head> tag
        assert!(
            matches!(fragments[0].fragment.as_ref(), Some(Fragment::Raw(raw)) if
                raw.value.contains("<!DOCTYPE html>") && raw.value.ends_with("<head>")),
            "First fragment should contain DOCTYPE and end with <head>, got: {:?}",
            fragments[0]
        );

        // head_start signal
        assert!(
            matches!(fragments[1].fragment.as_ref(), Some(Fragment::Signal(s)) if
                s.raw && s.value.strip_prefix(STRUCTURAL_SIGNAL_PREFIX) == Some("head_start")),
            "Second fragment should be head_start signal"
        );

        // Head content (meta, title, style, link)
        assert!(
            matches!(fragments[2].fragment.as_ref(), Some(Fragment::Raw(raw)) if
                raw.value.contains("<meta charset=\"utf-8\">") &&
                raw.value.contains("<meta name=\"viewport\"") &&
                raw.value.contains("<title>Complex Page</title>") &&
                raw.value.contains("<style>") &&
                raw.value.contains("body { margin: 0; padding: 0; }")),
            "Third fragment should contain all head content, got: {:?}",
            fragments[2]
        );

        // head_end signal
        assert!(
            matches!(fragments[3].fragment.as_ref(), Some(Fragment::Signal(s)) if
                s.raw && s.value.strip_prefix(STRUCTURAL_SIGNAL_PREFIX) == Some("head_end")),
            "Fourth fragment should be head_end signal"
        );

        // </head><body>
        assert!(
            matches!(fragments[4].fragment.as_ref(), Some(Fragment::Raw(raw)) if
                raw.value.contains("</head>") && raw.value.ends_with("<body>")),
            "Fifth fragment should contain </head><body>"
        );

        // body_start signal
        assert!(
            matches!(fragments[5].fragment.as_ref(), Some(Fragment::Signal(s)) if
                s.raw && s.value.strip_prefix(STRUCTURAL_SIGNAL_PREFIX) == Some("body_start")),
            "Sixth fragment should be body_start signal"
        );

        // Body content (h1 and script)
        assert!(
            matches!(fragments[6].fragment.as_ref(), Some(Fragment::Raw(raw)) if
                raw.value.contains("<h1>Hello World</h1>") &&
                raw.value.contains("<script")),
            "Seventh fragment should contain body content"
        );

        // body_end signal
        assert!(
            matches!(fragments[7].fragment.as_ref(), Some(Fragment::Signal(s)) if
                s.raw && s.value.strip_prefix(STRUCTURAL_SIGNAL_PREFIX) == Some("body_end")),
            "Eighth fragment should be body_end signal"
        );

        // Closing tags
        assert!(
            matches!(fragments[8].fragment.as_ref(), Some(Fragment::Raw(raw)) if
                raw.value.contains("</body>") && raw.value.contains("</html>")),
            "Ninth fragment should contain closing tags"
        );
    }

    // --- Binding count tests (with mock plugin) ---

    /// Mock plugin that records the binding attribute count for each element.
    struct BindingCountPlugin {
        counts: Vec<u32>,
    }

    impl BindingCountPlugin {
        fn new() -> Self {
            Self { counts: Vec::new() }
        }
    }

    impl crate::plugin::ParserPlugin for BindingCountPlugin {
        fn register_component_template(
            &mut self,
            _tag_name: &str,
            _component: &Component,
            _processed_template: &str,
        ) -> Result<()> {
            Ok(())
        }

        fn classify_attribute(&mut self, attr_name: &str) -> AttributeAction {
            if attr_name.starts_with('@') || attr_name == "f-ref" {
                AttributeAction::SkipAndCountBinding
            } else {
                AttributeAction::Keep
            }
        }

        fn finish_element(&mut self, binding_attribute_count: u32) -> Option<Vec<u8>> {
            self.counts.push(binding_attribute_count);
            if binding_attribute_count > 0 {
                Some(binding_attribute_count.to_le_bytes().to_vec())
            } else {
                None
            }
        }
    }

    struct TemplateCapturePlugin {
        template: Option<String>,
    }

    impl TemplateCapturePlugin {
        fn new() -> Self {
            Self { template: None }
        }
    }

    impl crate::plugin::ParserPlugin for TemplateCapturePlugin {
        fn register_component_template(
            &mut self,
            _tag_name: &str,
            _component: &Component,
            processed_template: &str,
        ) -> Result<()> {
            self.template = Some(processed_template.to_string());
            Ok(())
        }

        fn classify_attribute(&mut self, attr_name: &str) -> AttributeAction {
            if attr_name.starts_with('@') || attr_name == "f-ref" {
                AttributeAction::SkipAndCountBinding
            } else {
                AttributeAction::Keep
            }
        }

        fn finish_element(&mut self, _binding_attribute_count: u32) -> Option<Vec<u8>> {
            None
        }

        fn into_artifacts(self: Box<Self>) -> Result<ParserPluginArtifacts> {
            match self.template {
                Some(template) => Ok(ParserPluginArtifacts::ComponentTemplates(vec![
                    crate::plugin::ComponentTemplateArtifact::template(
                        "todo-app".to_string(),
                        template,
                    ),
                ])),
                None => Ok(ParserPluginArtifacts::None),
            }
        }
    }

    #[test]
    fn parser_passes_user_template_root_attrs_to_plugin_without_css_modules() {
        let mut parser = HtmlParser::with_plugin_options(
            Box::new(TemplateCapturePlugin::new()),
            CssStrategy::Link,
        );
        parser
            .component_registry_mut()
            .register_component(ComponentRegistration::new(
                "todo-app",
                r#"<template shadowrootmode="open" @toggle-item="{onToggleItem($e)}" @delete-item="{onDeleteItem($e)}" f-ref="{root}"><div>items</div></template>"#,
                Some(":host { display: block; }"),
             true,))
            .expect("register todo-app");

        parser
            .parse("index.html", "<todo-app></todo-app>")
            .expect("parse failed");

        let artifacts = parser.take_plugin_artifacts().expect("artifacts");
        let ParserPluginArtifacts::ComponentTemplates(templates) = artifacts else {
            panic!("expected captured component template");
        };
        let template = &templates[0].template;

        assert!(
            template.contains(r#"shadowrootmode="open""#),
            "parser must pass dev-authored root template attrs to plugins, got: {template}"
        );
        assert!(
            template.contains(r#"@toggle-item="{onToggleItem($e)}""#),
            "parser must pass dev-authored root event attrs to plugins, got: {template}"
        );
        assert!(
            template.contains(r#"@delete-item="{onDeleteItem($e)}""#),
            "parser must pass dev-authored root event attrs to plugins, got: {template}"
        );
        assert!(
            template.contains(r#"f-ref="{root}""#),
            "parser must pass dev-authored root FAST attrs to plugins, got: {template}"
        );
        assert!(
            !template.contains("shadowrootadoptedstylesheets"),
            "link CSS must not add CSS module adoption attrs, got: {template}"
        );
    }

    #[test]
    fn test_component_static_attrs_not_counted_as_bindings() {
        let mut parser = HtmlParser::with_plugin(Box::new(BindingCountPlugin::new()));
        parser
            .component_registry
            .register_component(ComponentRegistration::new(
                "my-btn",
                "<button><slot></slot></button>",
                None,
                true,
            ))
            .expect("register");

        // All attributes are static — binding count should be 0
        let html = r#"<my-btn class="primary" title="Click me">Go</my-btn>"#;
        let result = parser.parse("index.html", html);
        assert!(result.is_ok());

        let records = parser.into_fragment_records();

        // No Plugin fragment should appear (binding count = 0)
        let frags = &records["index.html"].fragments;
        let plugin_count = frags
            .iter()
            .filter(|f| matches!(f.fragment.as_ref(), Some(Fragment::Plugin(_))))
            .count();
        assert_eq!(
            plugin_count, 0,
            "Static-only component attributes should not emit a Plugin fragment"
        );
    }

    #[test]
    fn test_component_dynamic_attr_counted_as_binding() {
        let mut parser = HtmlParser::with_plugin(Box::new(BindingCountPlugin::new()));
        parser
            .component_registry
            .register_component(ComponentRegistration::new(
                "my-btn",
                "<button><slot></slot></button>",
                None,
                true,
            ))
            .expect("register");

        // One dynamic attribute ({{...}}) — binding count should be 1
        let html = r#"<my-btn appearance="{{style}}">Go</my-btn>"#;
        let result = parser.parse("index.html", html);
        assert!(result.is_ok());

        let records = parser.into_fragment_records();
        let frags = &records["index.html"].fragments;

        // Exactly one Plugin fragment with count = 1
        let plugin_frags: Vec<_> = frags
            .iter()
            .filter_map(|f| match f.fragment.as_ref() {
                Some(Fragment::Plugin(p)) => Some(&p.data),
                _ => None,
            })
            .collect();
        assert_eq!(plugin_frags.len(), 1, "Expected 1 Plugin fragment");
        let count = u32::from_le_bytes([
            plugin_frags[0][0],
            plugin_frags[0][1],
            plugin_frags[0][2],
            plugin_frags[0][3],
        ]);
        assert_eq!(count, 1, "Binding attribute count should be 1");
    }

    #[test]
    fn test_component_mixed_static_and_dynamic_attrs_binding_count() {
        let mut parser = HtmlParser::with_plugin(Box::new(BindingCountPlugin::new()));
        parser
            .component_registry
            .register_component(ComponentRegistration::new(
                "my-btn",
                "<button><slot></slot></button>",
                None,
                true,
            ))
            .expect("register");

        // 2 static, 1 dynamic, 1 skipped-with-plugin (@click) — only dynamic + skipped counted
        let html = r#"<my-btn class="primary" title="Submit" appearance="{{look}}" @click="{go}">Go</my-btn>"#;
        let result = parser.parse("index.html", html);
        assert!(result.is_ok());

        let records = parser.into_fragment_records();
        let frags = &records["index.html"].fragments;

        let plugin_frags: Vec<_> = frags
            .iter()
            .filter_map(|f| match f.fragment.as_ref() {
                Some(Fragment::Plugin(p)) => Some(&p.data),
                _ => None,
            })
            .collect();
        assert_eq!(plugin_frags.len(), 1);
        let count = u32::from_le_bytes([
            plugin_frags[0][0],
            plugin_frags[0][1],
            plugin_frags[0][2],
            plugin_frags[0][3],
        ]);
        // 1 dynamic (appearance) + 1 skipped-but-counted (@click) = 2
        assert_eq!(
            count, 2,
            "Binding count should include dynamic + plugin-skipped attrs, not static"
        );
    }

    #[test]
    fn test_component_only_skipped_plugin_attrs_counted() {
        let mut parser = HtmlParser::with_plugin(Box::new(BindingCountPlugin::new()));
        parser
            .component_registry
            .register_component(ComponentRegistration::new(
                "my-btn",
                "<button><slot></slot></button>",
                None,
                true,
            ))
            .expect("register");

        // Only plugin-skipped attrs (@click, f-ref) plus static — only skipped counted
        let html = r#"<my-btn title="Hello" @click="{go}" f-ref="{btn}">Go</my-btn>"#;
        let result = parser.parse("index.html", html);
        assert!(result.is_ok());

        let records = parser.into_fragment_records();
        let frags = &records["index.html"].fragments;

        let plugin_frags: Vec<_> = frags
            .iter()
            .filter_map(|f| match f.fragment.as_ref() {
                Some(Fragment::Plugin(p)) => Some(&p.data),
                _ => None,
            })
            .collect();
        assert_eq!(plugin_frags.len(), 1);
        let count = u32::from_le_bytes([
            plugin_frags[0][0],
            plugin_frags[0][1],
            plugin_frags[0][2],
            plugin_frags[0][3],
        ]);
        // @click + f-ref = 2, title is static = not counted
        assert_eq!(
            count, 2,
            "Only plugin-skipped attrs should be counted, not static title"
        );
    }

    // ── Comment stripping tests ──────────────────────────────────────

    #[test]
    fn test_comment_handlebars_signal_is_stripped() {
        let mut parser = HtmlParser::new();
        let html = "<!--{{tokens}}-->";
        parser.parse("test.html", html).expect("parse failed");
        let records = parser.into_fragment_records();

        assert_stream!(records, "test.html", []);
    }

    #[test]
    fn test_comment_triple_brace_raw_signal_is_stripped() {
        let mut parser = HtmlParser::new();
        let html = "<!--{{{tokens}}}-->";
        parser.parse("test.html", html).expect("parse failed");
        let records = parser.into_fragment_records();

        assert_stream!(records, "test.html", []);
    }

    #[test]
    fn test_comment_regular_stripped() {
        let mut parser = HtmlParser::new();
        let html = "<!-- regular comment -->";
        parser.parse("test.html", html).expect("parse failed");
        let records = parser.into_fragment_records();

        assert_stream!(records, "test.html", []);
    }

    #[test]
    fn test_comment_dotted_signal_is_stripped() {
        let mut parser = HtmlParser::new();
        let html = "<!--{{tokens.light}}-->";
        parser.parse("test.html", html).expect("parse failed");
        let records = parser.into_fragment_records();

        assert_stream!(records, "test.html", []);
    }

    #[test]
    fn test_comment_arbitrary_identifier_is_stripped() {
        let mut parser = HtmlParser::new();
        let html = "<!--{{someOtherBinding}}-->";
        parser.parse("test.html", html).expect("parse failed");
        let records = parser.into_fragment_records();

        assert_stream!(records, "test.html", []);
    }

    #[test]
    fn test_comment_with_surrounding_content_is_stripped() {
        let mut parser = HtmlParser::new();
        let html = "<div><!--{{tokens}}--></div>";
        parser.parse("test.html", html).expect("parse failed");
        let records = parser.into_fragment_records();

        assert_stream!(records, "test.html", [raw("<div></div>")]);
    }

    #[test]
    fn test_component_template_unterminated_comment_returns_error() {
        let mut parser = HtmlParser::new();
        parser
            .component_registry_mut()
            .register_component(ComponentRegistration::new(
                "x-bad",
                "<span>ok</span><!-- missing close",
                None,
                true,
            ))
            .expect("register failed");

        let result = parser.parse("index.html", "<x-bad></x-bad>");

        assert!(matches!(result, Err(ParserError::Template(diag)) if
            diag.to_string().contains("unterminated HTML comment")
        ));
    }

    #[test]
    fn test_webui_plugin_strips_component_comments_before_metadata() {
        let mut parser =
            HtmlParser::with_plugin(Box::new(crate::plugin::webui::WebUIParserPlugin::new()));
        parser
            .component_registry_mut()
            .register_component(ComponentRegistration::new(
                "x-bleed",
                r#"<!-- {{path}} @click="{bad()}" ?hidden="{{bad}}" --><div>hello</div>"#,
                None,
                true,
            ))
            .expect("register failed");

        parser
            .parse("index.html", "<x-bleed></x-bleed>")
            .expect("parse failed");

        let artifacts = parser.take_plugin_artifacts().unwrap();
        let ParserPluginArtifacts::ComponentTemplates(templates) = artifacts else {
            panic!("expected component templates");
        };
        let template = &templates[0].template_json;
        assert!(template.contains("<div>hello</div>"));
        assert!(!template.contains("{{path}}"));
        assert!(!template.contains("@click"));
        assert!(!template.contains("?hidden"));
        assert!(!template.contains("-->"));
    }

    #[test]
    fn test_webui_plugin_strips_component_style_line_comments_before_metadata() {
        let mut parser =
            HtmlParser::with_plugin(Box::new(crate::plugin::webui::WebUIParserPlugin::new()));
        parser
            .component_registry_mut()
            .register_component(ComponentRegistration::new(
                "x-style",
                "<style>// {{ignored}}\n.x { color: red; }</style><div>hello</div>",
                None,
                true,
            ))
            .expect("register failed");

        parser
            .parse("index.html", "<x-style></x-style>")
            .expect("parse failed");

        let artifacts = parser.take_plugin_artifacts().unwrap();
        let ParserPluginArtifacts::ComponentTemplates(templates) = artifacts else {
            panic!("expected component templates");
        };
        let template = &templates[0].template_json;
        assert!(template.contains(".x { color: red; }"));
        assert!(!template.contains("ignored"));
        assert!(!template.contains("//"));
    }

    // ── Token collection tests ───────────────────────────────────────

    #[test]
    fn test_tokens_from_style_tag() {
        let mut parser = HtmlParser::new();
        let html = r#"<style>
            .btn { color: var(--colorPrimary); background: var(--bgColor); }
        </style>"#;
        parser.parse("test.html", html).expect("parse failed");
        let tokens = parser.take_tokens();

        assert_eq!(tokens, vec!["bgColor", "colorPrimary"]);
    }

    #[test]
    fn test_tokens_from_malformed_style_error_on_unclosed_var() {
        let mut parser = HtmlParser::new();
        let html = r#"<style>
            .bad { color: var(--dangling; }
            .ok { color: var(--valid); }
        </style>"#;
        let result = parser.parse("test.html", html);

        assert!(matches!(result, Err(ParserError::Template(diag)) if
            diag.error_code() == Some(codes::INVALID_CSS)
                && diag.to_string().contains("Unterminated CSS var() call")
        ));
    }

    #[test]
    fn test_tokens_from_component_css() {
        let mut parser = HtmlParser::new();
        parser
            .component_registry_mut()
            .register_component(ComponentRegistration::new(
                "my-button",
                "<button>Click</button>",
                Some(":host { color: var(--textColor); border: var(--borderWidth); }"),
                true,
            ))
            .expect("register failed");

        let html = "<my-button></my-button>";
        parser.parse("test.html", html).expect("parse failed");
        let tokens = parser.take_tokens();

        assert_eq!(tokens, vec!["borderWidth", "textColor"]);
    }

    #[test]
    fn test_token_requirements_preserve_fallback_chains() {
        let mut parser = HtmlParser::new();
        parser
            .component_registry_mut()
            .register_component(ComponentRegistration::new(
                "my-card",
                "<div>Card</div>",
                Some(":host { color: var(--token-a, var(--token-b, var(--token-c), true)); }"),
                true,
            ))
            .expect("register failed");

        parser
            .parse("test.html", "<my-card></my-card>")
            .expect("parse failed");
        let analysis = parser.token_analysis();

        assert_eq!(analysis.fallback_chains.len(), 1);
        assert_eq!(
            analysis.fallback_chains[0].tokens,
            vec!["token-a", "token-b", "token-c"]
        );
        assert!(!analysis.fallback_chains[0].has_literal_fallback);
    }

    #[test]
    fn test_token_requirements_remove_locally_defined_candidates_from_fallback_chain() {
        let mut parser = HtmlParser::new();
        parser
            .component_registry_mut()
            .register_component(ComponentRegistration::new(
                "my-card",
                "<div>Card</div>",
                Some(
                    ":host { --token-a: red; --foo-bar: var(--token-a, var(--token-b, var(--token-c), true)); }",
                ),
             true,))
            .expect("register failed");

        parser
            .parse("test.html", "<my-card></my-card>")
            .expect("parse failed");
        let analysis = parser.token_analysis();

        assert_eq!(analysis.protocol_tokens, vec!["token-b", "token-c"]);
        assert_eq!(analysis.fallback_chains.len(), 1);
        assert_eq!(
            analysis.fallback_chains[0].tokens,
            vec!["token-b", "token-c"]
        );
    }

    #[test]
    fn test_token_analysis_theme_validation_reports_missing_unresolved_token() {
        let mut parser = HtmlParser::new();
        parser
            .component_registry_mut()
            .register_component(ComponentRegistration::new(
                "my-card",
                "<div>Card</div>",
                Some(
                    ":host { --token-a: red; --foo-bar: var(--token-a, var(--token-b, var(--token-c), true)); }",
                ),
             true,))
            .expect("register failed");
        parser
            .parse("test.html", "<my-card></my-card>")
            .expect("parse failed");

        let analysis = parser.token_analysis();
        let theme = webui_tokens::TokenFile {
            themes: HashMap::from([(
                "light".to_string(),
                HashMap::from([("token-b".to_string(), "green".to_string())]),
            )]),
        };

        let Err(ParserError::Template(diagnostic)) = analysis.validate_theme_tokens(&theme) else {
            panic!("missing --token-c in the theme must fail parser validation");
        };
        assert_eq!(diagnostic.error_code(), Some(codes::MISSING_THEME_TOKEN));
        // The help is concise: a typo suggestion plus the local-definition
        // escape hatch — it does not restate "add --token-c to theme".
        let help = diagnostic.help_text().expect("help text");
        assert!(help.contains("did you mean --token-b?"), "help: {help}");
        assert!(help.contains("define it locally"), "help: {help}");
        assert!(!help.contains("add --token"), "help: {help}");
    }

    #[test]
    fn test_token_analysis_literal_fallback_token_exempt_from_theme_validation() {
        let mut parser = HtmlParser::new();
        parser
            .component_registry_mut()
            .register_component(ComponentRegistration::new(
                "my-card",
                "<div>Card</div>",
                Some(":host { color: var(--brand, #000); }"),
                true,
            ))
            .expect("register failed");
        parser
            .parse("test.html", "<my-card></my-card>")
            .expect("parse failed");

        let analysis = parser.token_analysis();
        // The token is still hoisted so the runtime resolves it when a theme
        // does provide it.
        assert_eq!(analysis.protocol_tokens, vec!["brand"]);

        // A theme without `--brand` must NOT fail the build: the CSS literal
        // fallback (`#000`) already provides a value.
        let theme = webui_tokens::TokenFile {
            themes: HashMap::from([("light".to_string(), HashMap::new())]),
        };
        analysis
            .validate_theme_tokens(&theme)
            .expect("literal fallback must exempt --brand from theme validation");
    }

    #[test]
    fn test_token_analysis_mixed_literal_and_bare_usage_requires_token() {
        let mut parser = HtmlParser::new();
        parser
            .component_registry_mut()
            .register_component(ComponentRegistration::new(
                "my-card",
                "<div>Card</div>",
                // One usage has a literal fallback, the other does not. The
                // bare `var(--brand)` makes the token genuinely required.
                Some(":host { color: var(--brand, #000); background: var(--brand); }"),
                true,
            ))
            .expect("register failed");
        parser
            .parse("test.html", "<my-card></my-card>")
            .expect("parse failed");

        let analysis = parser.token_analysis();
        let theme = webui_tokens::TokenFile {
            themes: HashMap::from([("light".to_string(), HashMap::new())]),
        };

        let Err(ParserError::Template(diagnostic)) = analysis.validate_theme_tokens(&theme) else {
            panic!("a bare var(--brand) usage must require --brand in the theme");
        };
        assert_eq!(diagnostic.error_code(), Some(codes::MISSING_THEME_TOKEN));
        assert!(diagnostic.to_string().contains("--brand"));
    }

    #[test]
    fn test_unthemed_literal_fallback_tokens_flags_only_literal_only_absent_tokens() {
        let mut parser = HtmlParser::new();
        parser
            .component_registry_mut()
            .register_component(ComponentRegistration::new(
                "my-card",
                "<div>Card</div>",
                Some(
                    ":host { \
                       color: var(--colr-brand, #000); \
                       border: var(--present, 1px); \
                       margin: var(--required); \
                     }",
                ),
                true,
            ))
            .expect("register failed");
        parser
            .parse("test.html", "<my-card></my-card>")
            .expect("parse failed");

        let analysis = parser.token_analysis();
        let theme = webui_tokens::TokenFile {
            themes: HashMap::from([(
                "light".to_string(),
                HashMap::from([
                    ("present".to_string(), "2px".to_string()),
                    ("required".to_string(), "8px".to_string()),
                ]),
            )]),
        };

        // `colr-brand`: literal-only and absent from every theme → warned.
        // `present`: literal-only but defined in the theme → not warned.
        // `required`: has no literal fallback → a validation concern, not a warning.
        assert_eq!(
            analysis.unthemed_literal_fallback_tokens(&theme),
            vec!["colr-brand".to_string()]
        );
    }

    #[test]
    fn test_theme_token_error_reports_location_and_suggestion() {
        let mut parser = HtmlParser::new();
        parser
            .component_registry_mut()
            .register_component(ComponentRegistration::new(
                "my-card",
                "<div>Card</div>",
                ":host {\n  color: var(--color-neutral-2000);\n}".into(),
                true,
            ))
            .expect("register failed");
        parser
            .parse("test.html", "<my-card></my-card>")
            .expect("parse failed");

        let analysis = parser.token_analysis();
        let theme = webui_tokens::TokenFile {
            themes: HashMap::from([(
                "dark".to_string(),
                HashMap::from([("color-neutral-200".to_string(), "#222".to_string())]),
            )]),
        };

        let Err(ParserError::Template(diag)) = analysis.validate_theme_tokens(&theme) else {
            panic!("missing --color-neutral-2000 must fail validation");
        };
        assert_eq!(diag.error_code(), Some(codes::MISSING_THEME_TOKEN));
        // File + line:column, like other authoring diagnostics.
        let location = diag.location().expect("a source location");
        assert!(location.contains("my-card.css:2:"), "location: {location}");
        // Snippet shows the offending CSS line.
        assert!(
            diag.snippet_text()
                .is_some_and(|s| s.contains("--color-neutral-2000")),
            "snippet: {:?}",
            diag.snippet_text()
        );
        // Did-you-mean from the theme's own tokens.
        assert!(
            diag.help_text()
                .is_some_and(|h| h.contains("did you mean --color-neutral-200?")),
            "help: {:?}",
            diag.help_text()
        );
    }

    #[test]
    fn test_theme_token_warning_reports_location_and_suggestion() {
        let mut parser = HtmlParser::new();
        parser
            .component_registry_mut()
            .register_component(ComponentRegistration::new(
                "my-card",
                "<div>Card</div>",
                ":host {\n  color: var(--colr-brand, #000);\n}".into(),
                true,
            ))
            .expect("register failed");
        parser
            .parse("test.html", "<my-card></my-card>")
            .expect("parse failed");

        let analysis = parser.token_analysis();
        let theme = webui_tokens::TokenFile {
            themes: HashMap::from([(
                "light".to_string(),
                HashMap::from([("color-brand".to_string(), "#abc".to_string())]),
            )]),
        };

        let warnings = analysis.theme_token_warnings(&theme);
        assert_eq!(warnings.len(), 1, "warnings: {warnings:?}");
        let body = warnings[0].body();
        assert!(body.contains("my-card.css:2:"), "warning: {body}");
        assert!(body.contains("--colr-brand"), "warning: {body}");
        assert!(
            body.contains("did you mean --color-brand?"),
            "warning: {body}"
        );
    }

    #[test]
    fn test_tokens_from_malformed_component_css_error_on_unclosed_var() {
        let mut parser = HtmlParser::new();
        let result =
            parser
                .component_registry_mut()
                .register_component(ComponentRegistration::new(
                    "my-card",
                    "<div>Card</div>",
                    Some(".bad { color: var(--dangling; } .ok { color: var(--valid); }"),
                    true,
                ));

        assert!(matches!(result, Err(ParserError::Css(message)) if
            message.contains("Unterminated CSS var() call")
        ));
    }

    #[test]
    fn test_tokens_merged_from_style_and_components() {
        let mut parser = HtmlParser::new();
        parser
            .component_registry_mut()
            .register_component(ComponentRegistration::new(
                "my-widget",
                "<div>Widget</div>",
                Some(".w { padding: var(--spacingM); }"),
                true,
            ))
            .expect("register failed");

        let html = r#"<style>.root { color: var(--textColor); }</style><my-widget></my-widget>"#;
        parser.parse("test.html", html).expect("parse failed");
        let tokens = parser.take_tokens();

        assert_eq!(tokens, vec!["spacingM", "textColor"]);
    }

    #[test]
    fn test_tokens_deduplicated() {
        let mut parser = HtmlParser::new();
        parser
            .component_registry_mut()
            .register_component(ComponentRegistration::new(
                "my-btn",
                "<button>B</button>",
                Some(".b { color: var(--shared); }"),
                true,
            ))
            .expect("register failed");

        let html = r#"<style>.x { color: var(--shared); }</style><my-btn></my-btn>"#;
        parser.parse("test.html", html).expect("parse failed");
        let tokens = parser.take_tokens();

        assert_eq!(tokens, vec!["shared"]);
    }

    #[test]
    fn test_tokens_empty_when_no_vars() {
        let mut parser = HtmlParser::new();
        let html = "<div>Hello</div>";
        parser.parse("test.html", html).expect("parse failed");
        let tokens = parser.take_tokens();

        assert!(tokens.is_empty());
    }

    #[test]
    fn test_tokens_exclude_locally_defined_in_component() {
        let mut parser = HtmlParser::new();
        parser
            .component_registry_mut()
            .register_component(ComponentRegistration::new(
                "my-card",
                "<div>Card</div>",
                Some(":host { --local: 5px; width: var(--external); }"),
                true,
            ))
            .expect("register failed");

        let html = "<my-card></my-card>";
        parser.parse("test.html", html).expect("parse failed");
        let tokens = parser.take_tokens();

        assert_eq!(tokens, vec!["external"]);
    }

    #[test]
    fn test_tokens_exclude_entry_root_definitions() {
        let mut parser = HtmlParser::new();
        parser
            .component_registry_mut()
            .register_component(ComponentRegistration::new(
                "my-btn",
                "<button>B</button>",
                Some(".b { color: var(--color-primary); border-radius: var(--radius-m); }"),
                true,
            ))
            .expect("register failed");

        // Entry HTML defines --color-primary and --radius-m in :root
        // Components use them — they should NOT appear in hoisted tokens
        let html = r#"<style>
            :root {
                --color-primary: #0078d4;
                --radius-m: 6px;
            }
            body { color: var(--color-primary); }
        </style>
        <my-btn></my-btn>"#;
        parser.parse("test.html", html).expect("parse failed");
        let tokens = parser.take_tokens();

        // Both tokens are defined in entry :root, so neither should be hoisted
        assert!(
            tokens.is_empty(),
            "Tokens defined in entry :root should be excluded: {tokens:?}"
        );
    }

    #[test]
    fn test_tokens_entry_defs_exclude_but_external_kept() {
        let mut parser = HtmlParser::new();
        parser
            .component_registry_mut()
            .register_component(ComponentRegistration::new(
                "my-card",
                "<div>Card</div>",
                Some(".c { color: var(--color-primary); margin: var(--external-spacing); }"),
                true,
            ))
            .expect("register failed");

        // Entry defines --color-primary but NOT --external-spacing
        let html = r#"<style>
            :root { --color-primary: #0078d4; }
        </style>
        <my-card></my-card>"#;
        parser.parse("test.html", html).expect("parse failed");
        let tokens = parser.take_tokens();

        // Only --external-spacing should be hoisted (not defined in entry)
        assert_eq!(tokens, vec!["external-spacing"]);
    }

    // ── Route parsing tests ─────────────────────────────────────────────

    #[test]
    fn test_parse_simple_route() {
        let mut parser = HtmlParser::new();
        let html = r#"<route path="/profile" component="profile-page" exact />"#;
        parser.parse("test.html", html).expect("parse failed");

        // Routes are emitted as Fragment::Route
        let records = parser.into_fragment_records();
        let frags = &records["test.html"].fragments;
        assert_eq!(frags.len(), 1);
        match frags[0].fragment.as_ref() {
            Some(web_ui_fragment::Fragment::Route(r)) => {
                assert_eq!(r.path, "/profile");
                assert_eq!(r.fragment_id, "profile-page");
                assert!(r.exact);
            }
            other => panic!("expected Fragment::Route, got {:?}", other),
        }
    }

    #[test]
    fn test_parse_route_with_params() {
        let mut parser = HtmlParser::new();
        let html = r#"<route path="/profile/:id/view/:section" component="detail" />"#;
        parser.parse("test.html", html).expect("parse failed");

        // Route is emitted as Fragment::Route with correct path
        let records = parser.into_fragment_records();
        let frags = &records["test.html"].fragments;
        match frags[0].fragment.as_ref() {
            Some(web_ui_fragment::Fragment::Route(r)) => {
                assert_eq!(r.path, "/profile/:id/view/:section");
            }
            other => panic!("expected Fragment::Route, got {:?}", other),
        }
    }

    #[test]
    fn test_parse_route_requires_component() {
        let mut parser = HtmlParser::new();
        let html = r#"<route path="/old" />"#;
        let result = parser.parse("test.html", html);
        assert!(result.is_err());
    }

    #[test]
    fn test_parse_multiple_routes() {
        let mut parser = HtmlParser::new();
        let html = r#"<route path="/" component="app-layout" />
            <route path="/dashboard" component="dash-page" exact />
            <route path="/contacts" component="contacts-page" exact />"#;
        parser.parse("test.html", html).expect("parse failed");

        let records = parser.into_fragment_records();
        let frags = &records["test.html"].fragments;
        assert_eq!(frags.len(), 3);
        // All should be Fragment::Route
        for frag in frags {
            assert!(matches!(
                frag.fragment.as_ref(),
                Some(web_ui_fragment::Fragment::Route(_))
            ));
        }
    }

    #[test]
    fn test_parse_route_requires_component_with_body() {
        let mut parser = HtmlParser::new();
        let html = r#"<route path="/error">
            <div class="error"><h1>Not Found</h1></div>
        </route>"#;
        let result = parser.parse("test.html", html);
        assert!(
            result.is_err(),
            "Route without component attribute should fail"
        );
    }

    #[test]
    fn test_parse_multiple_routes_with_fragments() {
        let mut parser = HtmlParser::new();
        let html = r#"<route path="/" component="home-page" exact />
            <route path="/about" component="about-page" exact />
            <route path="/contact/:id" component="contact-page" />"#;
        parser.parse("test.html", html).expect("parse failed");

        let records = parser.into_fragment_records();
        let frags = &records["test.html"].fragments;
        assert_eq!(frags.len(), 3);

        // Verify individual routes
        if let Some(web_ui_fragment::Fragment::Route(r)) = frags[0].fragment.as_ref() {
            assert_eq!(r.path, "/");
        }
        if let Some(web_ui_fragment::Fragment::Route(r)) = frags[1].fragment.as_ref() {
            assert_eq!(r.path, "/about");
        }
        if let Some(web_ui_fragment::Fragment::Route(r)) = frags[2].fragment.as_ref() {
            assert_eq!(r.path, "/contact/:id");
        }
    }

    #[test]
    fn test_parse_route_with_query_allowlist() {
        let mut parser = HtmlParser::new();
        let html =
            r#"<route path="/compose" component="compose-page" query="action,to,subject" exact />"#;
        parser.parse("test.html", html).expect("parse failed");

        let records = parser.into_fragment_records();
        let frags = &records["test.html"].fragments;
        assert_eq!(frags.len(), 1);
        match frags[0].fragment.as_ref() {
            Some(web_ui_fragment::Fragment::Route(r)) => {
                assert_eq!(r.path, "/compose");
                assert_eq!(r.fragment_id, "compose-page");
                assert!(r.exact);
                assert_eq!(r.allowed_query, "action,to,subject");
            }
            other => panic!("expected Fragment::Route, got {:?}", other),
        }
    }

    #[test]
    fn test_parse_route_without_query_has_empty_allowed_query() {
        let mut parser = HtmlParser::new();
        let html = r#"<route path="/profile" component="profile-page" exact />"#;
        parser.parse("test.html", html).expect("parse failed");

        let records = parser.into_fragment_records();
        let frags = &records["test.html"].fragments;
        match frags[0].fragment.as_ref() {
            Some(web_ui_fragment::Fragment::Route(r)) => {
                assert!(r.allowed_query.is_empty());
            }
            other => panic!("expected Fragment::Route, got {:?}", other),
        }
    }

    #[test]
    fn test_route_body_rejects_malformed_non_route_html() {
        let mut parser = HtmlParser::new();
        let html = r#"<route path="/" component="home-page"><div><span></div></route>"#;
        let result = parser.parse("test.html", html);

        assert!(matches!(result, Err(ParserError::Template(diag)) if
            diag.to_string().contains("unclosed <span> tag")
        ));
    }

    #[test]
    fn test_route_body_rejects_malformed_style_css() {
        let mut parser = HtmlParser::new();
        let html = r#"<route path="/" component="home-page"><style>.bad { color: var(--x; }</style></route>"#;
        let result = parser.parse("test.html", html);

        assert!(matches!(result, Err(ParserError::Template(diag)) if
            diag.error_code() == Some(codes::INVALID_CSS)
                && diag.to_string().contains("Unterminated CSS var() call")
        ));
    }

    #[test]
    fn test_outlet_not_captured_by_for_loop() {
        let mut parser = HtmlParser::new();
        let html = r#"<template shadowrootmode="open">
  <ul>
    <for each="item in items">
      <li>{{item.name}}</li>
    </for>
  </ul>
  <main>
    <outlet />
  </main>
</template>"#;
        parser.parse("comp.html", html).expect("parse failed");

        let records = parser.into_fragment_records();
        let frags = &records["comp.html"].fragments;

        // The outlet should be a top-level fragment, NOT inside the for-loop's body
        let outlet_count = frags
            .iter()
            .filter(|f| {
                matches!(
                    f.fragment.as_ref(),
                    Some(web_ui_fragment::Fragment::Outlet(_))
                )
            })
            .count();
        assert_eq!(
            outlet_count, 1,
            "expected exactly 1 outlet in top-level fragments, got {outlet_count}"
        );

        // Verify outlet comes AFTER the raw "</ul>" text
        let outlet_idx = frags
            .iter()
            .position(|f| {
                matches!(
                    f.fragment.as_ref(),
                    Some(web_ui_fragment::Fragment::Outlet(_))
                )
            })
            .expect("no outlet found");
        let close_ul_idx = frags.iter().position(|f| match f.fragment.as_ref() {
            Some(web_ui_fragment::Fragment::Raw(r)) => r.value.contains("</ul>"),
            _ => false,
        });
        if let Some(ul_idx) = close_ul_idx {
            assert!(
                outlet_idx > ul_idx,
                "outlet (at {outlet_idx}) should come after </ul> (at {ul_idx})"
            );
        }
    }

    #[test]
    fn test_outlet_position_after_for_not_inside() {
        let mut parser = HtmlParser::new();
        let html = r#"<ul><for each="x in items"><li>ok</li></for></ul><outlet />"#;
        parser.parse("test.html", html).expect("parse failed");

        let records = parser.into_fragment_records();
        let frags = &records["test.html"].fragments;

        // Outlet should be its own fragment at top level
        let has_outlet = frags.iter().any(|f| {
            matches!(
                f.fragment.as_ref(),
                Some(web_ui_fragment::Fragment::Outlet(_))
            )
        });
        assert!(
            has_outlet,
            "outlet should be in top-level fragments: {frags:?}"
        );

        // The for-loop body should NOT contain the outlet
        let for_id = frags.iter().find_map(|f| match f.fragment.as_ref() {
            Some(web_ui_fragment::Fragment::ForLoop(fl)) => Some(fl.fragment_id.clone()),
            _ => None,
        });
        if let Some(id) = for_id {
            let for_frags = &records[&id].fragments;
            let outlet_in_for = for_frags.iter().any(|f| {
                matches!(
                    f.fragment.as_ref(),
                    Some(web_ui_fragment::Fragment::Outlet(_))
                )
            });
            assert!(
                !outlet_in_for,
                "outlet should NOT be inside for-loop body: {for_frags:?}"
            );
        }
    }

    #[test]
    fn test_style_element_with_handlebars_signal_comment_emits_signal() {
        let mut parser = HtmlParser::new();
        let html = r#"<html><head><style>
:root {
    /*{{{tokens.light}}}*/
}
</style></head><body></body></html>"#;

        let result = parser.parse("test.html", html);
        assert!(result.is_ok(), "Parse error: {:?}", result.err());
        let fragment_records = parser.into_fragment_records();

        assert_stream!(
            fragment_records,
            "test.html",
            [
                raw("<html><head>"),
                structural_matcher("head_start"),
                raw("<style>\n:root {\n    "),
                signal_raw("tokens.light"),
                raw("\n}\n</style>"),
                structural_matcher("head_end"),
                raw("</head><body>"),
                structural_matcher("body_start"),
                structural_matcher("body_end"),
                raw("</body></html>"),
            ]
        );
    }

    #[test]
    fn test_style_comment_signal_with_spaces_emits_signal() {
        let mut parser = HtmlParser::new();
        let html = r#"<html><head><style>
:root {
    /* {{{tokens.light}}} */
}
</style></head><body></body></html>"#;

        let result = parser.parse("test.html", html);
        assert!(result.is_ok(), "Parse error: {:?}", result.err());
        let fragment_records = parser.into_fragment_records();

        assert_stream!(
            fragment_records,
            "test.html",
            [
                raw("<html><head>"),
                structural_matcher("head_start"),
                raw("<style>\n:root {\n    "),
                signal_raw("tokens.light"),
                raw("\n}\n</style>"),
                structural_matcher("head_end"),
                raw("</head><body>"),
                structural_matcher("body_start"),
                structural_matcher("body_end"),
                raw("</body></html>"),
            ]
        );
    }

    #[test]
    fn test_style_comment_signal_double_brace_emits_signal() {
        let mut parser = HtmlParser::new();
        let html = "<html><head><style>/*{{themeCss}}*/</style></head><body></body></html>";

        let result = parser.parse("test.html", html);
        assert!(result.is_ok(), "Parse error: {:?}", result.err());
        let fragment_records = parser.into_fragment_records();

        assert_stream!(
            fragment_records,
            "test.html",
            [
                raw("<html><head>"),
                structural_matcher("head_start"),
                raw("<style>"),
                signal("themeCss"),
                raw("</style>"),
                structural_matcher("head_end"),
                raw("</head><body>"),
                structural_matcher("body_start"),
                structural_matcher("body_end"),
                raw("</body></html>"),
            ]
        );
    }

    #[test]
    fn test_style_comment_with_extra_text_is_stripped() {
        let mut parser = HtmlParser::new();
        let html = "<html><head><style>/* theme: {{token}} */</style></head><body></body></html>";

        let result = parser.parse("test.html", html);
        assert!(result.is_ok(), "Parse error: {:?}", result.err());
        let fragment_records = parser.into_fragment_records();

        assert_stream!(
            fragment_records,
            "test.html",
            [
                raw("<html><head>"),
                structural_matcher("head_start"),
                raw("<style></style>"),
                structural_matcher("head_end"),
                raw("</head><body>"),
                structural_matcher("body_start"),
                structural_matcher("body_end"),
                raw("</body></html>"),
            ]
        );
    }

    #[test]
    fn test_style_comment_with_multiple_signals_is_stripped() {
        let mut parser = HtmlParser::new();
        let html = "<html><head><style>/*{{a}}{{b}}*/</style></head><body></body></html>";

        let result = parser.parse("test.html", html);
        assert!(result.is_ok(), "Parse error: {:?}", result.err());
        let fragment_records = parser.into_fragment_records();

        assert_stream!(
            fragment_records,
            "test.html",
            [
                raw("<html><head>"),
                structural_matcher("head_start"),
                raw("<style></style>"),
                structural_matcher("head_end"),
                raw("</head><body>"),
                structural_matcher("body_start"),
                structural_matcher("body_end"),
                raw("</body></html>"),
            ]
        );
    }

    #[test]
    fn test_style_bare_handlebars_stays_raw() {
        let mut parser = HtmlParser::new();
        // Handlebars outside a CSS comment must NOT be parsed as signals.
        let html =
            "<html><head><style>body { color: {{textColor}}; }</style></head><body></body></html>";

        let result = parser.parse("test.html", html);
        assert!(result.is_ok(), "Parse error: {:?}", result.err());
        let fragment_records = parser.into_fragment_records();

        assert_stream!(
            fragment_records,
            "test.html",
            [
                raw("<html><head>"),
                structural_matcher("head_start"),
                raw("<style>body { color: {{textColor}}; }</style>"),
                structural_matcher("head_end"),
                raw("</head><body>"),
                structural_matcher("body_start"),
                structural_matcher("body_end"),
                raw("</body></html>"),
            ]
        );
    }

    #[test]
    fn test_style_mixed_css_and_comment_signal_emits_signal() {
        let mut parser = HtmlParser::new();
        let html = r#"<html><head><style>
  .a { color: red; }
  /*{{themeCss}}*/
  .b { color: blue; }
</style></head><body></body></html>"#;

        let result = parser.parse("test.html", html);
        assert!(result.is_ok(), "Parse error: {:?}", result.err());
        let fragment_records = parser.into_fragment_records();

        assert_stream!(
            fragment_records,
            "test.html",
            [
                raw("<html><head>"),
                structural_matcher("head_start"),
                raw("<style>\n  .a { color: red; }\n  "),
                signal("themeCss"),
                raw("\n  .b { color: blue; }\n</style>"),
                structural_matcher("head_end"),
                raw("</head><body>"),
                structural_matcher("body_start"),
                structural_matcher("body_end"),
                raw("</body></html>"),
            ]
        );
    }

    #[test]
    fn test_style_legal_comment_preserved_inline_by_default() {
        let mut parser = HtmlParser::new();
        let html = "<html><head><style>/*! @license MIT */ .x { color: red; } /* remove */</style></head><body></body></html>";

        parser.parse("test.html", html).expect("parse failed");
        let fragment_records = parser.into_fragment_records();

        assert_stream!(
            fragment_records,
            "test.html",
            [
                raw("<html><head>"),
                structural_matcher("head_start"),
                raw("<style>/*! @license MIT */ .x { color: red; } </style>"),
                structural_matcher("head_end"),
                raw("</head><body>"),
                structural_matcher("body_start"),
                structural_matcher("body_end"),
                raw("</body></html>"),
            ]
        );
    }

    #[test]
    fn test_style_legal_comments_none_strips_legal_comment() {
        let mut parser = HtmlParser::with_options(ParserOptions {
            legal_comments: LegalComments::None,
            ..ParserOptions::default()
        });
        let html = "<html><head><style>/*! @license MIT */ .x { color: red; }</style></head><body></body></html>";

        parser.parse("test.html", html).expect("parse failed");
        let fragment_records = parser.into_fragment_records();

        assert_stream!(
            fragment_records,
            "test.html",
            [
                raw("<html><head>"),
                structural_matcher("head_start"),
                raw("<style> .x { color: red; }</style>"),
                structural_matcher("head_end"),
                raw("</head><body>"),
                structural_matcher("body_start"),
                structural_matcher("body_end"),
                raw("</body></html>"),
            ]
        );
    }

    #[test]
    fn test_style_element_plain_css_unchanged() {
        let mut parser = HtmlParser::new();
        let html = "<html><head><style>body { margin: 0; }</style></head><body></body></html>";

        let result = parser.parse("test.html", html);
        assert!(result.is_ok(), "Parse error: {:?}", result.err());
        let fragment_records = parser.into_fragment_records();

        // Plain CSS with no handlebars should remain as a single raw fragment.
        assert_stream!(
            fragment_records,
            "test.html",
            [
                raw("<html><head>"),
                structural_matcher("head_start"),
                raw("<style>body { margin: 0; }</style>"),
                structural_matcher("head_end"),
                raw("</head><body>"),
                structural_matcher("body_start"),
                structural_matcher("body_end"),
                raw("</body></html>"),
            ]
        );
    }

    #[test]
    fn test_unclosed_style_element_returns_error() {
        let mut parser = HtmlParser::new();
        let result = parser.parse("index.html", "<style>.x { color: red; ");
        assert!(matches!(result, Err(ParserError::Template(diag)) if
            diag.to_string().contains("unclosed <style> tag")
        ));
    }

    #[test]
    fn test_text_with_binding_after_void_element() {
        // Text nodes containing {{binding}} after void elements like <br>
        // must be preserved. Regression test for content being dropped.
        let (fragments, _) = parse_and_get_fragments(
            "Part A: {{value1}}<button>Click</button><br>Part B: {{value2}}<button>Click2</button>",
        );
        assert_fragments!(
            fragments,
            [
                raw("Part A: "),
                signal("value1"),
                raw("<button>Click</button><br>Part B: "),
                signal("value2"),
                raw("<button>Click2</button>"),
            ]
        );
    }
}

// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

//! Shared declaration discovery and invocation resolution for SSR and client compilation.

mod tables;

use std::collections::HashMap;
use std::ops::Range;

use crate::html_parser::{self as html, Element, Event, Walker};
use crate::{codes, Diagnostic, Result};

pub(crate) const RECORD_PREFIX: &str = "}}}webui:fragment:";

pub(crate) fn record_id(owner: &str, name: &str) -> String {
    format!("{RECORD_PREFIX}{}:{owner}:{name}", owner.len())
}

pub(crate) struct FragmentDeclaration {
    pub name: String,
    pub fragment_id: String,
    pub range: Range<usize>,
    pub body: Range<usize>,
}

pub(crate) struct RenderDirective {
    pub target: usize,
    pub scope: String,
    pub alias: String,
    pub range: Range<usize>,
}

pub(crate) struct FragmentDeclarations {
    pub root_body: Range<usize>,
    pub declarations: Vec<FragmentDeclaration>,
    pub renders: Vec<RenderDirective>,
    pub table_wrappers: Vec<TableWrapper>,
}

pub(crate) struct TableWrapper {
    pub table_start: usize,
    pub range: Range<usize>,
    pub tag: &'static str,
}

struct UnresolvedRender {
    name: String,
    directive: RenderDirective,
}

struct ScanFrame<'a> {
    walker: Walker<'a>,
    declarations_allowed: bool,
    blocked: bool,
    entry_container: bool,
    ignored_route: bool,
    route_children: bool,
}

impl FragmentDeclarations {
    pub(crate) fn collect(
        owner: &str,
        source: &str,
        is_component: bool,
        reject: bool,
    ) -> Result<Option<Self>> {
        if !contains_directives(source) {
            return Ok(None);
        }
        Self::collect_present(owner, source, is_component, reject).map(Some)
    }

    pub(crate) fn collect_present(
        owner: &str,
        source: &str,
        is_component: bool,
        reject: bool,
    ) -> Result<Self> {
        let root_body = component_root_body(source, is_component);
        let mut declarations = Vec::new();
        let mut calls = Vec::new();
        let mut names = HashMap::new();
        let mut stack = vec![ScanFrame {
            walker: Walker::new_range(source, 0, source.len()),
            declarations_allowed: root_body.start == 0,
            blocked: false,
            entry_container: !is_component,
            ignored_route: false,
            route_children: false,
        }];
        while let Some(frame) = stack.last_mut() {
            let Some(event) = frame.walker.next() else {
                stack.pop();
                continue;
            };
            let Event::Element(element) = event else {
                continue;
            };
            let directive = if reject {
                directive_name(element.name())
            } else {
                matches!(element.name(), "fragment" | "render")
            };
            if directive && reject {
                return Err(directive_error(
                    owner,
                    &element,
                    codes::UNSUPPORTED_FRAGMENT_DIRECTIVE,
                    "named fragment directives are not supported by FAST",
                    "use the WebUI plugin, or replace <fragment>/<render> with ordinary FAST template content",
                ));
            }
            if directive && (frame.blocked || frame.ignored_route) {
                return Err(directive_error(
                    owner,
                    &element,
                    codes::INVALID_FRAGMENT_PLACEMENT,
                    "fragment directives cannot render in raw, inert, or ignored content",
                    "move the declaration to the owning root and its render call into active HTML content",
                ));
            }
            if element.name() == "fragment" {
                if !frame.declarations_allowed {
                    return Err(directive_error(
                        owner,
                        &element,
                        codes::INVALID_FRAGMENT_PLACEMENT,
                        "a fragment declaration must be a direct child of its owning root",
                        "declare fragments directly inside the component root or entry <body>, not inside another element or directive",
                    ));
                }
                let declaration = parse_declaration(owner, &element)?;
                let index = declarations.len();
                if names.insert(declaration.name.clone(), index).is_some() {
                    return Err(directive_error(
                        owner,
                        &element,
                        codes::DUPLICATE_FRAGMENT,
                        "duplicate local fragment name",
                        "give every fragment in this entry or component a unique static name",
                    ));
                }
                declarations.push(declaration);
            } else if element.name() == "render" {
                calls.push(parse_render(owner, &element)?);
                continue;
            }
            if element.inner().is_empty() {
                continue;
            }
            let is_root = element.inner() == root_body;
            let entry_body = !is_component
                && frame.entry_container
                && element.name().eq_ignore_ascii_case("body");
            let blocked = frame.blocked || (!is_root && is_inert_context(element.name()));
            let route_children = element.name().eq_ignore_ascii_case("route");
            let ignored_route = route_children
                || (frame.ignored_route && !(frame.route_children && element.name() == "boundary"));
            let entry_container =
                frame.entry_container && element.name().eq_ignore_ascii_case("html");
            stack.push(ScanFrame {
                walker: Walker::new_range(source, element.content_start, element.content_end),
                declarations_allowed: is_root || entry_body,
                blocked,
                entry_container,
                ignored_route,
                route_children,
            });
        }
        let mut renders = Vec::with_capacity(calls.len());
        for mut call in calls {
            let Some(target) = names.get(&call.name).copied() else {
                return Err(unknown_fragment(owner, source, &call, &names));
            };
            call.directive.target = target;
            renders.push(call.directive);
        }
        let mut graph = Self {
            root_body,
            declarations,
            renders,
            table_wrappers: Vec::new(),
        };
        if !graph.declarations.is_empty() {
            graph.table_wrappers = tables::collect(source, &graph);
        }
        Ok(graph)
    }

    pub(crate) fn render_at(&self, offset: usize) -> Option<&RenderDirective> {
        self.renders
            .binary_search_by_key(&offset, |render| render.range.start)
            .ok()
            .map(|index| &self.renders[index])
    }

    pub(crate) fn declaration_at(&self, offset: usize) -> Option<&FragmentDeclaration> {
        self.declarations
            .binary_search_by_key(&offset, |declaration| declaration.range.start)
            .ok()
            .map(|index| &self.declarations[index])
    }
}

pub(crate) fn contains_directives(source: &str) -> bool {
    source.match_indices('<').any(|(offset, _)| {
        let tail = &source.as_bytes()[offset + 1..];
        starts_tag(tail, b"fragment") || starts_tag(tail, b"render")
    })
}

#[derive(Default)]
pub(crate) struct SourceFeatures {
    pub(crate) directives: bool,
    pub(crate) scripts: bool,
}

impl SourceFeatures {
    pub(crate) fn scan(source: &str) -> Self {
        let mut features = Self::default();
        for (offset, _) in source.match_indices('<') {
            let tail = &source.as_bytes()[offset + 1..];
            features.directives |= starts_tag(tail, b"fragment") || starts_tag(tail, b"render");
            // Match the opening-tag scanner's optional whitespace after `<`.
            features.scripts |= starts_tag(tail.trim_ascii_start(), b"script");
            if features.directives && features.scripts {
                break;
            }
        }
        features
    }
}

fn starts_tag(tail: &[u8], tag: &[u8]) -> bool {
    tail.get(..tag.len())
        .is_some_and(|name| name.eq_ignore_ascii_case(tag))
        && tail
            .get(tag.len())
            .is_none_or(|byte| byte.is_ascii_whitespace() || matches!(byte, b'/' | b'>'))
}

fn directive_name(name: &str) -> bool {
    name.eq_ignore_ascii_case("fragment") || name.eq_ignore_ascii_case("render")
}

fn component_root_body(source: &str, is_component: bool) -> Range<usize> {
    if !is_component {
        return entry_root_body(source);
    }
    let mut root = None;
    for event in Walker::new_range(source, 0, source.len()) {
        match event {
            Event::Comment(_) => {}
            Event::Text(text) if text.trim().is_empty() => {}
            Event::Element(element)
                if root.is_none()
                    && element.name().eq_ignore_ascii_case("template")
                    && (element.attrs().next().is_none()
                        || element.attrs().any(|attr| {
                            attr.name.eq_ignore_ascii_case("shadowrootmode")
                                && attr
                                    .value
                                    .is_some_and(|value| value.eq_ignore_ascii_case("open"))
                        })
                        || element.has_attr("w-render")
                        || element.has_attr("w-hydrate")) =>
            {
                root = Some(element.inner());
            }
            _ => return 0..source.len(),
        }
    }
    root.unwrap_or(0..source.len())
}

fn entry_root_body(source: &str) -> Range<usize> {
    let mut root = 0..source.len();
    for _ in 0..2 {
        let mut html = None;
        for event in Walker::new_range(source, root.start, root.end) {
            let Event::Element(element) = event else {
                continue;
            };
            if element.name().eq_ignore_ascii_case("body") {
                return element.inner();
            }
            if element.name().eq_ignore_ascii_case("html") {
                html = Some(element.inner());
            }
        }
        let Some(inner) = html else {
            break;
        };
        root = inner;
    }
    root
}

fn is_inert_context(name: &str) -> bool {
    html::is_raw_text_element(name)
        || name.eq_ignore_ascii_case("template")
        || name.eq_ignore_ascii_case("noscript")
        || name.eq_ignore_ascii_case("textarea")
        || name.eq_ignore_ascii_case("title")
        || name.eq_ignore_ascii_case("plaintext")
}

fn parse_declaration(owner: &str, element: &Element<'_>) -> Result<FragmentDeclaration> {
    validate_attributes(owner, element, &["name"])?;
    let name = element.attr("name").unwrap_or_default();
    if !identifier(name, true) {
        return Err(directive_error(
            owner,
            element,
            codes::INVALID_FRAGMENT,
            "invalid static fragment name",
            "use name=\"tree-items\": start with an ASCII letter or underscore, followed by letters, digits, underscores, or hyphens",
        ));
    }
    validate_closed(owner, element)?;
    Ok(FragmentDeclaration {
        name: name.to_string(),
        fragment_id: record_id(owner, name),
        range: element.start..element.close_end(),
        body: element.inner(),
    })
}

fn parse_render(owner: &str, element: &Element<'_>) -> Result<UnresolvedRender> {
    validate_attributes(owner, element, &["fragment", "scope", "as"])?;
    let name = element.attr("fragment").unwrap_or_default();
    if !identifier(name, true) {
        return Err(directive_error(
            owner,
            element,
            codes::INVALID_RENDER,
            "invalid static render target",
            "provide fragment=\"local-name\" naming a declaration in this entry or component",
        ));
    }
    validate_closed(owner, element)?;
    for event in Walker::new_range(element.source(), element.content_start, element.content_end) {
        match event {
            Event::Comment(range)
                if html::find_comment_close(&element.source()[range.start..range.end])
                    .is_some() => {}
            Event::Text(text) if text.trim().is_empty() => {}
            _ => {
                return Err(directive_error(
                    owner, element, codes::INVALID_RENDER, "a render call cannot contain a body",
                    "use a self-closing <render ... /> or leave only whitespace and comments between its tags",
                ));
            }
        }
    }
    let (scope, alias) = if !element.has_attr("scope") && !element.has_attr("as") {
        ("", "")
    } else {
        let scope = element.attr("scope").and_then(scope_path);
        let alias = element.attr("as").filter(|value| identifier(value, false));
        match (scope, alias) {
            (Some(scope), Some(alias)) => (scope, alias),
            _ => {
                return Err(directive_error(
                    owner, element, codes::INVALID_RENDER_SCOPE, "invalid render scope or alias",
                    "provide both scope=\"{{object.path}}\" and as=\"items\", or omit both; scope must be one dotted identifier path, not an expression or array index",
                ));
            }
        }
    };
    Ok(UnresolvedRender {
        name: name.to_string(),
        directive: RenderDirective {
            target: 0,
            scope: scope.to_string(),
            alias: alias.to_string(),
            range: element.start..element.close_end(),
        },
    })
}

fn validate_attributes(owner: &str, element: &Element<'_>, allowed: &[&str]) -> Result<()> {
    let mut seen = 0u8;
    for attr in element.attrs() {
        let Some(index) = allowed.iter().position(|name| *name == attr.name) else {
            return Err(directive_error(
                owner, element, codes::INVALID_FRAGMENT_ATTRIBUTE, "unsupported fragment directive attribute",
                "use only name on <fragment>, and fragment plus the optional scope/as pair on <render>; remove bindings and other attributes from the directive",
            ));
        };
        let bit = 1 << index;
        if seen & bit != 0 {
            return Err(directive_error(
                owner,
                element,
                codes::INVALID_FRAGMENT_ATTRIBUTE,
                "duplicate fragment directive attribute",
                "provide each directive attribute exactly once",
            ));
        }
        seen |= bit;
    }
    Ok(())
}

fn validate_closed(owner: &str, element: &Element<'_>) -> Result<()> {
    if !element.self_closing() && element.close_end() == element.content_end() {
        return Err(directive_error(
            owner,
            element,
            codes::UNCLOSED_HTML_TAG,
            "unclosed fragment directive",
            "add the matching closing tag or use a self-closing directive",
        ));
    }
    Ok(())
}

fn scope_path(value: &str) -> Option<&str> {
    let value = value.trim();
    let path = if value.starts_with("{{") {
        value.strip_prefix("{{")?.strip_suffix("}}")?.trim()
    } else {
        value
    };
    path.split('.')
        .all(|part| identifier(part, false))
        .then_some(path)
}

fn identifier(value: &str, hyphens: bool) -> bool {
    let mut bytes = value.bytes();
    bytes
        .next()
        .is_some_and(|first| first.is_ascii_alphabetic() || first == b'_')
        && bytes
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_' || (hyphens && byte == b'-'))
}

#[cold]
#[inline(never)]
fn directive_error(
    owner: &str,
    element: &Element<'_>,
    code: &'static str,
    title: &str,
    help: &str,
) -> crate::ParserError {
    Diagnostic::error(title)
        .code(code)
        .component(owner)
        .element(element.name())
        .at_offset(element.source(), element.start)
        .snippet(element.opening())
        .help(help)
        .into()
}

#[cold]
#[inline(never)]
fn unknown_fragment(
    owner: &str,
    source: &str,
    call: &UnresolvedRender,
    names: &HashMap<String, usize>,
) -> crate::ParserError {
    let diagnostic = Diagnostic::error(format!("unknown local fragment \"{}\"", call.name))
        .code(codes::UNKNOWN_FRAGMENT)
        .component(owner)
        .element("render")
        .at_offset(source, call.directive.range.start)
        .snippet(&source[call.directive.range.clone()]);
    let mut candidates: Vec<&str> = names.keys().map(String::as_str).collect();
    candidates.sort_unstable();
    let suggestion = crate::suggest::closest_match(&call.name, candidates.into_iter());
    diagnostic.help(match suggestion {
        Some(name) => format!("did you mean fragment=\"{name}\"? Fragment names are local to this entry or component"),
        None => "declare this name with <fragment name=\"...\"> at the owning root; fragments cannot be looked up in another component".to_string(),
    }).into()
}

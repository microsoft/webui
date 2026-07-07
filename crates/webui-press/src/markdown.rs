// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

//! Markdown → HTML conversion with syntax highlighting.
//! Uses comrak for GFM markdown and syntect for dual-theme code highlighting.

use comrak::nodes::{NodeHtmlBlock, NodeValue};
use comrak::{parse_document, Arena, Options};
use syntect::parsing::SyntaxSet;
use syntect::util::LinesWithEndings;

use crate::error::Result;

/// Highlighter with pre-loaded syntax definitions and theme.
/// Outputs semantic CSS classes (`hl-keyword`, `hl-string`, etc.)
/// instead of inline styles — colors are controlled via design tokens.
pub struct Highlighter {
    ss: SyntaxSet,
    theme: syntect::highlighting::Theme,
}

impl Default for Highlighter {
    fn default() -> Self {
        Self::new()
    }
}

impl Highlighter {
    pub fn new() -> Self {
        use syntect::highlighting::ThemeSet;
        let mut ts = ThemeSet::load_defaults();
        // Move out of the map so we own it (avoids per-call clone).
        let theme = ts
            .themes
            .remove("InspiredGitHub")
            .unwrap_or_else(|| ts.themes.into_values().next().unwrap_or_default());
        Self {
            ss: SyntaxSet::load_defaults_newlines(),
            theme,
        }
    }

    /// Highlight a code block using semantic CSS classes.
    /// Colors are controlled via CSS custom properties for dual light/dark
    /// theme support (GitHub Light / GitHub Dark style).
    fn highlight_code(&self, code: &str, lang: &str) -> String {
        use syntect::easy::HighlightLines;

        // Map language aliases — syntect's defaults lack TypeScript, TOML, etc.
        let mapped_lang = match lang {
            "ts" | "typescript" | "tsx" | "jsx" => "js",
            "sh" | "shell" | "zsh" => "bash",
            "yml" => "yaml",
            "golang" => "go",
            "csharp" => "cs",
            "toml" => "ini", // closest available
            other => other,
        };

        let syntax = self
            .ss
            .find_syntax_by_token(mapped_lang)
            .unwrap_or_else(|| self.ss.find_syntax_plain_text());

        let mut hl = HighlightLines::new(syntax, &self.theme);

        let mut out = String::with_capacity(code.len() * 2);
        out.push_str("<code-block><pre class=\"code-block\"><code>");

        for line in LinesWithEndings::from(code) {
            let regions = hl.highlight_line(line, &self.ss).unwrap_or_default();
            for &(style, text) in &regions {
                let cls = scope_to_css_class(style.foreground);
                if cls.is_empty() {
                    emit_escaped(&mut out, text);
                } else {
                    out.push_str("<span class=\"hl-");
                    out.push_str(cls);
                    out.push_str("\">");
                    emit_escaped(&mut out, text);
                    out.push_str("</span>");
                }
            }
        }

        out.push_str("</code></pre></code-block>");
        out
    }
}

fn emit_escaped(buf: &mut String, s: &str) {
    // Byte-scan: find the next char needing escape, bulk-copy the rest.
    // `{` and `}` are escaped so `{{...}}` inside code spans/blocks is not
    // interpreted as a WebUI signal binding by the template parser.
    let bytes = s.as_bytes();
    let mut start = 0;
    for (i, &b) in bytes.iter().enumerate() {
        let esc = match b {
            b'&' => "&amp;",
            b'<' => "&lt;",
            b'>' => "&gt;",
            b'{' => "&#123;",
            b'}' => "&#125;",
            _ => continue,
        };
        // SAFETY: all matched bytes are ASCII so byte indices are valid char boundaries.
        buf.push_str(&s[start..i]);
        buf.push_str(esc);
        start = i + 1;
    }
    buf.push_str(&s[start..]);
}

/// Map a highlight color from the InspiredGitHub theme to a semantic CSS class.
/// The actual display colors come from CSS custom properties — this mapping
/// just classifies tokens into semantic groups for theming.
fn scope_to_css_class(fg: syntect::highlighting::Color) -> &'static str {
    let hex = ((fg.r as u32) << 16) | ((fg.g as u32) << 8) | (fg.b as u32);
    match hex {
        // Keywords (if, else, return, import, export, const, let, fn, pub, use, etc.)
        0xa71d5d => "keyword",
        // Strings
        0x183691 => "string",
        // Comments
        0x969896 => "comment",
        // Numbers
        0xed6a43 => "number",
        // Tags (HTML)
        0x63a35c => "tag",
        // Attributes / decorators
        0x795da3 => "attr",
        // Properties / built-ins
        0x0086b3 => "property",
        // Functions
        0x6f42c1 => "function",
        // Types (in some themes)
        0x333333 => "variable",
        _ => "",
    }
}

/// Convert heading text to a URL-friendly slug.
fn slugify(text: &str) -> String {
    let mut slug = String::with_capacity(text.len());
    for ch in text.chars() {
        if ch.is_ascii_alphanumeric() {
            slug.push(ch.to_ascii_lowercase());
        } else if (ch == ' ' || ch == '-' || ch == '_') && !slug.ends_with('-') {
            slug.push('-');
        }
    }
    slug.trim_end_matches('-').to_string()
}

/// Collect text from a comrak node's children (iterative).
/// Returns `(plain_text, html_text)` — plain for slugs, HTML for display.
fn collect_node_text<'a>(
    node: &'a comrak::arena_tree::Node<'a, std::cell::RefCell<comrak::nodes::Ast>>,
) -> (String, String) {
    let mut plain = String::new();
    let mut html = String::new();
    let mut stack: Vec<&comrak::arena_tree::Node<'a, std::cell::RefCell<comrak::nodes::Ast>>> =
        Vec::new();
    // Push children in reverse for left-to-right traversal
    let mut child = node.last_child();
    while let Some(c) = child {
        stack.push(c);
        child = c.previous_sibling();
    }
    while let Some(n) = stack.pop() {
        let data = n.data.borrow();
        match &data.value {
            NodeValue::Text(t) => {
                plain.push_str(t);
                html.push_str(t);
            }
            NodeValue::Code(code) => {
                plain.push_str(&code.literal);
                html.push_str("<code>");
                // Escape HTML entities and template braces inside code spans
                // so `{{...}}` is not interpreted as a WebUI signal binding.
                emit_escaped(&mut html, &code.literal);
                html.push_str("</code>");
            }
            _ => {
                drop(data);
                let mut c = n.last_child();
                while let Some(ch) = c {
                    stack.push(ch);
                    c = ch.previous_sibling();
                }
            }
        }
    }
    (plain, html)
}

/// Render markdown content to HTML with syntax-highlighted code blocks.
/// Internal links (starting with `/`) are prefixed with `base_path`.
pub fn render_markdown(
    content: &str,
    highlighter: &Highlighter,
    base_path: &str,
) -> Result<String> {
    let arena = Arena::new();
    let mut options = Options::default();
    options.extension.table = true;
    options.extension.strikethrough = true;
    options.extension.autolink = true;
    options.render.r#unsafe = true; // Allow raw HTML passthrough

    let root = parse_document(&arena, content, &options);

    // Multi-pass approach: collect node pointers first, then modify.
    // This avoids modifying the tree during iteration and lets heading
    // rendering see original code-span nodes before they are converted to raw
    // HTML for WebUI template-signal escaping.

    // Pass 1: Rewrite internal links before custom heading rendering.
    for node in root.descendants() {
        let mut data = node.data.borrow_mut();
        if let NodeValue::Link(ref mut link) = data.value {
            // Prepend base_path to absolute internal links
            if link.url.starts_with('/') && !link.url.starts_with(base_path) && base_path != "/" {
                link.url = format!("{}{}", base_path.trim_end_matches('/'), &link.url);
            }
        }
    }

    // Pass 2: Collect heading nodes, then replace them
    let headings: Vec<_> = root
        .descendants()
        .filter(|node| matches!(node.data.borrow().value, NodeValue::Heading(_)))
        .collect();

    for node in headings {
        let (level, plain_text, html_text) = {
            let data = node.data.borrow();
            if let NodeValue::Heading(ref h) = data.value {
                let (plain, html) = collect_node_text(node);
                (h.level, plain, html)
            } else {
                continue;
            }
        };
        let slug = slugify(&plain_text);
        let html = format!(
            "<h{level} id=\"{slug}\">{html_text} <a class=\"header-anchor\" href=\"#{slug}\">#</a></h{level}>"
        );
        // Detach children first, then replace value. The heading is a
        // block-level node, so its raw-HTML replacement must be an
        // `HtmlBlock` (not `HtmlInline`) — comrak 0.53 validates that block
        // containers only hold block-level children.
        while let Some(child) = node.first_child() {
            child.detach();
        }
        node.data.borrow_mut().value = NodeValue::HtmlBlock(NodeHtmlBlock {
            block_type: 6,
            literal: html,
        });
    }

    // Pass 3: Replace code blocks and inline code after heading extraction,
    // so headings can still collect code-span literals for display and slugs.
    for node in root.descendants() {
        let mut data = node.data.borrow_mut();
        match &mut data.value {
            NodeValue::CodeBlock(ref mut block) => {
                let lang = if block.info.is_empty() {
                    "text"
                } else {
                    block.info.split_whitespace().next().unwrap_or("text")
                };
                let highlighted = highlighter.highlight_code(&block.literal, lang);
                // A code block is block-level: replace with an `HtmlBlock` so
                // comrak 0.53's AST validation accepts it as a block child.
                data.value = NodeValue::HtmlBlock(NodeHtmlBlock {
                    block_type: 6,
                    literal: highlighted,
                });
            }
            NodeValue::Code(ref code) => {
                // Replace inline code with pre-escaped HTML so `{{...}}` inside
                // code spans is not interpreted as a WebUI signal binding.
                let mut html = String::with_capacity(code.literal.len() + 13);
                html.push_str("<code>");
                emit_escaped(&mut html, &code.literal);
                html.push_str("</code>");
                data.value = NodeValue::HtmlInline(html);
            }
            _ => {}
        }
    }

    let mut html = String::with_capacity(content.len());
    comrak::format_html(root, &options, &mut html)
        .map_err(|e| crate::error::Error::Markdown(e.to_string()))?;

    Ok(html)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn syntect_language_mapping() {
        let ss = SyntaxSet::load_defaults_newlines();
        // Languages we need for the docs site
        for tok in &[
            "js", "go", "python", "py", "cpp", "rust", "rs", "html", "css", "json", "bash", "yaml",
            "cs",
        ] {
            assert!(
                ss.find_syntax_by_token(tok).is_some(),
                "syntect should support token '{tok}'"
            );
        }
        // These are NOT in syntect defaults — we alias them
        for tok in &["ts", "typescript", "tsx", "jsx", "toml", "csharp", "golang"] {
            assert!(
                ss.find_syntax_by_token(tok).is_none(),
                "'{tok}' should NOT be in syntect defaults (we alias it)"
            );
        }
    }

    #[test]
    fn inline_code_escapes_template_braces() {
        // `{{value}}` inside inline code (e.g. in markdown tables or prose)
        // must not be interpreted as a WebUI signal binding by the template
        // parser — escape `{` and `}` to HTML entities.
        let h = Highlighter::new();
        let html = render_markdown("Use `{{value}}` for escaped output.", &h, "/").unwrap();
        assert!(
            html.contains("&#123;&#123;value&#125;&#125;"),
            "inline code braces should be escaped: {html}"
        );
        assert!(
            !html.contains("{{value}}"),
            "raw `{{{{value}}}}` must not survive: {html}"
        );
    }

    #[test]
    fn heading_inline_code_survives_custom_anchor_rendering() {
        let h = Highlighter::new();
        let html = match render_markdown("# `<for>` Loop Directive\n", &h, "/") {
            Ok(html) => html,
            Err(e) => panic!("render_markdown should succeed: {e}"),
        };

        assert!(
            html.contains(
                r##"<h1 id="for-loop-directive"><code>&lt;for&gt;</code> Loop Directive"##
            ),
            "heading inline code should render as code: {html}"
        );
    }

    #[test]
    fn code_block_escapes_template_braces() {
        // Fenced code blocks must also escape braces so example template
        // snippets render literally instead of being parsed as bindings.
        let h = Highlighter::new();
        let html = render_markdown("```html\n<p>{{name}}</p>\n```\n", &h, "/").unwrap();
        assert!(
            html.contains("&#123;&#123;name&#125;&#125;"),
            "code block braces should be escaped: {html}"
        );
    }
}

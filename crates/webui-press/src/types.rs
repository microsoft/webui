// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

use serde::Deserialize;

/// Documentation site configuration (read from config.json).
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DocsConfig {
    pub site: SiteConfig,
    pub base_path: String,
    pub content_dir: String,
    #[serde(default = "default_out_dir")]
    pub out_dir: String,
    #[serde(default = "default_public_dir")]
    pub public_dir: String,
    pub css: Option<String>,
    pub components: Option<Vec<String>>,
    #[serde(default)]
    pub head: Vec<HeadTag>,
    pub nav: Vec<NavLink>,
    pub sidebar: Vec<SidebarSection>,
    #[serde(default)]
    pub sidebar_groups: std::collections::BTreeMap<String, Vec<SidebarSection>>,
    #[serde(default)]
    pub custom_pages: std::collections::HashMap<String, CustomPage>,
    pub hero: Option<HeroConfig>,
    pub footer: Option<FooterConfig>,
}

fn default_out_dir() -> String {
    "./dist".to_string()
}
fn default_public_dir() -> String {
    "./public".to_string()
}

#[derive(Debug, Deserialize)]
pub struct SiteConfig {
    pub title: String,
    #[serde(default)]
    pub description: String,
}

#[derive(Debug, Deserialize)]
pub struct NavLink {
    pub text: String,
    pub link: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct SidebarSection {
    pub title: String,
    pub items: Vec<SidebarItem>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct SidebarItem {
    pub text: String,
    #[serde(default)]
    pub link: String,
    #[serde(default)]
    pub items: Vec<SidebarItem>,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
pub enum CustomPage {
    Html(String),
    #[serde(rename_all = "camelCase")]
    Full {
        html: String,
        layout: Option<String>,
        /// Inline state object merged into the page's render state under the
        /// `pageData` key. Mutually exclusive with `state_file`.
        state: Option<serde_json::Value>,
        /// Path (relative to `config.json`) of a JSON file whose contents
        /// are merged into the page's render state under the `pageData` key.
        /// The build pipeline caches each unique file so that pages that share
        /// a state file only read it once. Mutually exclusive with `state`.
        state_file: Option<String>,
    },
}

impl CustomPage {
    pub fn html(&self) -> &str {
        match self {
            Self::Html(s) => s,
            Self::Full { html, .. } => html,
        }
    }

    pub fn layout(&self) -> &str {
        match self {
            Self::Html(_) => "doc",
            Self::Full { layout, .. } => layout.as_deref().unwrap_or("doc"),
        }
    }

    pub fn inline_state(&self) -> Option<&serde_json::Value> {
        match self {
            Self::Html(_) => None,
            Self::Full { state, .. } => state.as_ref(),
        }
    }

    pub fn state_file(&self) -> Option<&str> {
        match self {
            Self::Html(_) => None,
            Self::Full { state_file, .. } => state_file.as_deref(),
        }
    }
}

#[derive(Debug, Deserialize)]
pub struct HeroConfig {
    #[serde(default)]
    pub text: Option<String>,
    pub tagline: String,
    #[serde(default)]
    pub manifesto: Option<String>,
    pub actions: Vec<HeroAction>,
    pub features: Vec<HeroFeature>,
}

#[derive(Debug, Deserialize)]
pub struct HeroAction {
    pub text: String,
    pub link: String,
    #[serde(default)]
    pub brand: bool,
}

#[derive(Debug, Deserialize)]
pub struct HeroFeature {
    pub icon: String,
    pub title: String,
    pub description: String,
}

/// An HTML tag to inject into `<head>`.
///
/// JSON format: `{ "tag": "link", "attrs": { "rel": "icon", "href": "/favicon.ico" } }`
/// or with content: `{ "tag": "script", "attrs": { "type": "text/javascript" }, "content": "..." }`
#[derive(Debug, Deserialize)]
pub struct HeadTag {
    pub tag: String,
    #[serde(default)]
    pub attrs: std::collections::BTreeMap<String, String>,
    #[serde(default)]
    pub content: Option<String>,
}

impl HeadTag {
    /// Render this tag as an HTML string.
    pub fn to_html(&self) -> String {
        let mut html = String::with_capacity(128);
        html.push('<');
        html.push_str(&self.tag);
        for (key, value) in &self.attrs {
            html.push(' ');
            html.push_str(key);
            html.push_str("=\"");
            for ch in value.chars() {
                match ch {
                    '"' => html.push_str("&quot;"),
                    '&' => html.push_str("&amp;"),
                    '<' => html.push_str("&lt;"),
                    _ => html.push(ch),
                }
            }
            html.push('"');
        }
        html.push('>');

        if let Some(ref content) = self.content {
            html.push_str(content);
            html.push_str("</");
            html.push_str(&self.tag);
            html.push('>');
        } else if !is_void_element(&self.tag) {
            html.push_str("</");
            html.push_str(&self.tag);
            html.push('>');
        }

        html
    }
}

fn is_void_element(tag: &str) -> bool {
    matches!(
        tag,
        "link" | "meta" | "br" | "hr" | "img" | "input" | "base" | "col" | "embed" | "source"
    )
}

#[derive(Debug, Deserialize)]
pub struct FooterConfig {
    pub html: String,
}

/// A processed page ready for rendering.
pub struct PageDescriptor {
    pub path: String,
    pub is_home: bool,
    pub state: serde_json::Value,
}

/// Build output statistics.
#[allow(dead_code)] // Public lib API; the binary's main.rs does not consume the fields.
pub struct BuildStats {
    pub pages: usize,
    pub protocol_bytes: usize,
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    // --- HeadTag::to_html ------------------------------------------------

    fn tag(name: &str, attrs: &[(&str, &str)], content: Option<&str>) -> HeadTag {
        let mut map = BTreeMap::new();
        for (k, v) in attrs {
            map.insert((*k).to_string(), (*v).to_string());
        }
        HeadTag {
            tag: name.to_string(),
            attrs: map,
            content: content.map(String::from),
        }
    }

    #[test]
    fn head_tag_void_element_has_no_closing_tag() {
        let t = tag("link", &[("rel", "icon"), ("href", "/favicon.ico")], None);
        let html = t.to_html();
        assert!(html.starts_with("<link "), "got {html}");
        assert!(
            !html.contains("</link>"),
            "void element must not close: {html}"
        );
    }

    #[test]
    fn head_tag_non_void_self_closes() {
        let t = tag("script", &[("src", "/app.js")], None);
        let html = t.to_html();
        assert!(html.ends_with("</script>"), "got {html}");
    }

    #[test]
    fn head_tag_with_content_emits_inner() {
        let t = tag("script", &[], Some("console.log('x');"));
        assert_eq!(t.to_html(), "<script>console.log('x');</script>");
    }

    #[test]
    fn head_tag_attribute_order_is_deterministic() {
        // BTreeMap iterates in sorted key order; same input → same output every time.
        let t = tag(
            "meta",
            &[("zz", "last"), ("aa", "first"), ("mm", "mid")],
            None,
        );
        let html = t.to_html();
        let aa = html.find("aa=").expect("aa present");
        let mm = html.find("mm=").expect("mm present");
        let zz = html.find("zz=").expect("zz present");
        assert!(aa < mm && mm < zz, "expected sorted order, got {html}");
    }

    #[test]
    fn head_tag_escapes_attribute_values() {
        let t = tag("meta", &[("content", r#"a"b<c&d"#)], None);
        let html = t.to_html();
        assert!(html.contains("&quot;"), "{html}");
        assert!(html.contains("&lt;"), "{html}");
        assert!(html.contains("&amp;"), "{html}");
    }

    #[test]
    fn head_tag_multiple_runs_produce_identical_output() {
        // Regression: HashMap order non-determinism would have flunked this.
        let t1 = tag(
            "meta",
            &[("a", "1"), ("b", "2"), ("c", "3"), ("d", "4")],
            None,
        );
        let t2 = tag(
            "meta",
            &[("d", "4"), ("c", "3"), ("b", "2"), ("a", "1")],
            None,
        );
        assert_eq!(t1.to_html(), t2.to_html());
    }
}

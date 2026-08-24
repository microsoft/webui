// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

//! Component registry for WebUI framework.
//!
//! This module manages the registry of web components used in the application.

use crate::component_policy::{
    parse_component_render_policy, validate_policy_client_ownership, ComponentRenderPolicy,
};
use crate::plugin::{
    ComponentSource, ComponentSourceResult, ComponentSourceTransform, TransformedComponentSource,
};
use crate::{CssFallbackChain, CssParser, LegalComments, ParserError, Result};
use std::collections::{HashMap, HashSet};
#[cfg(feature = "fs")]
use std::fs;
#[cfg(feature = "fs")]
use std::path::Path;
#[cfg(feature = "fs")]
use walkdir::WalkDir;

type ProcessedCss = (String, Vec<String>, Vec<CssFallbackChain>);

/// Represents a web component in the registry.
///
/// ```
/// use webui_parser::Component;
///
/// let component = Component {
///     tag_name: "example-card".to_string(),
///     html_content: "<p>Example</p>".to_string(),
///     css_content: None,
///     css_definitions: Vec::new(),
///     css_fallback_chains: Vec::new(),
///     is_client_owned: false,
/// };
/// assert_eq!(component.tag_name, "example-card");
/// ```
#[derive(Debug, Clone)]
pub struct Component {
    /// The custom element tag name (e.g., "hello-world")
    pub tag_name: String,

    /// The HTML content of the component
    pub html_content: String,

    /// The CSS content of the component, if any
    pub css_content: Option<String>,

    /// CSS custom property definitions from this component's CSS.
    pub css_definitions: Vec<String>,

    /// CSS `var()` fallback chains from this component's CSS.
    pub css_fallback_chains: Vec<CssFallbackChain>,

    /// Whether authored browser code owns this custom element tag.
    pub is_client_owned: bool,
}

/// Inputs for registering a component from content strings.
///
/// Grouping the fields keeps [`ComponentRegistry::register_component`] a
/// single-argument call without growing its arity.
#[derive(Debug, Clone)]
pub struct ComponentRegistration<'a> {
    /// The custom element tag name (must contain a hyphen).
    pub tag_name: &'a str,
    /// The component's HTML template content.
    pub html_content: &'a str,
    /// The component's CSS content, if any.
    pub css_content: Option<&'a str>,
    /// Whether authored browser code owns this custom element tag.
    pub is_client_owned: bool,
}

impl<'a> ComponentRegistration<'a> {
    /// Create a component registration.
    #[must_use]
    pub fn new(
        tag_name: &'a str,
        html_content: &'a str,
        css_content: Option<&'a str>,
        is_client_owned: bool,
    ) -> Self {
        Self {
            tag_name,
            html_content,
            css_content,
            is_client_owned,
        }
    }
}

/// Registry of web components.
#[derive(Debug)]
pub struct ComponentRegistry {
    /// Map of component tag names to their component data
    components: HashMap<String, Component>,
    /// Compiler-owned rendering policies kept outside the public component API.
    render_policies: HashMap<String, ComponentRenderPolicy>,
    /// Components whose render policy CSS has been appended.
    policy_css: HashSet<String>,
    /// Authored client artifact sources retained by a component-source
    /// transform, keyed by resolved tag name. Framework-neutral: any plugin
    /// that owns a distinct client template populates it.
    component_artifact_sources: HashMap<String, String>,
    /// Optional plugin-supplied transform applied to each component's authored
    /// source before insertion. `None` stores sources unchanged.
    source_transform: Option<ComponentSourceTransform>,
    /// Reusable CSS parser for token extraction during registration.
    css_parser: CssParser,
    /// Legal comment preservation policy for component CSS.
    legal_comments: LegalComments,
}

#[cfg(feature = "fs")]
/// Return whether a component has an authored sibling module.
///
/// Use `try_exists()` rather than `exists()`: `exists()` converts metadata
/// errors into `false`, which could silently classify an inaccessible authored
/// component as scriptless and bypass projection-manifest coverage.
fn has_component_script(html_path: &Path) -> Result<bool> {
    for ext in ["ts", "js"] {
        let candidate = html_path.with_extension(ext);
        if candidate.try_exists().map_err(|source| ParserError::IO {
            context: format!(
                "Failed to inspect component script: {}",
                candidate.display()
            ),
            source,
        })? {
            return Ok(true);
        }
    }
    Ok(false)
}

impl Default for ComponentRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl ComponentRegistry {
    /// Create a new component registry.
    pub fn new() -> Self {
        Self::with_legal_comments(LegalComments::default())
    }

    pub(crate) fn with_legal_comments(legal_comments: LegalComments) -> Self {
        Self {
            components: HashMap::new(),
            render_policies: HashMap::new(),
            policy_css: HashSet::new(),
            component_artifact_sources: HashMap::new(),
            source_transform: None,
            css_parser: CssParser::new(),
            legal_comments,
        }
    }

    /// Install the component-source transform supplied by the active plugin.
    ///
    /// Passing `None` (the default) stores authored sources verbatim, so
    /// framework-specific markup is inert.
    pub(crate) fn set_component_source_transform(
        &mut self,
        transform: Option<ComponentSourceTransform>,
    ) {
        self.source_transform = transform;
    }

    /// Apply the installed source transform, borrowing the authored source and
    /// returning owned replacement views only when a transform fires.
    ///
    /// Returns `Some(TransformedComponentSource)` when the plugin transformed
    /// the source, or `None` when no transform is installed or it returned
    /// [`ComponentSourceResult::Unchanged`]. Borrowing the input means the
    /// caller allocates an owned copy of the authored HTML only when the source
    /// is preserved unchanged, never when it is replaced by transformed output.
    fn resolve_component_source(
        &self,
        tag_name: &str,
        html_content: &str,
    ) -> Result<Option<TransformedComponentSource>> {
        if let Some(transform) = self.source_transform {
            if let ComponentSourceResult::Transformed(transformed) = transform(ComponentSource {
                tag_name,
                html_content,
            })? {
                return Ok(Some(transformed));
            }
        }
        Ok(None)
    }

    /// Register multiple components from directories recursively.
    #[cfg(feature = "fs")]
    pub fn register_from_paths<P: AsRef<Path>>(&mut self, directories: &[P]) -> Result<&mut Self> {
        for dir in directories {
            for entry in WalkDir::new(dir.as_ref())
                .into_iter()
                .filter_map(|e| e.ok())
            {
                let path = entry.path();
                if !path.is_file() {
                    continue;
                }
                // Only process HTML files
                if !path.extension().is_some_and(|ext| ext == "html") {
                    continue;
                }
                let Some(filename) = path.file_stem().and_then(|s| s.to_str()) else {
                    continue;
                };
                // A filename-derived custom-element name (hyphenated stem) is
                // discoverable directly. Otherwise a plugin source transform
                // (e.g. FAST's `<f-template name>`) may still resolve a registry
                // name from the authored source, so admit the file only when a
                // transform is installed to claim it. With neither, keep the
                // zero-overhead fast path that ignores non-custom-element files
                // without reading them.
                if !filename.contains('-') && self.source_transform.is_none() {
                    continue;
                }
                let css_path = path.with_extension("css");
                let css_path = css_path.exists().then_some(css_path);
                // Discovery ignores a non-custom-element file the transform does
                // not claim, rather than failing the build over it.
                self.register_component_from_paths_inner(path, css_path.as_deref(), true)?;
            }
        }
        Ok(self)
    }

    /// Register a web component from paths to HTML and CSS files.
    #[cfg(feature = "fs")]
    pub fn register_component_from_paths<P: AsRef<Path>, Q: AsRef<Path>>(
        &mut self,
        html_path: P,
        css_path: Option<Q>,
    ) -> Result<()> {
        let css_path = css_path.as_ref().map(AsRef::as_ref);
        self.register_component_from_paths_inner(html_path.as_ref(), css_path, false)
    }

    /// Register a component from HTML/CSS paths, optionally skipping a file that
    /// no source transform claims.
    ///
    /// When `allow_skip_unresolved` is set (recursive discovery), a file whose
    /// source transform returns [`ComponentSourceResult::Unchanged`] and whose
    /// stem is not itself a custom-element name is silently ignored rather than
    /// rejected: it is not a component in the plugin's dialect. Explicit
    /// registration passes `false`, keeping the strict "must contain a hyphen"
    /// contract for an unresolved name.
    #[cfg(feature = "fs")]
    fn register_component_from_paths_inner(
        &mut self,
        html_path: &Path,
        css_path: Option<&Path>,
        allow_skip_unresolved: bool,
    ) -> Result<()> {
        // Extract component name from file name (without extension)
        let tag_name = html_path
            .file_stem()
            .and_then(|s| s.to_str())
            .ok_or_else(|| ParserError::Component("Invalid component file name".to_string()))?;

        // Read HTML content
        let html_content = fs::read_to_string(html_path).map_err(|source| ParserError::IO {
            context: format!("Failed to read HTML file: {}", html_path.display()),
            source,
        })?;
        let resolved = match self.resolve_component_source(tag_name, &html_content)? {
            Some(transformed) => transformed,
            None => {
                // No transform claimed the file. During discovery, a stem that
                // is not itself a custom-element name means this is not a
                // component in the active plugin's dialect, so ignore it.
                if allow_skip_unresolved && !tag_name.contains('-') {
                    return Ok(());
                }
                // Preserve the filename-derived tag and reuse the already-owned
                // file contents as the parser view without a second allocation.
                TransformedComponentSource {
                    tag_name: tag_name.to_string(),
                    parser_content: html_content,
                    artifact_content: None,
                }
            }
        };
        Self::validate_component_name(&resolved.tag_name)?;

        // Check for duplicate component
        if self.components.contains_key(&resolved.tag_name) {
            return Err(ParserError::Component(format!(
                "Component '{}' is already registered",
                resolved.tag_name
            )));
        }

        // Read CSS content and extract definitions/fallback requirements if available
        let (css_content, css_definitions, css_fallback_chains) = if let Some(css_path) = css_path {
            if css_path.exists() {
                let content = fs::read_to_string(css_path).map_err(|source| ParserError::IO {
                    context: format!("Failed to read CSS file: {}", css_path.display()),
                    source,
                })?;
                let (content, definitions, requirements) = self.process_css_content(&content)?;
                (Some(content), definitions, requirements)
            } else {
                (None, Vec::new(), Vec::new())
            }
        } else {
            (None, Vec::new(), Vec::new())
        };

        let is_client_owned = has_component_script(html_path)?;

        let render_policy =
            parse_component_render_policy(&resolved.tag_name, &resolved.parser_content)?;
        validate_policy_client_ownership(
            &resolved.tag_name,
            &resolved.parser_content,
            &render_policy,
            is_client_owned,
        )?;

        // Create and register the component
        let component = Component {
            tag_name: resolved.tag_name,
            html_content: resolved.parser_content,
            css_content,
            css_definitions,
            css_fallback_chains,
            is_client_owned,
        };

        self.render_policies
            .insert(component.tag_name.clone(), render_policy);
        if let Some(artifact) = resolved.artifact_content {
            self.component_artifact_sources
                .insert(component.tag_name.clone(), artifact);
        }
        self.components
            .insert(component.tag_name.clone(), component);
        Ok(())
    }

    /// Register a component directly from provided content strings.
    ///
    /// Exact JavaScript state ownership is supplied later by a bundler projection
    /// manifest; the parser stores only whether this tag is client-owned.
    pub fn register_component(&mut self, registration: ComponentRegistration<'_>) -> Result<()> {
        let ComponentRegistration {
            tag_name,
            html_content,
            css_content,
            is_client_owned,
        } = registration;

        let resolved = match self.resolve_component_source(tag_name, html_content)? {
            Some(transformed) => transformed,
            // No transform fired: allocate the owned parser view only now, not
            // eagerly before the transform had a chance to replace it.
            None => TransformedComponentSource {
                tag_name: tag_name.to_string(),
                parser_content: html_content.to_string(),
                artifact_content: None,
            },
        };
        Self::validate_component_name(&resolved.tag_name)?;

        // Check for duplicate component
        if self.components.contains_key(&resolved.tag_name) {
            return Err(ParserError::Component(format!(
                "Component '{}' is already registered",
                resolved.tag_name
            )));
        }

        // Extract CSS definitions/fallback requirements if CSS content is provided
        let (css_content, css_definitions, css_fallback_chains) = match css_content {
            Some(css) => {
                let (content, definitions, requirements) = self.process_css_content(css)?;
                (Some(content), definitions, requirements)
            }
            None => (None, Vec::new(), Vec::new()),
        };
        let render_policy =
            parse_component_render_policy(&resolved.tag_name, &resolved.parser_content)?;
        validate_policy_client_ownership(
            &resolved.tag_name,
            &resolved.parser_content,
            &render_policy,
            is_client_owned,
        )?;
        let component: Component = Component {
            tag_name: resolved.tag_name,
            html_content: resolved.parser_content,
            css_content,
            css_definitions,
            css_fallback_chains,
            is_client_owned,
        };

        // Register the component
        self.render_policies
            .insert(component.tag_name.clone(), render_policy);
        if let Some(artifact) = resolved.artifact_content {
            self.component_artifact_sources
                .insert(component.tag_name.clone(), artifact);
        }
        self.components
            .insert(component.tag_name.clone(), component);
        Ok(())
    }

    fn validate_component_name(tag_name: &str) -> Result<()> {
        if tag_name.contains('-') {
            return Ok(());
        }

        Err(ParserError::Component(format!(
            "Component name '{}' must contain a hyphen",
            tag_name
        )))
    }

    /// Strip comments and extract CSS definitions/fallback requirements.
    fn process_css_content(&mut self, css_content: &str) -> Result<ProcessedCss> {
        let (_tokens, definitions, requirements, stripped) = self
            .css_parser
            .extract_tokens_definitions_requirements_and_strip_comments(
                css_content,
                self.legal_comments,
            )?;
        let mut sorted_definitions: Vec<String> = definitions.into_iter().collect();
        sorted_definitions.sort();
        Ok((stripped.into_owned(), sorted_definitions, requirements))
    }

    /// Append the render policy CSS appropriate for the component's CSS tree.
    pub(crate) fn prepare_policy_css(
        &mut self,
        tag_name: &str,
        uses_shadow_dom: bool,
    ) -> Result<()> {
        let Some(policy) = self.render_policies.get(tag_name) else {
            return Err(ParserError::NotFound(format!(
                "component <{tag_name}> disappeared before render policy preparation"
            )));
        };
        if policy.reserve_block_size().is_none() || self.policy_css.contains(tag_name) {
            return Ok(());
        }
        let component_tag = {
            let component = self.components.get_mut(tag_name).ok_or_else(|| {
                ParserError::NotFound(format!(
                    "component <{tag_name}> disappeared before render policy preparation"
                ))
            })?;
            let css = component
                .css_content
                .get_or_insert_with(|| String::with_capacity(112));
            css.reserve(144);
            let mut escaped_tag = String::with_capacity(32);
            crate::css_light::push_css_identifier(&mut escaped_tag, component.tag_name.as_str());
            if uses_shadow_dom {
                policy.append_shadow_css(css, &escaped_tag);
            } else {
                policy.append_light_css(css, &escaped_tag);
            }
            component.tag_name.clone()
        };
        self.policy_css.insert(component_tag);
        Ok(())
    }

    /// Check if a tag name is registered as a component.
    pub fn contains(&self, tag_name: &str) -> bool {
        self.components.contains_key(tag_name)
    }

    /// Get a component by its tag name.
    pub fn get(&self, tag_name: &str) -> Option<&Component> {
        self.components.get(tag_name)
    }

    pub(crate) fn render_policy(&self, tag_name: &str) -> Option<&ComponentRenderPolicy> {
        self.render_policies.get(tag_name)
    }

    pub(crate) fn work_policies(&self) -> impl Iterator<Item = (&str, u8)> {
        self.render_policies
            .iter()
            .filter_map(|(tag, policy)| policy.metadata_code().map(|code| (tag.as_str(), code)))
    }

    /// Return component CSS for source diagnostics.
    pub(crate) fn diagnostic_css_content(&self, tag_name: &str) -> Option<&str> {
        self.components
            .get(tag_name)
            .and_then(|component| component.css_content.as_deref())
    }

    /// Get the retained authored client artifact source for a component.
    ///
    /// Populated only when a component-source transform returned a distinct
    /// artifact view; otherwise `None`.
    pub(crate) fn component_artifact_source(&self, tag_name: &str) -> Option<&str> {
        self.component_artifact_sources
            .get(tag_name)
            .map(String::as_str)
    }

    /// Get all registered components.
    pub fn get_all(&self) -> impl Iterator<Item = &Component> {
        self.components.values()
    }

    /// Build deterministic document-level CSS for the supplied component tags
    /// that opt into browser-managed lazy rendering.
    #[must_use]
    pub fn render_policy_css<'a>(&self, tag_names: impl IntoIterator<Item = &'a str>) -> String {
        let mut policies: Vec<(&str, &ComponentRenderPolicy)> = tag_names
            .into_iter()
            .filter_map(|tag_name| {
                self.render_policies
                    .get(tag_name)
                    .map(|policy| (tag_name, policy))
            })
            .filter(|(_, policy)| policy.reserve_block_size().is_some())
            .collect();
        policies.sort_unstable_by(|left, right| left.0.cmp(right.0));
        policies.dedup_by(|left, right| left.0 == right.0);

        let mut css = String::with_capacity(policies.len() * 112);
        for (tag_name, policy) in policies {
            crate::css_light::push_css_identifier(&mut css, tag_name);
            css.push_str(r#":not([w-render="eager"]){"#);
            policy.append_declarations(&mut css);
            css.push('}');
        }
        css
    }

    /// Iterate the registered component tag names (e.g. `mp-button`).
    ///
    /// Used to offer "did you mean …?" suggestions when an unknown component
    /// tag is encountered during parsing.
    pub fn names(&self) -> impl Iterator<Item = &str> {
        self.components.keys().map(String::as_str)
    }

    /// Get the number of registered components.
    pub fn len(&self) -> usize {
        self.components.len()
    }

    /// Check if the registry has no registered components.
    pub fn is_empty(&self) -> bool {
        self.components.is_empty()
    }
}

#[cfg(test)]
#[allow(clippy::disallowed_methods)]
mod tests {
    use super::*;
    use crate::codes;
    use crate::plugin::TransformedComponentSource;
    use webui_test_utils::TestFileSystem;

    #[test]
    fn test_register_component() {
        let html_content = "<p>Hello World</p>";
        let css_content = "p { color: red; }";

        // Create temporary files with proper names directly
        let mut fs = TestFileSystem::new();
        let html_path = fs.add_file("components/test-component.html", html_content);
        let css_path = fs.add_file("components/test-component.css", css_content);

        // Register the component (no rename needed)
        let mut registry = ComponentRegistry::new();
        let result = registry.register_component_from_paths(&html_path, Some(&css_path));

        assert!(result.is_ok());
        assert!(registry.contains("test-component"));

        let component = registry
            .get("test-component")
            .expect("Failed to retrieve registered component");
        assert_eq!(component.html_content, html_content);
        assert_eq!(component.css_content.as_deref(), Some(css_content));
        assert!(!component.is_client_owned);
    }

    #[test]
    #[cfg(feature = "fs")]
    fn transform_applies_to_path_registration() {
        let mut fs = TestFileSystem::new();
        let html_path = fs.add_file(
            "components/file-card.html",
            r#"<mock-template name="renamed-card"><template>x</template></mock-template>"#,
        );

        let mut registry = ComponentRegistry::new();
        registry.set_component_source_transform(Some(mock_rename_transform));
        registry
            .register_component_from_paths(&html_path, None::<&str>)
            .expect("register");

        assert!(!registry.contains("file-card"));
        let component = registry.get("renamed-card").expect("component");
        assert_eq!(
            component.html_content,
            "<template><span>parser</span></template>"
        );
        assert_eq!(
            registry.component_artifact_source("renamed-card"),
            Some("<template><span>artifact</span></template>")
        );
    }

    #[cfg(feature = "fs")]
    fn fast_transform() -> Option<crate::plugin::ComponentSourceTransform> {
        use crate::plugin::{fast_v2::FastV2ParserPlugin, ParserPlugin};
        FastV2ParserPlugin::new().component_source_transform()
    }

    #[test]
    #[cfg(feature = "fs")]
    fn discovery_registers_verbatim_fast_filename_via_transform() {
        // Fluent's committed filenames are `<component>.template.html`, whose
        // stem (`button.template`) is not itself a custom-element name. With a
        // FAST source transform installed, discovery still registers it under
        // the authored `<f-template name>`.
        let mut fs = TestFileSystem::new();
        let html = fs.add_file(
            "components/button.template.html",
            r#"<f-template name="fluent-button" shadowrootmode="open"><template @click="{clickHandler($e)}"><slot></slot></template></f-template>"#,
        );
        let mut registry = ComponentRegistry::new();
        registry.set_component_source_transform(fast_transform());
        registry
            .register_from_paths(&[html.parent().expect("dir")])
            .expect("discover");
        assert!(registry.contains("fluent-button"));
        assert!(!registry.contains("button.template"));
    }

    #[test]
    #[cfg(feature = "fs")]
    fn discovery_ignores_verbatim_fast_filename_without_transform() {
        // No plugin / the `webui` plugin installs no transform, so a
        // non-custom-element filename is ignored exactly as before — the FAST
        // `<f-template>` source is inert and never registered.
        let mut fs = TestFileSystem::new();
        let html = fs.add_file(
            "components/button.template.html",
            r#"<f-template name="fluent-button"><template><slot></slot></template></f-template>"#,
        );
        let mut registry = ComponentRegistry::new();
        registry
            .register_from_paths(&[html.parent().expect("dir")])
            .expect("discover");
        assert!(!registry.contains("fluent-button"));
        assert!(registry.get_all().next().is_none());
    }

    #[test]
    #[cfg(feature = "fs")]
    fn discovery_ignores_non_component_file_the_transform_does_not_claim() {
        // A non-custom-element filename whose source has no `<f-template>` is
        // not a FAST component; discovery ignores it rather than failing the
        // build over a non-hyphen name.
        let mut fs = TestFileSystem::new();
        let html = fs.add_file("components/partial.template.html", "<p>plain fragment</p>");
        let mut registry = ComponentRegistry::new();
        registry.set_component_source_transform(fast_transform());
        registry
            .register_from_paths(&[html.parent().expect("dir")])
            .expect("discover");
        assert!(registry.get_all().next().is_none());
    }

    #[test]
    #[cfg(feature = "fs")]
    fn discovery_still_registers_hyphen_stem_without_f_template() {
        // A hyphenated stem is a custom-element name on its own: even with a
        // transform installed, a plain (non-`<f-template>`) source registers
        // under the filename, unchanged from the no-transform path.
        let mut fs = TestFileSystem::new();
        let html = fs.add_file(
            "components/my-card.html",
            "<template><slot></slot></template>",
        );
        let mut registry = ComponentRegistry::new();
        registry.set_component_source_transform(fast_transform());
        registry
            .register_from_paths(&[html.parent().expect("dir")])
            .expect("discover");
        assert!(registry.contains("my-card"));
    }

    #[test]
    #[cfg(feature = "fs")]
    fn discovery_rejects_invalid_authored_name() {
        // A resolved `<f-template name>` that is not a valid custom-element name
        // is rejected rather than silently skipped.
        let mut fs = TestFileSystem::new();
        let html = fs.add_file(
            "components/widget.template.html",
            r#"<f-template name="notcustom"><template><slot></slot></template></f-template>"#,
        );
        let mut registry = ComponentRegistry::new();
        registry.set_component_source_transform(fast_transform());
        let err = registry
            .register_from_paths(&[html.parent().expect("dir")])
            .expect_err("invalid authored name should error");
        assert!(
            matches!(err, ParserError::Component(ref msg) if msg.contains("must contain a hyphen")),
            "unexpected error: {err:?}"
        );
    }

    #[test]
    #[cfg(feature = "fs")]
    fn discovery_rejects_authored_name_collision() {
        // Two files whose `<f-template name>` resolve to the same tag collide,
        // exactly like two identically named component files.
        let mut fs = TestFileSystem::new();
        let first = fs.add_file(
            "a/button.template.html",
            r#"<f-template name="fluent-button"><template><slot></slot></template></f-template>"#,
        );
        let second = fs.add_file(
            "b/toggle-button.template.html",
            r#"<f-template name="fluent-button"><template><slot></slot></template></f-template>"#,
        );
        let mut registry = ComponentRegistry::new();
        registry.set_component_source_transform(fast_transform());
        let err = registry
            .register_from_paths(&[first.parent().expect("dir"), second.parent().expect("dir")])
            .expect_err("colliding authored names should error");
        assert!(
            matches!(err, ParserError::Component(ref msg) if msg.contains("already registered")),
            "unexpected error: {err:?}"
        );
    }

    #[test]
    fn test_register_component_detects_ts_sibling_script() {
        let mut fs = TestFileSystem::new();
        let html_path = fs.add_file("components/scripted-card.html", "<p>Scripted</p>");
        std::fs::write(html_path.with_extension("ts"), "export {};")
            .expect("Failed to write sibling script");

        let mut registry = ComponentRegistry::new();
        registry
            .register_component_from_paths(&html_path, None::<&str>)
            .expect("register failed");

        let component = registry
            .get("scripted-card")
            .expect("Failed to retrieve registered component");
        assert!(component.is_client_owned);
    }

    #[test]
    fn test_register_component_detects_js_sibling_script() {
        let mut fs = TestFileSystem::new();
        let html_path = fs.add_file("components/scripted-card.html", "<p>Scripted</p>");
        std::fs::write(html_path.with_extension("js"), "export {};")
            .expect("Failed to write sibling script");

        let mut registry = ComponentRegistry::new();
        registry
            .register_component_from_paths(&html_path, None::<&str>)
            .expect("register failed");

        let component = registry
            .get("scripted-card")
            .expect("Failed to retrieve registered component");
        assert!(component.is_client_owned);
    }

    #[test]
    fn test_register_component_detects_non_utf8_sibling_without_reading_it() {
        let mut fs = TestFileSystem::new();
        let html_path = fs.add_file("components/scripted-card.html", "<p>Scripted</p>");
        std::fs::write(html_path.with_extension("ts"), [0xFF, 0xFE, 0x00])
            .expect("Failed to write invalid sibling script");

        let mut registry = ComponentRegistry::new();
        registry
            .register_component_from_paths(&html_path, None::<&str>)
            .expect("registration should inspect presence without decoding source");

        let component = registry
            .get("scripted-card")
            .expect("Failed to retrieve registered component");
        assert!(component.is_client_owned);
    }

    #[test]
    fn test_register_component_ignores_tsx_sibling_script() {
        let mut fs = TestFileSystem::new();
        let html_path = fs.add_file("components/scripted-card.html", "<p>Scripted</p>");
        std::fs::write(html_path.with_extension("tsx"), "export {};")
            .expect("Failed to write sibling script");

        let mut registry = ComponentRegistry::new();
        registry
            .register_component_from_paths(&html_path, None::<&str>)
            .expect("register failed");

        let component = registry
            .get("scripted-card")
            .expect("Failed to retrieve registered component");
        assert!(!component.is_client_owned);
    }

    #[test]
    fn test_component_name_validation() {
        let html_content = "<p>Invalid</p>";

        // Create temporary file with invalid name (no hyphen)
        let mut fs = TestFileSystem::new();
        let html_path = fs.add_file("invalid_name.html", html_content);

        // Try to register the component
        let mut registry = ComponentRegistry::new();
        let result = registry.register_component_from_paths(&html_path, None::<&str>);

        assert!(result.is_err());
        assert_eq!(registry.len(), 0);
    }

    #[test]
    fn test_missing_css_file() {
        let html_content = "<p>CSS Optional</p>";

        // Create temporary HTML file
        let mut fs = TestFileSystem::new();
        let html_path = fs.add_file("test-component.html", html_content);

        // Register with non-existent CSS file
        let mut registry = ComponentRegistry::new();
        let result = registry.register_component_from_paths(&html_path, None::<&str>);

        assert!(result.is_ok());
        let component = registry
            .get("test-component")
            .expect("Failed to retrieve registered component");
        assert_eq!(component.html_content, html_content);
        assert_eq!(component.css_content, None);
    }

    #[test]
    fn test_register_component_from_strings() {
        let html_content = "<p>Hello from string!</p>";
        let css_content = "p { color: green; }";

        // Register component directly from strings
        let mut registry = ComponentRegistry::new();
        let result = registry.register_component(ComponentRegistration::new(
            "string-component",
            html_content,
            Some(css_content),
            true,
        ));

        assert!(result.is_ok());
        assert!(registry.contains("string-component"));

        let component = registry
            .get("string-component")
            .expect("Failed to retrieve registered component");
        assert_eq!(component.html_content, html_content);
        assert_eq!(component.css_content.as_deref(), Some(css_content));
        assert!(component.is_client_owned);
    }

    #[test]
    fn lazy_render_policy_generates_stable_document_css() {
        let mut registry = ComponentRegistry::new();
        for (tag, size) in [("z-card", "18rem"), ("a-row", "72px")] {
            let html = format!(
                r#"<template w-render="lazy" w-reserve-block-size="{size}"><p>x</p></template>"#
            );
            registry
                .register_component(ComponentRegistration::new(tag, &html, None, true))
                .expect("valid lazy render policy");
        }

        assert_eq!(
            registry.render_policy_css(["z-card", "a-row"]),
            concat!(
                r#"a-row:not([w-render="eager"]){content-visibility:auto;contain-intrinsic-block-size:auto 72px;}"#,
                r#"z-card:not([w-render="eager"]){content-visibility:auto;contain-intrinsic-block-size:auto 18rem;}"#,
            )
        );
        assert_eq!(
            registry
                .get("a-row")
                .and_then(|component| component.css_content.as_deref()),
            None
        );
        registry
            .prepare_policy_css("a-row", true)
            .expect("prepare Shadow policy CSS");
        assert_eq!(
            registry
                .get("a-row")
                .and_then(|component| component.css_content.as_deref()),
            Some(
                r#":host(a-row:not([w-render="eager"])),a-row:not([w-render="eager"]){content-visibility:auto;contain-intrinsic-block-size:auto 72px;}"#,
            )
        );
    }

    #[test]
    fn lazy_render_policy_uses_a_normal_selector_for_light_css() {
        let mut registry = ComponentRegistry::new();
        registry
            .register_component(ComponentRegistration::new(
                "light-row",
                r#"<template w-render="lazy" w-reserve-block-size="72px"><p>x</p></template>"#,
                None,
                false,
            ))
            .expect("valid lazy render policy");

        registry
            .prepare_policy_css("light-row", false)
            .expect("prepare Light policy CSS");
        assert_eq!(
            registry
                .get("light-row")
                .and_then(|component| component.css_content.as_deref()),
            Some(
                r#"light-row:not([w-render="eager"]){content-visibility:auto;contain-intrinsic-block-size:auto 72px;}"#,
            )
        );
    }

    #[test]
    fn lazy_hydration_policy_emits_no_rendering_css() {
        let mut registry = ComponentRegistry::new();
        registry
            .register_component(ComponentRegistration::new(
                "lazy-row",
                r#"<template w-hydrate="lazy"><p>x</p></template>"#,
                None,
                true,
            ))
            .expect("valid hydration policy");
        assert!(registry.render_policy_css(["lazy-row"]).is_empty());
    }

    #[test]
    fn lazy_render_policy_requires_a_valid_reservation() {
        for (html, code) in [
            (
                r#"<template w-render="lazy"><p>x</p></template>"#,
                codes::MISSING_RENDER_RESERVATION,
            ),
            (
                r#"<template w-render="lazy" w-reserve-block-size="50%"><p>x</p></template>"#,
                codes::INVALID_RENDER_RESERVATION,
            ),
            (
                r#"<div w-render="lazy" w-reserve-block-size="10px"></div>"#,
                codes::INVALID_COMPONENT_RENDER_POLICY,
            ),
        ] {
            let mut registry = ComponentRegistry::new();
            let Err(ParserError::Template(diagnostic)) = registry
                .register_component(ComponentRegistration::new("bad-card", html, None, true))
            else {
                panic!("invalid component policy must produce a template diagnostic");
            };
            assert_eq!(diagnostic.error_code(), Some(code));
            assert!(diagnostic.help_text().is_some());
        }
    }

    #[test]
    fn test_register_component_strips_css_comments() {
        let mut registry = ComponentRegistry::new();
        registry
            .register_component(ComponentRegistration::new(
                "style-component",
                "<p>Styled</p>",
                Some("/* var(--ignored) */ p { color: var(--textColor); } /* remove */"),
                true,
            ))
            .expect("register failed");

        let component = registry
            .get("style-component")
            .expect("Failed to retrieve registered component");
        assert_eq!(
            component.css_content.as_deref(),
            Some(" p { color: var(--textColor); } ")
        );
        assert_eq!(component.css_fallback_chains.len(), 1);
    }

    #[test]
    fn test_register_component_preserves_legal_css_comments_by_default() {
        let mut registry = ComponentRegistry::new();
        registry
            .register_component(ComponentRegistration::new(
                "legal-component",
                "<p>Styled</p>",
                Some("/*! @license MIT */ p { color: red; } /* remove */"),
                true,
            ))
            .expect("register failed");

        let component = registry
            .get("legal-component")
            .expect("Failed to retrieve registered component");
        assert_eq!(
            component.css_content.as_deref(),
            Some("/*! @license MIT */ p { color: red; } ")
        );
    }

    #[test]
    fn test_register_component_strips_legal_css_comments_when_disabled() {
        let mut registry = ComponentRegistry::with_legal_comments(LegalComments::None);
        registry
            .register_component(ComponentRegistration::new(
                "legal-component",
                "<p>Styled</p>",
                Some("/*! @license MIT */ p { color: red; }"),
                true,
            ))
            .expect("register failed");

        let component = registry
            .get("legal-component")
            .expect("Failed to retrieve registered component");
        assert_eq!(component.css_content.as_deref(), Some(" p { color: red; }"));
    }

    #[test]
    fn test_invalid_component_name_from_strings() {
        let html_content = "<p>Invalid component</p>";

        // Try registering with invalid name (no hyphen)
        let mut registry = ComponentRegistry::new();
        let result = registry.register_component(ComponentRegistration::new(
            "invalid",
            html_content,
            None,
            true,
        ));

        // More idiomatic approach using assert!() with message
        assert!(result.is_err(), "Expected error for invalid component name");

        // Better pattern matching using matches!() macro
        assert!(
            matches!(result, Err(ParserError::Component(ref msg)) if msg.contains("must contain a hyphen")),
            "Wrong error type or message: {:?}",
            result
        );
    }

    #[test]
    fn test_duplicate_component_from_strings() {
        let html_content = "<p>First component</p>";
        let html_content2 = "<p>Second component with same name</p>";

        // Register the first component
        let mut registry = ComponentRegistry::new();
        let result1 = registry.register_component(ComponentRegistration::new(
            "dupe-component",
            html_content,
            None,
            true,
        ));
        assert!(result1.is_ok());

        // Try to register a second component with the same name
        let result2 = registry.register_component(ComponentRegistration::new(
            "dupe-component",
            html_content2,
            None,
            true,
        ));
        assert!(result2.is_err());

        // Verify the error message
        assert!(
            matches!(result2, Err(ParserError::Component(ref msg)) if msg.contains("already registered")),
            "Expected 'already registered' error, got: {:?}",
            result2
        );

        // Verify the original component is still there unchanged
        let component = registry
            .get("dupe-component")
            .expect("Failed to retrieve registered component");
        assert_eq!(component.html_content, html_content);
    }

    #[test]
    fn test_duplicate_component_from_paths() {
        let html_content1 = "<p>Component from dir A</p>";
        let html_content2 = "<p>Component from dir B</p>";

        // Create temporary directories and files.
        let mut fs = TestFileSystem::new();
        let file_path_a = fs.add_file("dir_a/my-comp.html", html_content1);
        let file_path_b = fs.add_file("dir_b/my-comp.html", html_content2);

        // Register the first component
        let mut registry = ComponentRegistry::new();
        let result1 = registry.register_component_from_paths(&file_path_a, None::<&str>);
        assert!(result1.is_ok());

        // Try to register the second component with the same name from a different path
        let result2 = registry.register_component_from_paths(&file_path_b, None::<&str>);
        assert!(result2.is_err());

        // Verify the error message
        assert!(
            matches!(result2, Err(ParserError::Component(ref msg)) if msg.contains("already registered")),
            "Expected 'already registered' error, got: {:?}",
            result2
        );

        // Verify the original component is still there unchanged
        let component = registry
            .get("my-comp")
            .expect("Failed to retrieve registered component");
        assert_eq!(component.html_content, html_content1);
    }

    fn mock_rename_transform(source: ComponentSource<'_>) -> Result<ComponentSourceResult> {
        if !source.html_content.contains("<mock-template") {
            return Ok(ComponentSourceResult::Unchanged);
        }
        Ok(ComponentSourceResult::Transformed(
            TransformedComponentSource {
                tag_name: "renamed-card".to_string(),
                parser_content: "<template><span>parser</span></template>".to_string(),
                artifact_content: Some("<template><span>artifact</span></template>".to_string()),
            },
        ))
    }

    fn mock_failing_transform(_source: ComponentSource<'_>) -> Result<ComponentSourceResult> {
        Err(ParserError::Component("mock transform failure".to_string()))
    }

    #[test]
    fn default_registry_leaves_authored_source_unchanged() {
        // Without an installed transform, framework-shaped markup is inert:
        // no renaming, conversion, or extra artifact source.
        let html = r#"<mock-template name="renamed-card"><template>x</template></mock-template>"#;
        let mut registry = ComponentRegistry::new();
        registry
            .register_component(ComponentRegistration::new("file-card", html, None, true))
            .expect("register");

        assert!(registry.contains("file-card"));
        assert!(!registry.contains("renamed-card"));
        assert_eq!(
            registry
                .get("file-card")
                .map(|component| component.html_content.as_str()),
            Some(html)
        );
        assert_eq!(registry.component_artifact_source("file-card"), None);
    }

    #[test]
    fn transform_renames_and_provides_parser_and_artifact_views() {
        let mut registry = ComponentRegistry::new();
        registry.set_component_source_transform(Some(mock_rename_transform));
        registry
            .register_component(ComponentRegistration::new(
                "file-card",
                r#"<mock-template name="renamed-card"><template>x</template></mock-template>"#,
                Some(".root { color: red; }"),
                true,
            ))
            .expect("register");

        assert!(!registry.contains("file-card"));
        let component = registry.get("renamed-card").expect("component");
        assert_eq!(component.tag_name, "renamed-card");
        assert_eq!(
            component.html_content,
            "<template><span>parser</span></template>"
        );
        assert_eq!(
            registry.component_artifact_source("renamed-card"),
            Some("<template><span>artifact</span></template>")
        );
    }

    #[test]
    fn transform_returning_unchanged_preserves_source() {
        let mut registry = ComponentRegistry::new();
        registry.set_component_source_transform(Some(mock_rename_transform));
        let html = r#"<template><span>{{title}}</span></template>"#;
        registry
            .register_component(ComponentRegistration::new("plain-card", html, None, true))
            .expect("register");

        assert!(registry.contains("plain-card"));
        assert_eq!(
            registry
                .get("plain-card")
                .map(|component| component.html_content.as_str()),
            Some(html)
        );
        assert_eq!(registry.component_artifact_source("plain-card"), None);
    }

    #[test]
    fn transform_error_is_transactional() {
        let mut registry = ComponentRegistry::new();
        registry.set_component_source_transform(Some(mock_failing_transform));
        let err = registry
            .register_component(ComponentRegistration::new(
                "file-card",
                "<template>x</template>",
                None,
                true,
            ))
            .expect_err("failing transform should abort registration");

        assert!(matches!(err, ParserError::Component(ref msg) if msg.contains("mock transform")));
        assert!(!registry.contains("file-card"));
        assert!(registry.is_empty());
    }

    #[test]
    fn transform_duplicate_resolved_name_is_rejected() {
        let mut registry = ComponentRegistry::new();
        registry.set_component_source_transform(Some(mock_rename_transform));
        registry
            .register_component(ComponentRegistration::new(
                "first-card",
                r#"<mock-template name="renamed-card"><template>x</template></mock-template>"#,
                None,
                true,
            ))
            .expect("first register");
        let err = registry
            .register_component(ComponentRegistration::new(
                "second-card",
                r#"<mock-template name="renamed-card"><template>y</template></mock-template>"#,
                None,
                true,
            ))
            .expect_err("duplicate resolved name should error");

        assert!(
            matches!(err, ParserError::Component(ref msg) if msg.contains("already registered"))
        );
    }

    #[test]
    fn test_exclude_dot_in_component_name() {
        let mut registry = ComponentRegistry::new();
        let result = registry.register_component(ComponentRegistration::new(
            "fluent.button",
            "<p>Dot name</p>",
            None,
            true,
        ));

        assert!(
            result.is_err(),
            "Component name with dot but no hyphen should be rejected"
        );
        assert!(
            matches!(result, Err(ParserError::Component(ref msg)) if msg.contains("must contain a hyphen")),
            "Wrong error type or message: {:?}",
            result
        );
        assert_eq!(registry.len(), 0);
    }

    #[test]
    fn test_exclude_no_hyphen_html() {
        let mut registry = ComponentRegistry::new();
        let result = registry.register_component(ComponentRegistration::new(
            "foobar",
            "<p>No hyphen</p>",
            None,
            true,
        ));

        assert!(
            result.is_err(),
            "Component name without hyphen should be rejected"
        );
        assert!(
            matches!(result, Err(ParserError::Component(ref msg)) if msg.contains("must contain a hyphen")),
            "Wrong error type or message: {:?}",
            result
        );
        assert_eq!(registry.len(), 0);
    }

    #[test]
    fn test_valid_component_with_hyphen() {
        let mut registry = ComponentRegistry::new();
        let result = registry.register_component(ComponentRegistration::new(
            "fluent-button",
            "<button>Click me</button>",
            None,
            true,
        ));

        assert!(
            result.is_ok(),
            "Component name with hyphen should be accepted"
        );
        assert!(registry.contains("fluent-button"));
        assert_eq!(registry.len(), 1);
    }

    #[test]
    fn test_valid_component_css_only() {
        let mut registry = ComponentRegistry::new();
        let result = registry.register_component(ComponentRegistration::new(
            "styled-widget",
            "",
            Some(".widget { color: blue; }"),
            true,
        ));

        assert!(
            result.is_ok(),
            "Component with empty HTML and CSS should be accepted"
        );
        assert!(registry.contains("styled-widget"));

        let component = registry
            .get("styled-widget")
            .expect("Failed to retrieve registered component");
        assert_eq!(component.html_content, "");
        assert_eq!(
            component.css_content.as_deref(),
            Some(".widget { color: blue; }")
        );
    }

    #[test]
    fn test_component_name_requires_hyphen() {
        let mut registry = ComponentRegistry::new();
        let result = registry.register_component(ComponentRegistration::new(
            "single",
            "<p>Single word</p>",
            None,
            true,
        ));

        assert!(
            result.is_err(),
            "Single-word component name should be rejected"
        );
        assert!(
            matches!(result, Err(ParserError::Component(ref msg)) if msg.contains("must contain a hyphen")),
            "Wrong error type or message: {:?}",
            result
        );
        assert_eq!(registry.len(), 0);
    }

    #[test]
    fn test_multiple_hyphens_valid() {
        let mut registry = ComponentRegistry::new();
        let result = registry.register_component(ComponentRegistration::new(
            "my-custom-element",
            "<div>Custom element</div>",
            None,
            true,
        ));

        assert!(
            result.is_ok(),
            "Component name with multiple hyphens should be accepted"
        );
        assert!(registry.contains("my-custom-element"));
        assert_eq!(registry.len(), 1);
    }

    #[test]
    fn test_empty_component_name_rejected() {
        let mut registry = ComponentRegistry::new();
        let result = registry.register_component(ComponentRegistration::new(
            "",
            "<p>Empty name</p>",
            None,
            true,
        ));

        assert!(result.is_err(), "Empty component name should be rejected");
        assert!(
            matches!(result, Err(ParserError::Component(ref msg)) if msg.contains("must contain a hyphen")),
            "Wrong error type or message: {:?}",
            result
        );
        assert_eq!(registry.len(), 0);
    }

    #[test]
    fn test_register_component_extracts_css_fallback_requirements() {
        let mut registry = ComponentRegistry::new();
        registry
            .register_component(ComponentRegistration::new(
                "my-btn",
                "<button>Click</button>",
                Some(":host { color: var(--text-color); padding: var(--spacing-m); }"),
                true,
            ))
            .expect("register failed");

        let component = registry.get("my-btn").expect("component not found");
        assert_eq!(component.css_fallback_chains.len(), 2);
    }

    #[test]
    fn test_register_component_no_css_no_requirements() {
        let mut registry = ComponentRegistry::new();
        registry
            .register_component(ComponentRegistration::new(
                "my-card",
                "<div>Card</div>",
                None,
                true,
            ))
            .expect("register failed");

        let component = registry.get("my-card").expect("component not found");
        assert!(component.css_fallback_chains.is_empty());
    }

    #[test]
    fn test_register_component_tracks_css_fallback_requirements() {
        let mut registry = ComponentRegistry::new();
        registry
            .register_component(ComponentRegistration::new(
                "my-widget",
                "<div>W</div>",
                Some(":host { --local: 5px; margin: var(--external); width: var(--local); }"),
                true,
            ))
            .expect("register failed");

        let component = registry.get("my-widget").expect("component not found");
        assert_eq!(component.css_fallback_chains.len(), 2);
    }
}

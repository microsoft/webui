// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

//! Validation and diagnostics for component module mappings.
//!
//! Lazy regions load the code of the custom elements they introduce through
//! the application's bundler. A component's module is its discovered `.ts` or
//! `.js` sibling, or an explicit mapping from its tag to a module specifier
//! (`--component-module`, `componentModules`) for packages without one.

use crate::{codes, suggest, Diagnostic};
use std::path::Path;

/// Validate one explicit mapping from a custom element tag to a module specifier.
///
/// The tag must have the form of a custom element name (see
/// [`is_custom_element_name`]). The specifier is written verbatim as a dynamic
/// import, so it must be non-empty and contain no whitespace, control
/// characters, quotes, or backslashes.
///
/// # Errors
///
/// Returns an [`codes::INVALID_COMPONENT_MODULE`] diagnostic naming the
/// rejected part.
pub fn validate_component_module(tag: &str, specifier: &str) -> Result<(), Box<Diagnostic>> {
    if !is_custom_element_name(tag) {
        return Err(invalid_tag(tag, specifier));
    }
    if !is_module_specifier(specifier) {
        return Err(invalid_specifier(tag, specifier));
    }
    Ok(())
}

/// Whether `tag` has the form of a custom element name: a lowercase ASCII
/// letter followed by lowercase letters, digits, `-`, `.`, or `_`, with at
/// least one `-`.
#[must_use]
pub fn is_custom_element_name(tag: &str) -> bool {
    let bytes = tag.as_bytes();
    bytes.first().is_some_and(u8::is_ascii_lowercase)
        && bytes.contains(&b'-')
        && bytes.iter().all(|byte| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || matches!(byte, b'-' | b'.' | b'_')
        })
}

fn is_module_specifier(specifier: &str) -> bool {
    !specifier.is_empty()
        && !specifier.chars().any(|ch| {
            ch.is_whitespace() || ch.is_control() || matches!(ch, '"' | '\'' | '`' | '\\')
        })
}

#[cold]
#[inline(never)]
fn invalid_tag(tag: &str, specifier: &str) -> Box<Diagnostic> {
    Box::new(
        Diagnostic::error(format!("invalid component module tag '{tag}'"))
            .code(codes::INVALID_COMPONENT_MODULE)
            .snippet(mapping(tag, specifier))
            .help(
                "map a custom element name: lowercase letters, digits, '-', '.', or '_', starting with a letter and containing '-', for example settings-panel=@contoso/ui/settings-panel.js",
            ),
    )
}

#[cold]
#[inline(never)]
fn invalid_specifier(tag: &str, specifier: &str) -> Box<Diagnostic> {
    Box::new(
        Diagnostic::error(format!("invalid module specifier for <{tag}>"))
            .code(codes::INVALID_COMPONENT_MODULE)
            .snippet(mapping(tag, specifier))
            .help(
                "use a package specifier or a path relative to the generated-modules directory, without spaces, quotes, or backslashes, for example @contoso/ui/settings-panel.js or ../src/settings-panel/settings-panel.ts",
            ),
    )
}

fn mapping(tag: &str, specifier: &str) -> String {
    let mut text = String::with_capacity(tag.len() + specifier.len() + 1);
    text.push_str(tag);
    text.push('=');
    text.push_str(specifier);
    text
}

/// Warning for a mapping whose tag no registered component uses.
///
/// `known` lists registered tags in a stable order; the closest one becomes a
/// "did you mean" suggestion.
#[cold]
#[inline(never)]
#[must_use]
pub fn unknown_component_module<'a>(
    tag: &str,
    specifier: &str,
    known: impl Iterator<Item = &'a str>,
) -> Diagnostic {
    let help = match suggest::closest_match(tag, known) {
        Some(name) => format!(
            "did you mean <{name}>? A mapping only applies to a component the build discovers; fix the tag or remove the mapping"
        ),
        None => "a mapping only applies to a component the build discovers; fix the tag or remove the mapping".to_owned(),
    };
    Diagnostic::warning(format!(
        "component module mapping for <{tag}> matches no component"
    ))
    .code(codes::UNKNOWN_COMPONENT_MODULE)
    .snippet(mapping(tag, specifier))
    .help(help)
}

/// Warning for a client-owned component that lazy regions introduce but whose
/// module is unknown, so the generated `lazy.webui.js` cannot load it.
#[cold]
#[inline(never)]
#[must_use]
pub fn lazy_component_without_module(tag: &str, regions: &[&str]) -> Diagnostic {
    let plural = if regions.len() == 1 { "" } else { "s" };
    Diagnostic::warning(format!(
        "<{tag}> has client code but no known module, so lazy region{plural} {} cannot load it",
        regions.join(", ")
    ))
    .code(codes::LAZY_COMPONENT_WITHOUT_MODULE)
    .component(tag)
    .help(format!(
        "add --component-module {tag}=<specifier> (componentModules in the Node and WASM APIs) so lazy.webui.js imports it with the region, or import it statically if it must be defined at startup"
    ))
}

/// Error for a component module with no path relative to the directory that
/// receives `lazy.webui.js`.
#[cold]
#[inline(never)]
#[must_use]
pub fn unreachable_component_module(tag: &str, module: &Path, modules_dir: &Path) -> Diagnostic {
    Diagnostic::error(format!(
        "the module for <{tag}> has no path relative to the generated-modules directory"
    ))
    .code(codes::UNREACHABLE_COMPONENT_MODULE)
    .component(tag)
    .snippet(format!(
        "{} (from {})",
        module.display(),
        modules_dir.display()
    ))
    .help(format!(
        "write generated modules on the same drive as the component with --modules-out (modulesOut), or map it to a package specifier with --component-module {tag}=<specifier>"
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mappings_need_custom_element_names_and_plain_specifiers() {
        for (tag, specifier) in [
            ("settings-panel", "../src/settings-panel/settings-panel.ts"),
            ("x-card", "@contoso/ui/x-card.js"),
            ("my.widget-2_b", "./widget.js"),
        ] {
            assert!(validate_component_module(tag, specifier).is_ok(), "{tag}");
        }
        for tag in [
            "",
            "card",
            "Settings-panel",
            "1-card",
            "-card",
            "x card",
            "x-Card",
            "x-c\u{e9}",
        ] {
            let error = validate_component_module(tag, "./x.js").unwrap_err();
            assert_eq!(
                error.error_code(),
                Some(codes::INVALID_COMPONENT_MODULE),
                "{tag}"
            );
            assert!(error.help_text().is_some());
        }
        for specifier in [
            "",
            "./a b.js",
            "./a\"b.js",
            "./a'b.js",
            ".\\a.js",
            "./a\tb.js",
            "./a`b.js",
        ] {
            let error = validate_component_module("x-card", specifier).unwrap_err();
            assert_eq!(
                error.error_code(),
                Some(codes::INVALID_COMPONENT_MODULE),
                "{specifier}"
            );
            assert!(error.title().contains("specifier"), "{specifier}");
        }
    }

    #[test]
    fn warnings_carry_stable_codes_and_actionable_help() {
        let unknown = unknown_component_module(
            "settings-panl",
            "./settings.js",
            ["app-shell", "settings-panel"].into_iter(),
        );
        assert_eq!(unknown.error_code(), Some(codes::UNKNOWN_COMPONENT_MODULE));
        assert_eq!(unknown.severity(), crate::Severity::Warning);
        assert!(unknown
            .help_text()
            .unwrap_or_default()
            .contains("did you mean <settings-panel>"));

        let missing = lazy_component_without_module("x-card", &["app-shell::lazy:@0"]);
        assert_eq!(
            missing.error_code(),
            Some(codes::LAZY_COMPONENT_WITHOUT_MODULE)
        );
        assert_eq!(missing.severity(), crate::Severity::Warning);
        let help = missing.help_text().unwrap_or_default();
        assert!(
            help.contains("--component-module x-card=<specifier>"),
            "{help}"
        );
        assert!(help.contains("componentModules"), "{help}");
        assert!(help.contains("import it statically"), "{help}");

        let unreachable = unreachable_component_module(
            "x-card",
            Path::new("D:/lib/x-card.js"),
            Path::new("C:/app/dist"),
        );
        assert_eq!(
            unreachable.error_code(),
            Some(codes::UNREACHABLE_COMPONENT_MODULE)
        );
        assert_eq!(unreachable.severity(), crate::Severity::Error);
        assert!(unreachable
            .help_text()
            .unwrap_or_default()
            .contains("--modules-out"));
    }
}

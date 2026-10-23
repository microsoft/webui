// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

pub(crate) use webui_protocol::{lazy_block_id as block_id, lazy_fragment_id as fragment_id};

pub(crate) fn if_attribute_error(tag: &crate::html_parser::Tag<'_>) -> Option<crate::Diagnostic> {
    let invalid = tag.attrs().find(|attr| match attr.name {
        "condition" | "lazy" => false,
        "id" => !tag.has_attr("lazy"),
        _ => true,
    })?;
    Some(invalid_if_attribute(invalid.name))
}

#[cold]
#[inline(never)]
fn invalid_if_attribute(name: &str) -> crate::Diagnostic {
    crate::Diagnostic::error(format!("unsupported attribute '{name}' on <if>"))
        .code(crate::codes::INVALID_IF_ATTRIBUTE)
        .element("if")
        .snippet(name)
        .help("use condition, optional boolean lazy, and id only for a lazy region")
}

pub(crate) fn valid_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 128
        && name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn named_and_anonymous_regions_have_distinct_owner_scoped_identities() {
        assert_eq!(
            fragment_id("app-shell", Some("settings"), 0),
            "app-shell::lazy:settings"
        );
        assert_eq!(fragment_id("app-shell", None, 2), "app-shell::lazy:@2");
        assert_eq!(block_id("app-shell", 3), "app-shell::block:3");
        assert_ne!(
            fragment_id("first-owner", Some("settings"), 0),
            fragment_id("second-owner", Some("settings"), 0)
        );
    }

    #[test]
    fn names_are_static_bounded_identifiers() {
        for name in ["settings", "settings-panel", "account.settings", "panel_2"] {
            assert!(valid_name(name));
        }
        for name in ["", "@0", "{{name}}", "two words", "x/y"] {
            assert!(!valid_name(name));
        }
        assert!(!valid_name(&"x".repeat(129)));
    }
}

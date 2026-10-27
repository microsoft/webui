// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

pub(crate) use webui_protocol::{lazy_block_id as block_id, lazy_fragment_id as fragment_id};

/// When a lazy region's template pack may be fetched before activation.
///
/// The numeric value is the client metadata encoding.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Trigger {
    /// Pointer, focus, or touch intent inside the owning component (default).
    Intent = 1,
    /// The first idle period after the owner mounts.
    Idle = 2,
    /// As soon as the owner mounts.
    Load = 3,
    /// Only when the condition becomes true.
    Demand = 4,
}

const TRIGGERS: [(&str, Trigger); 4] = [
    ("intent", Trigger::Intent),
    ("idle", Trigger::Idle),
    ("load", Trigger::Load),
    ("demand", Trigger::Demand),
];

impl Trigger {
    /// Parse the `lazy` attribute value; a bare attribute means [`Trigger::Intent`].
    pub(crate) fn parse(value: Option<&str>) -> Option<Self> {
        let value = value.unwrap_or_default();
        if value.is_empty() {
            return Some(Self::Intent);
        }
        TRIGGERS
            .iter()
            .find_map(|(name, trigger)| (*name == value).then_some(*trigger))
    }

    /// Metadata code consumed by the client runtime.
    pub(crate) fn code(self) -> u8 {
        self as u8
    }
}

pub(crate) fn if_attribute_error(tag: &crate::html_parser::Tag<'_>) -> Option<crate::Diagnostic> {
    let invalid = tag
        .attrs()
        .find(|attr| !matches!(attr.name, "condition" | "lazy"))?;
    Some(invalid_if_attribute(invalid.name))
}

#[cold]
#[inline(never)]
fn invalid_if_attribute(name: &str) -> crate::Diagnostic {
    let help = if name == "id" {
        "remove id: the framework loads a lazy region when its condition becomes true"
    } else {
        "use condition and an optional lazy, lazy=\"idle\", lazy=\"load\", or lazy=\"demand\""
    };
    crate::Diagnostic::error(format!("unsupported attribute '{name}' on <if>"))
        .code(crate::codes::INVALID_IF_ATTRIBUTE)
        .element("if")
        .snippet(name)
        .help(help)
}

/// Build the diagnostic for an unknown `lazy` trigger (cold path).
#[cold]
#[inline(never)]
pub(crate) fn invalid_trigger(value: &str) -> crate::Diagnostic {
    let help = match crate::suggest::closest_match(value, TRIGGERS.iter().map(|(name, _)| *name)) {
        Some(name) => {
            format!("did you mean lazy=\"{name}\"? Use intent (the default), idle, load, or demand")
        }
        None => "use lazy (prefetch on intent), lazy=\"idle\", lazy=\"load\", or lazy=\"demand\""
            .to_owned(),
    };
    crate::Diagnostic::error(format!("unsupported lazy trigger '{value}'"))
        .code(crate::codes::INVALID_LAZY_ATTRIBUTE)
        .element("if")
        .snippet(format!("lazy=\"{value}\""))
        .help(help)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn regions_have_owner_scoped_build_local_identities() {
        assert_eq!(fragment_id("app-shell", 2), "app-shell::lazy:@2");
        assert_eq!(block_id("app-shell", 3), "app-shell::block:3");
        assert_ne!(
            fragment_id("first-owner", 0),
            fragment_id("second-owner", 0)
        );
    }

    #[test]
    fn triggers_default_to_intent_and_reject_unknown_values() {
        assert_eq!(Trigger::parse(None), Some(Trigger::Intent));
        assert_eq!(Trigger::parse(Some("")), Some(Trigger::Intent));
        assert_eq!(Trigger::parse(Some("intent")), Some(Trigger::Intent));
        assert_eq!(Trigger::parse(Some("idle")), Some(Trigger::Idle));
        assert_eq!(Trigger::parse(Some("load")), Some(Trigger::Load));
        assert_eq!(Trigger::parse(Some("demand")), Some(Trigger::Demand));
        for value in ["true", "Idle", "{{trigger}}", "hover"] {
            assert_eq!(Trigger::parse(Some(value)), None, "{value}");
        }
        assert_eq!(
            [
                Trigger::Intent,
                Trigger::Idle,
                Trigger::Load,
                Trigger::Demand
            ]
            .map(Trigger::code),
            [1, 2, 3, 4]
        );
    }

    #[test]
    fn trigger_typos_suggest_the_closest_value() {
        let diagnostic = invalid_trigger("idel").to_string();
        assert!(
            diagnostic.contains("did you mean lazy=\"idle\""),
            "{diagnostic}"
        );
        let diagnostic = invalid_trigger("hover").to_string();
        assert!(diagnostic.contains("lazy=\"demand\""), "{diagnostic}");
    }
}

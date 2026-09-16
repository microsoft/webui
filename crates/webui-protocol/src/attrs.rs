// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

//! Attribute-name ↔ property-name mapping for irregular HTML attributes.
//!
//! Some HTML attributes use concatenated lowercase names that do not follow
//! standard camelCase-to-kebab-case conversion rules. This module provides
//! lookup tables covering two categories:
//!
//! 1. **Multi-word ARIA attributes** — e.g., `aria-describedby` ↔
//!    `ariaDescribedBy`, per the [ARIAMixin] specification.
//! 2. **HTML global/element attributes** — e.g., `readonly` ↔ `readOnly`,
//!    `tabindex` ↔ `tabIndex`.
//!
//! Both directions are generated from a single list via
//! [`define_attr_property_mappings!`] so they cannot drift.
//!
//! [ARIAMixin]: https://w3c.github.io/aria/#ARIAMixin

/// Define bidirectional property ↔ attribute lookup functions from a single
/// list of `(property, attribute)` pairs. This guarantees the two match
/// statements stay in sync — adding or removing an entry in one direction
/// automatically applies to the other.
macro_rules! define_attr_property_mappings {
    ($( $property:literal => $attribute:literal, )*) => {
        /// Map a camelCase property name to its HTML attribute.
        ///
        /// Returns `None` for names that follow standard camelCase ↔ kebab
        /// conversion.
        #[must_use]
        pub fn property_to_attribute(name: &str) -> Option<&'static str> {
            match name {
                $( $property => Some($attribute), )*
                _ => None,
            }
        }

        /// Map an HTML attribute to its camelCase property name.
        ///
        /// Inverse of [`property_to_attribute`].
        #[must_use]
        pub fn attribute_to_property(name: &str) -> Option<&'static str> {
            match name {
                $( $attribute => Some($property), )*
                _ => None,
            }
        }

        /// All `(property, attribute)` pairs for exhaustive testing.
        #[cfg(test)]
        const ALL_MAPPINGS: &[(&str, &str)] = &[
            $( ($property, $attribute), )*
        ];
    };
}

define_attr_property_mappings! {
    // --- ARIA (ARIAMixin) ---
    "ariaActiveDescendant" => "aria-activedescendant",
    "ariaAutoComplete" => "aria-autocomplete",
    "ariaBrailleLabel" => "aria-braillelabel",
    "ariaBrailleRoleDescription" => "aria-brailleroledescription",
    "ariaColCount" => "aria-colcount",
    "ariaColIndex" => "aria-colindex",
    "ariaColIndexText" => "aria-colindextext",
    "ariaColSpan" => "aria-colspan",
    "ariaDescribedBy" => "aria-describedby",
    "ariaDropEffect" => "aria-dropeffect",
    "ariaErrorMessage" => "aria-errormessage",
    "ariaFlowTo" => "aria-flowto",
    "ariaHasPopup" => "aria-haspopup",
    "ariaKeyShortcuts" => "aria-keyshortcuts",
    "ariaLabelledBy" => "aria-labelledby",
    "ariaMultiLine" => "aria-multiline",
    "ariaMultiSelectable" => "aria-multiselectable",
    "ariaPosInSet" => "aria-posinset",
    "ariaReadOnly" => "aria-readonly",
    "ariaRoleDescription" => "aria-roledescription",
    "ariaRowCount" => "aria-rowcount",
    "ariaRowIndex" => "aria-rowindex",
    "ariaRowIndexText" => "aria-rowindextext",
    "ariaRowSpan" => "aria-rowspan",
    "ariaSetSize" => "aria-setsize",
    "ariaValueMax" => "aria-valuemax",
    "ariaValueMin" => "aria-valuemin",
    "ariaValueNow" => "aria-valuenow",
    "ariaValueText" => "aria-valuetext",
    // --- HTML global/element attributes ---
    "accessKey" => "accesskey",
    "autoCapitalize" => "autocapitalize",
    "contentEditable" => "contenteditable",
    "crossOrigin" => "crossorigin",
    "dirName" => "dirname",
    "fetchPriority" => "fetchpriority",
    "formAction" => "formaction",
    "formEnctype" => "formenctype",
    "formMethod" => "formmethod",
    "formNoValidate" => "formnovalidate",
    "formTarget" => "formtarget",
    "inputMode" => "inputmode",
    "isMap" => "ismap",
    "maxLength" => "maxlength",
    "minLength" => "minlength",
    "noModule" => "nomodule",
    "noValidate" => "novalidate",
    "readOnly" => "readonly",
    "referrerPolicy" => "referrerpolicy",
    "tabIndex" => "tabindex",
    "useMap" => "usemap",
}

/// Convert a camelCase property name to its HTML attribute name.
///
/// Checks the irregular attribute lookup table first, then falls back to
/// generic camelCase → kebab-case conversion (inserting `-` before each
/// uppercase letter).
///
/// # Examples
/// ```
/// # use webui_protocol::attrs::camel_to_kebab;
/// assert_eq!(camel_to_kebab("ariaDescribedBy"), "aria-describedby");
/// assert_eq!(camel_to_kebab("readOnly"), "readonly");
/// assert_eq!(camel_to_kebab("totalContacts"), "total-contacts");
/// ```
#[must_use]
pub fn camel_to_kebab(name: &str) -> String {
    if let Some(attr) = property_to_attribute(name) {
        return attr.to_string();
    }
    let mut result = String::with_capacity(name.len() + 4);
    for ch in name.chars() {
        if ch.is_uppercase() && !result.is_empty() {
            result.push('-');
            for lc in ch.to_lowercase() {
                result.push(lc);
            }
        } else {
            result.push(ch);
        }
    }
    result
}

/// Convert an HTML attribute name to its camelCase property name.
///
/// Checks the irregular attribute lookup table first, then falls back to
/// generic kebab-case → camelCase conversion (removing `-` and capitalizing
/// the following character).
///
/// # Examples
/// ```
/// # use webui_protocol::attrs::attribute_to_camel;
/// assert_eq!(attribute_to_camel("aria-describedby"), "ariaDescribedBy");
/// assert_eq!(attribute_to_camel("readonly"), "readOnly");
/// assert_eq!(attribute_to_camel("data-title"), "dataTitle");
/// ```
#[must_use]
pub fn attribute_to_camel(name: &str) -> String {
    if let Some(prop) = attribute_to_property(name) {
        return prop.to_string();
    }
    let mut result = String::with_capacity(name.len());
    let mut capitalize_next = false;
    for ch in name.chars() {
        if ch == '-' {
            capitalize_next = true;
        } else if capitalize_next {
            result.extend(ch.to_uppercase());
            capitalize_next = false;
        } else {
            result.push(ch);
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exhaustive_property_to_attribute() {
        for &(prop, attr) in ALL_MAPPINGS {
            assert_eq!(
                property_to_attribute(prop),
                Some(attr),
                "property_to_attribute({prop}) should be {attr}"
            );
        }
    }

    #[test]
    fn exhaustive_attribute_to_property() {
        for &(prop, attr) in ALL_MAPPINGS {
            assert_eq!(
                attribute_to_property(attr),
                Some(prop),
                "attribute_to_property({attr}) should be {prop}"
            );
        }
    }

    #[test]
    fn exhaustive_roundtrip() {
        for &(prop, _) in ALL_MAPPINGS {
            let attr = camel_to_kebab(prop);
            let back = attribute_to_camel(&attr);
            assert_eq!(back, prop, "roundtrip failed for {prop}");
        }
    }

    #[test]
    fn single_word_attrs_return_none() {
        assert_eq!(property_to_attribute("ariaLabel"), None);
        assert_eq!(property_to_attribute("ariaHidden"), None);
        assert_eq!(attribute_to_property("aria-label"), None);
        assert_eq!(attribute_to_property("aria-hidden"), None);
    }

    #[test]
    fn non_mapped_returns_none() {
        assert_eq!(property_to_attribute("myProp"), None);
        assert_eq!(attribute_to_property("my-prop"), None);
    }

    #[test]
    fn camel_to_kebab_regular() {
        assert_eq!(camel_to_kebab("ariaLabel"), "aria-label");
        assert_eq!(camel_to_kebab("totalContacts"), "total-contacts");
        assert_eq!(camel_to_kebab("dataTitle"), "data-title");
    }

    #[test]
    fn attribute_to_camel_regular() {
        assert_eq!(attribute_to_camel("aria-label"), "ariaLabel");
        assert_eq!(attribute_to_camel("data-title"), "dataTitle");
    }
}

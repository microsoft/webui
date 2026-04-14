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
//! [ARIAMixin]: https://w3c.github.io/aria/#ARIAMixin

/// Map a camelCase property name to its HTML attribute.
///
/// Returns `None` for names that follow standard camelCase ↔ kebab conversion.
#[must_use]
pub fn property_to_attribute(name: &str) -> Option<&'static str> {
    match name {
        // --- ARIA (ARIAMixin) ---
        "ariaActiveDescendant" => Some("aria-activedescendant"),
        "ariaAutoComplete" => Some("aria-autocomplete"),
        "ariaBrailleLabel" => Some("aria-braillelabel"),
        "ariaBrailleRoleDescription" => Some("aria-brailleroledescription"),
        "ariaColCount" => Some("aria-colcount"),
        "ariaColIndex" => Some("aria-colindex"),
        "ariaColIndexText" => Some("aria-colindextext"),
        "ariaColSpan" => Some("aria-colspan"),
        "ariaDescribedBy" => Some("aria-describedby"),
        "ariaDropEffect" => Some("aria-dropeffect"),
        "ariaErrorMessage" => Some("aria-errormessage"),
        "ariaFlowTo" => Some("aria-flowto"),
        "ariaHasPopup" => Some("aria-haspopup"),
        "ariaKeyShortcuts" => Some("aria-keyshortcuts"),
        "ariaLabelledBy" => Some("aria-labelledby"),
        "ariaMultiLine" => Some("aria-multiline"),
        "ariaMultiSelectable" => Some("aria-multiselectable"),
        "ariaPosInSet" => Some("aria-posinset"),
        "ariaReadOnly" => Some("aria-readonly"),
        "ariaRoleDescription" => Some("aria-roledescription"),
        "ariaRowCount" => Some("aria-rowcount"),
        "ariaRowIndex" => Some("aria-rowindex"),
        "ariaRowIndexText" => Some("aria-rowindextext"),
        "ariaRowSpan" => Some("aria-rowspan"),
        "ariaSetSize" => Some("aria-setsize"),
        "ariaValueMax" => Some("aria-valuemax"),
        "ariaValueMin" => Some("aria-valuemin"),
        "ariaValueNow" => Some("aria-valuenow"),
        "ariaValueText" => Some("aria-valuetext"),
        // --- HTML global/element attributes ---
        "accessKey" => Some("accesskey"),
        "autoCapitalize" => Some("autocapitalize"),
        "contentEditable" => Some("contenteditable"),
        "crossOrigin" => Some("crossorigin"),
        "dirName" => Some("dirname"),
        "fetchPriority" => Some("fetchpriority"),
        "formAction" => Some("formaction"),
        "formEnctype" => Some("formenctype"),
        "formMethod" => Some("formmethod"),
        "formNoValidate" => Some("formnovalidate"),
        "formTarget" => Some("formtarget"),
        "inputMode" => Some("inputmode"),
        "isMap" => Some("ismap"),
        "maxLength" => Some("maxlength"),
        "minLength" => Some("minlength"),
        "noModule" => Some("nomodule"),
        "noValidate" => Some("novalidate"),
        "readOnly" => Some("readonly"),
        "referrerPolicy" => Some("referrerpolicy"),
        "tabIndex" => Some("tabindex"),
        "useMap" => Some("usemap"),
        _ => None,
    }
}

/// Map an HTML attribute to its camelCase property name.
///
/// Inverse of [`property_to_attribute`].
#[must_use]
pub fn attribute_to_property(name: &str) -> Option<&'static str> {
    match name {
        // --- ARIA (ARIAMixin) ---
        "aria-activedescendant" => Some("ariaActiveDescendant"),
        "aria-autocomplete" => Some("ariaAutoComplete"),
        "aria-braillelabel" => Some("ariaBrailleLabel"),
        "aria-brailleroledescription" => Some("ariaBrailleRoleDescription"),
        "aria-colcount" => Some("ariaColCount"),
        "aria-colindex" => Some("ariaColIndex"),
        "aria-colindextext" => Some("ariaColIndexText"),
        "aria-colspan" => Some("ariaColSpan"),
        "aria-describedby" => Some("ariaDescribedBy"),
        "aria-dropeffect" => Some("ariaDropEffect"),
        "aria-errormessage" => Some("ariaErrorMessage"),
        "aria-flowto" => Some("ariaFlowTo"),
        "aria-haspopup" => Some("ariaHasPopup"),
        "aria-keyshortcuts" => Some("ariaKeyShortcuts"),
        "aria-labelledby" => Some("ariaLabelledBy"),
        "aria-multiline" => Some("ariaMultiLine"),
        "aria-multiselectable" => Some("ariaMultiSelectable"),
        "aria-posinset" => Some("ariaPosInSet"),
        "aria-readonly" => Some("ariaReadOnly"),
        "aria-roledescription" => Some("ariaRoleDescription"),
        "aria-rowcount" => Some("ariaRowCount"),
        "aria-rowindex" => Some("ariaRowIndex"),
        "aria-rowindextext" => Some("ariaRowIndexText"),
        "aria-rowspan" => Some("ariaRowSpan"),
        "aria-setsize" => Some("ariaSetSize"),
        "aria-valuemax" => Some("ariaValueMax"),
        "aria-valuemin" => Some("ariaValueMin"),
        "aria-valuenow" => Some("ariaValueNow"),
        "aria-valuetext" => Some("ariaValueText"),
        // --- HTML global/element attributes ---
        "accesskey" => Some("accessKey"),
        "autocapitalize" => Some("autoCapitalize"),
        "contenteditable" => Some("contentEditable"),
        "crossorigin" => Some("crossOrigin"),
        "dirname" => Some("dirName"),
        "fetchpriority" => Some("fetchPriority"),
        "formaction" => Some("formAction"),
        "formenctype" => Some("formEnctype"),
        "formmethod" => Some("formMethod"),
        "formnovalidate" => Some("formNoValidate"),
        "formtarget" => Some("formTarget"),
        "inputmode" => Some("inputMode"),
        "ismap" => Some("isMap"),
        "maxlength" => Some("maxLength"),
        "minlength" => Some("minLength"),
        "nomodule" => Some("noModule"),
        "novalidate" => Some("noValidate"),
        "readonly" => Some("readOnly"),
        "referrerpolicy" => Some("referrerPolicy"),
        "tabindex" => Some("tabIndex"),
        "usemap" => Some("useMap"),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn aria_property_to_attribute() {
        assert_eq!(
            property_to_attribute("ariaDescribedBy"),
            Some("aria-describedby")
        );
        assert_eq!(
            property_to_attribute("ariaLabelledBy"),
            Some("aria-labelledby")
        );
        assert_eq!(
            property_to_attribute("ariaActiveDescendant"),
            Some("aria-activedescendant")
        );
        assert_eq!(
            property_to_attribute("ariaAutoComplete"),
            Some("aria-autocomplete")
        );
        assert_eq!(property_to_attribute("ariaColCount"), Some("aria-colcount"));
        assert_eq!(property_to_attribute("ariaColSpan"), Some("aria-colspan"));
        assert_eq!(
            property_to_attribute("ariaErrorMessage"),
            Some("aria-errormessage")
        );
        assert_eq!(property_to_attribute("ariaHasPopup"), Some("aria-haspopup"));
        assert_eq!(property_to_attribute("ariaPosInSet"), Some("aria-posinset"));
        assert_eq!(
            property_to_attribute("ariaValueText"),
            Some("aria-valuetext")
        );
        assert_eq!(
            property_to_attribute("ariaBrailleRoleDescription"),
            Some("aria-brailleroledescription")
        );
    }

    #[test]
    fn aria_attribute_to_property() {
        assert_eq!(
            attribute_to_property("aria-describedby"),
            Some("ariaDescribedBy")
        );
        assert_eq!(
            attribute_to_property("aria-labelledby"),
            Some("ariaLabelledBy")
        );
        assert_eq!(
            attribute_to_property("aria-activedescendant"),
            Some("ariaActiveDescendant")
        );
        assert_eq!(
            attribute_to_property("aria-valuetext"),
            Some("ariaValueText")
        );
        assert_eq!(
            attribute_to_property("aria-roledescription"),
            Some("ariaRoleDescription")
        );
    }

    #[test]
    fn html_global_property_to_attribute() {
        assert_eq!(property_to_attribute("readOnly"), Some("readonly"));
        assert_eq!(property_to_attribute("tabIndex"), Some("tabindex"));
        assert_eq!(property_to_attribute("accessKey"), Some("accesskey"));
        assert_eq!(
            property_to_attribute("contentEditable"),
            Some("contenteditable")
        );
        assert_eq!(property_to_attribute("crossOrigin"), Some("crossorigin"));
        assert_eq!(property_to_attribute("inputMode"), Some("inputmode"));
        assert_eq!(property_to_attribute("maxLength"), Some("maxlength"));
        assert_eq!(property_to_attribute("formAction"), Some("formaction"));
        assert_eq!(property_to_attribute("noValidate"), Some("novalidate"));
        assert_eq!(
            property_to_attribute("referrerPolicy"),
            Some("referrerpolicy")
        );
    }

    #[test]
    fn html_global_attribute_to_property() {
        assert_eq!(attribute_to_property("readonly"), Some("readOnly"));
        assert_eq!(attribute_to_property("tabindex"), Some("tabIndex"));
        assert_eq!(attribute_to_property("accesskey"), Some("accessKey"));
        assert_eq!(
            attribute_to_property("contenteditable"),
            Some("contentEditable")
        );
        assert_eq!(attribute_to_property("maxlength"), Some("maxLength"));
        assert_eq!(attribute_to_property("formaction"), Some("formAction"));
    }

    #[test]
    fn single_word_attrs_return_none() {
        assert_eq!(property_to_attribute("ariaLabel"), None);
        assert_eq!(property_to_attribute("ariaHidden"), None);
        assert_eq!(attribute_to_property("aria-label"), None);
        assert_eq!(attribute_to_property("aria-hidden"), None);
    }

    #[test]
    fn non_aria_non_global_returns_none() {
        assert_eq!(property_to_attribute("myProp"), None);
        assert_eq!(attribute_to_property("my-prop"), None);
    }

    #[test]
    fn roundtrip() {
        let props = [
            "ariaDescribedBy",
            "ariaLabelledBy",
            "ariaActiveDescendant",
            "ariaValueText",
            "readOnly",
            "tabIndex",
            "contentEditable",
            "maxLength",
            "formAction",
        ];
        for prop in props {
            let attr = property_to_attribute(prop).unwrap();
            let back = attribute_to_property(attr).unwrap();
            assert_eq!(back, prop, "roundtrip failed for {prop}");
        }
    }
}

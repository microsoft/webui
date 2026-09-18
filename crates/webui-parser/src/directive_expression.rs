// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

//! Borrowed normalization at the `<if>` / `<for>` attribute boundary.

pub(crate) fn normalize(input: &str) -> Option<&str> {
    let input = input.trim();
    let Some(inner) = input.strip_prefix("{{") else {
        return Some(input);
    };
    let inner = inner.strip_suffix("}}")?.trim();
    if inner.is_empty() || !whole_expression(inner) {
        return None;
    }
    Some(inner)
}

fn whole_expression(input: &str) -> bool {
    let mut quote = 0;
    let mut escaped = false;
    for byte in input.bytes() {
        if quote != 0 {
            if escaped {
                escaped = false;
            } else if byte == b'\\' {
                escaped = true;
            } else if byte == quote {
                quote = 0;
            }
        } else {
            match byte {
                b'\'' | b'"' => quote = byte,
                b'{' | b'}' => return false,
                _ => {}
            }
        }
    }
    quote == 0
}

pub(crate) fn valid_for_identifier(input: &str) -> bool {
    input
        .bytes()
        .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-' | b'.'))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalization_borrows_bare_and_wrapped_expressions() {
        let bare = "child in items";
        assert_eq!(normalize(bare).map(str::as_ptr), Some(bare.as_ptr()));
        let wrapped = " \t{{ child in items }}\n";
        let expression = &wrapped[5..19];
        assert_eq!(normalize(wrapped), Some(expression));
        assert_eq!(
            normalize(wrapped).map(str::as_ptr),
            Some(expression.as_ptr())
        );
        assert_eq!(
            normalize("{{ child.children.length }}"),
            Some("child.children.length")
        );
        assert_eq!(normalize("{{ label == '}}' }}"), Some("label == '}}'"));
        assert_eq!(
            normalize("label == '{{literal}}'"),
            Some("label == '{{literal}}'")
        );
        assert_eq!(
            normalize(r#"{{ label == "a \" }} b" }}"#),
            Some(r#"label == "a \" }} b""#)
        );
    }

    #[test]
    fn normalization_rejects_partial_nested_and_triple_wrappers() {
        for expression in [
            "{{",
            "{{ }}",
            "{{{ready}}}",
            "{{{{ready}}}}",
            "{{ready}",
            "{{ready}}tail",
            "{{ready}} && {{other}}",
            "{{ready}}}",
            "{{name == 'unterminated}}",
        ] {
            assert_eq!(normalize(expression), None, "{expression}");
        }
    }

    #[test]
    fn normalization_does_not_widen_the_condition_language() {
        assert!(crate::ConditionParser::new().parse("{{ready}}").is_err());
        for identifier in ["item", "_item2", "group.items", "some-items"] {
            assert!(valid_for_identifier(identifier));
        }
        for identifier in ["{{item}}", "items[0]", "it em", "élève"] {
            assert!(!valid_for_identifier(identifier));
        }
    }
}

// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

extern crate serde_json;

use std::borrow::Cow;

use serde_json::Value;

/// Finds a value in a JSON object by dotted path and returns a borrowed value when possible.
///
/// Most lookups borrow directly from `state`. Synthetic values such as array
/// and string `.length` are returned as owned values because they do not exist
/// in the source JSON tree. Synthetic `.length` must be the final path segment;
/// string lengths count UTF-8 bytes, and numeric array indices are not resolved.
#[must_use]
pub fn find_value_by_dotted_path_ref<'a>(path: &str, state: &'a Value) -> Option<Cow<'a, Value>> {
    let mut current_value = state;
    let mut remaining = path;
    loop {
        match current_value {
            Value::Object(map) => {
                let Some((part, rest)) = remaining.split_once('.') else {
                    return map.get(remaining).map(Cow::Borrowed);
                };
                current_value = map.get(part)?;
                remaining = rest;
            }
            Value::Array(arr) if remaining == "length" => {
                return Some(Cow::Owned(Value::Number(serde_json::Number::from(
                    arr.len(),
                ))));
            }
            Value::String(s) if remaining == "length" => {
                return Some(Cow::Owned(Value::Number(serde_json::Number::from(s.len()))));
            }
            _ => return None,
        }
    }
}

/// Finds a value in a JSON object by dotted path and returns an owned value.
///
/// Prefer [`find_value_by_dotted_path_ref`] in render-time hot paths so state
/// values can be borrowed without cloning.
#[must_use]
pub fn find_value_by_dotted_path(path: &str, state: &Value) -> Option<Value> {
    find_value_by_dotted_path_ref(path, state).map(Cow::into_owned)
}

#[cfg(test)]
mod tests {
    use super::*;
    use webui_test_utils::test_json;

    #[test]
    fn test_find_value_by_dotted_path() {
        // Create a JSON object
        let data = test_json!({
            "name": {
                "first": "John",
                "last": "Doe"
            },
            "favorite": {
                "categories": {
                    "movies": ["The Matrix", "The Godfather"],
                    "music": ["Jazz", "Blues"]
                }
            },
            "age": 30
        });

        // Test a successful path
        let value = find_value_by_dotted_path("name.first", &data);
        assert_eq!(
            value,
            Some(Value::String("John".to_string())),
            "Failed to get string."
        );

        // Test a path that leads to single string
        let value = find_value_by_dotted_path("age", &data);
        assert_eq!(
            value,
            Some(Value::Number(serde_json::Number::from(30))),
            "Failed to get number."
        );

        // Test a path that leads to an array
        let value = find_value_by_dotted_path("favorite.categories.music", &data);
        assert_eq!(
            value,
            Some(Value::Array(vec![
                Value::String("Jazz".to_string()),
                Value::String("Blues".to_string())
            ])),
            "Failed to get array."
        );

        // Test a non-existent path
        let value = find_value_by_dotted_path("name.middle", &data);
        assert_eq!(value, None, "Failed to handle non-existent path.");

        // Test a path that leads to a non-object value
        let value = find_value_by_dotted_path("age.years", &data);
        assert_eq!(
            value, None,
            "Failed to handle path that leads to a non-object value."
        );

        // Get length of array.
        let value = find_value_by_dotted_path("favorite.categories.music.length", &data);
        assert_eq!(
            value,
            Some(Value::Number(serde_json::Number::from(2))),
            "Failed to get length of array."
        );
    }

    #[test]
    fn test_nested_nested_path() {
        let data = test_json!({
            "user": {
                "profile": {
                    "nameObj": {
                        "first": "John"
                    }
                }
            }
        });
        let value = find_value_by_dotted_path("user.profile.nameObj.first", &data);
        assert_eq!(value, Some(Value::String("John".to_string())));
    }

    #[test]
    fn test_empty_string_path() {
        let data = test_json!({ "key": "value" });
        let value = find_value_by_dotted_path("", &data);
        assert_eq!(value, None);
    }

    #[test]
    fn test_path_with_spaces() {
        let data = test_json!({
            "user": {
                "profile": {
                    "name": "Alice"
                }
            }
        });
        let value = find_value_by_dotted_path("user.profile.non existent", &data);
        assert_eq!(value, None);
    }

    #[test]
    fn test_numeric_string_token() {
        let data = test_json!({ "numeric_string": "123" });
        let value = find_value_by_dotted_path("numeric_string", &data);
        assert_eq!(value, Some(Value::String("123".to_string())));
    }

    #[test]
    fn test_length_on_empty_list() {
        let data = test_json!({ "empty_list": [] });
        let value = find_value_by_dotted_path("empty_list.length", &data);
        assert_eq!(value, Some(Value::Number(serde_json::Number::from(0))));
    }

    #[test]
    fn test_length_on_nonempty_list() {
        let data = test_json!({ "items": [1, 2, 3] });
        let value = find_value_by_dotted_path("items.length", &data);
        assert_eq!(value, Some(Value::Number(serde_json::Number::from(3))));
    }

    #[test]
    fn test_length_named_property() {
        let data = test_json!({
            "nestedlength": {
                "length": "some value"
            }
        });
        let value = find_value_by_dotted_path("nestedlength.length", &data);
        assert_eq!(value, Some(Value::String("some value".to_string())));
    }

    #[test]
    fn test_length_on_nested_list() {
        let data = test_json!({
            "nested": {
                "data": ["a", "b"]
            }
        });
        let value = find_value_by_dotted_path("nested.data.length", &data);
        assert_eq!(value, Some(Value::Number(serde_json::Number::from(2))));
    }

    #[test]
    fn test_nonexistent_path_with_length() {
        let data = test_json!({ "existing": "value" });
        let value = find_value_by_dotted_path("non_existent.length", &data);
        assert_eq!(value, None);
    }

    #[test]
    fn test_length_on_string() {
        let data = test_json!({ "str_val": "hello" });
        let value = find_value_by_dotted_path("str_val.length", &data);
        assert_eq!(value, Some(Value::Number(serde_json::Number::from(5))));
    }

    #[test]
    fn test_length_on_string_matches_nodejs() {
        // NodeJS: 'string_value.length' on "test" → 4
        let data = test_json!({ "string_value": "test" });
        let value = find_value_by_dotted_path("string_value.length", &data);
        assert_eq!(value, Some(Value::Number(serde_json::Number::from(4))));
    }

    #[test]
    fn string_length_counts_utf8_bytes() {
        for (text, length) in [("é", 2), ("😀", 4), ("é😀", 6), ("a\u{301}", 3)] {
            let data = test_json!({"text": text});
            assert_eq!(
                find_value_by_dotted_path("text.length", &data),
                Some(Value::from(length))
            );
            assert_eq!(find_value_by_dotted_path("text.length.more", &data), None);
        }
    }

    #[test]
    fn test_array_index_not_resolved() {
        let data = test_json!({
            "foo": {
                "bar": [10, 20, 30]
            }
        });
        let value = find_value_by_dotted_path("foo.bar.0", &data);
        assert_eq!(value, None);
    }

    #[test]
    fn test_negative_array_index_not_resolved() {
        let data = test_json!({
            "foo": {
                "bar": [10, 20, 30]
            }
        });
        let value = find_value_by_dotted_path("foo.bar.-1", &data);
        assert_eq!(value, None);
    }

    #[test]
    fn test_numeric_property_on_object() {
        let data = test_json!({
            "foo": {
                "barObj": {
                    "0": "not an array"
                }
            }
        });
        let value = find_value_by_dotted_path("foo.barObj.0", &data);
        assert_eq!(value, Some(Value::String("not an array".to_string())));
    }

    #[test]
    fn test_property_value_zero() {
        let data = test_json!({
            "foo": {
                "zero": 0
            }
        });
        let value = find_value_by_dotted_path("foo.zero", &data);
        assert_eq!(value, Some(Value::Number(serde_json::Number::from(0))));
    }

    #[test]
    fn test_property_value_false() {
        let data = test_json!({
            "foo": {
                "isFalse": false
            }
        });
        let value = find_value_by_dotted_path("foo.isFalse", &data);
        assert_eq!(value, Some(Value::Bool(false)));
    }

    #[test]
    fn test_property_value_empty_string() {
        let data = test_json!({
            "foo": {
                "empty": ""
            }
        });
        let value = find_value_by_dotted_path("foo.empty", &data);
        assert_eq!(value, Some(Value::String("".to_string())));
    }

    #[test]
    fn test_deeply_nested_property() {
        let data = test_json!({
            "a": {
                "b": {
                    "c": {
                        "d": {
                            "e": {
                                "f": 42
                            }
                        }
                    }
                }
            }
        });
        let value = find_value_by_dotted_path("a.b.c.d.e.f", &data);
        assert_eq!(value, Some(Value::Number(serde_json::Number::from(42))));
    }

    #[test]
    fn test_property_with_special_char() {
        let data = test_json!({
            "foo": {
                "$bar": 123
            }
        });
        let value = find_value_by_dotted_path("foo.$bar", &data);
        assert_eq!(value, Some(Value::Number(serde_json::Number::from(123))));
    }

    #[test]
    fn test_property_named_tostring() {
        let data = test_json!({
            "foo": {
                "toString": "stringify"
            }
        });
        let value = find_value_by_dotted_path("foo.toString", &data);
        assert_eq!(value, Some(Value::String("stringify".to_string())));
    }

    #[test]
    fn test_intermediate_property_not_found() {
        let data = test_json!({
            "foo": {
                "bar": "baz"
            }
        });
        let value = find_value_by_dotted_path("foo.undefinedMid.baz", &data);
        assert_eq!(value, None);
    }

    #[test]
    fn test_string_in_deeply_nested_path() {
        let data = test_json!({
            "first": {
                "second": {
                    "third": {
                        "data": "deep_string"
                    }
                }
            }
        });
        let value = find_value_by_dotted_path("first.second.third.data", &data);
        assert_eq!(value, Some(Value::String("deep_string".to_string())));
    }

    #[test]
    fn test_double_value_directly() {
        let data = test_json!({ "double_value": 1.23 });
        let value = find_value_by_dotted_path("double_value", &data);
        assert_eq!(
            value,
            Some(Value::Number(serde_json::Number::from_f64(1.23).unwrap()))
        );
    }

    #[test]
    fn test_ref_lookup_borrows_existing_value() {
        let data = test_json!({
            "user": {
                "name": "Alice"
            }
        });
        let value = find_value_by_dotted_path_ref("user.name", &data).expect("value");
        let expected = data
            .get("user")
            .and_then(|user| user.get("name"))
            .expect("expected value");

        assert!(matches!(value, std::borrow::Cow::Borrowed(_)));
        assert!(std::ptr::eq(value.as_ref(), expected));
    }

    #[test]
    fn test_ref_lookup_owns_synthetic_length() {
        let data = test_json!({ "items": [1, 2, 3] });
        let value = find_value_by_dotted_path_ref("items.length", &data).expect("value");

        assert!(matches!(value, std::borrow::Cow::Owned(_)));
        assert_eq!(value.as_ref(), &Value::Number(serde_json::Number::from(3)));
    }

    #[test]
    fn test_synthetic_length_must_be_terminal() {
        let data = test_json!({"items": [1, 2], "text": "é"});
        for path in [
            "items.length.trailing",
            "items.length.",
            "text.length.trailing",
            "text.length.",
        ] {
            assert_eq!(find_value_by_dotted_path_ref(path, &data), None);
            assert_eq!(find_value_by_dotted_path(path, &data), None);
        }
        assert_eq!(
            find_value_by_dotted_path("text.length", &data),
            Some(Value::from(2))
        );
    }

    #[test]
    fn test_object_length_property_can_have_descendants() {
        let data = test_json!({"object": {"length": {"trailing": false}}});
        assert_eq!(
            find_value_by_dotted_path("object.length.trailing", &data),
            Some(Value::Bool(false))
        );
    }

    #[test]
    fn terminal_object_leaves_borrow_every_json_type() {
        for value in [
            Value::Null,
            test_json!(false),
            test_json!(42),
            test_json!("text"),
            test_json!([1, 2]),
            test_json!({"child": "value"}),
        ] {
            let data = test_json!({"leaf": value, "nested": {"leaf": value}});
            for path in ["leaf", "nested.leaf"] {
                let Some(Cow::Borrowed(selected)) = find_value_by_dotted_path_ref(path, &data)
                else {
                    panic!("existing leaf must borrow: {path}");
                };
                let expected = if path == "leaf" {
                    &data["leaf"]
                } else {
                    &data["nested"]["leaf"]
                };
                assert!(std::ptr::eq(selected, expected));
                assert_eq!(find_value_by_dotted_path(path, &data), Some(value.clone()));
            }
        }
    }

    #[test]
    fn suffix_walk_preserves_empty_keys_unicode_and_synthetic_length() {
        let data = test_json!({
            "": {"": false, "leaf": "root-empty"},
            "nested": {"": {"leaf": 3}, "0": "numeric-key"},
            "é": {"😀": "borrowed"},
            "length": {"length": {"leaf": true}},
            "array": [1, 2],
            "text": "é😀"
        });
        for (path, expected) in [
            (".", Some(test_json!(false))),
            (".leaf", Some(test_json!("root-empty"))),
            ("nested..leaf", Some(test_json!(3))),
            ("nested.0", Some(test_json!("numeric-key"))),
            ("é.😀", Some(test_json!("borrowed"))),
            ("length.length.leaf", Some(test_json!(true))),
            ("array.length", Some(test_json!(2))),
            ("text.length", Some(test_json!(6))),
            ("array.length.", None),
            ("text.length.more", None),
            ("array.0", None),
            ("nested...", None),
            ("é.😀.", None),
        ] {
            assert_eq!(find_value_by_dotted_path(path, &data), expected, "{path}");
        }
        assert_eq!(find_value_by_dotted_path("", &data), Some(data[""].clone()));
        for scalar in [Value::Null, test_json!(true), test_json!(42)] {
            for path in ["", ".", "length", "missing"] {
                assert_eq!(find_value_by_dotted_path_ref(path, &scalar), None);
            }
        }
    }
}

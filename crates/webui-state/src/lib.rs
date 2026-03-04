extern crate serde_json;

use serde_json::Value;

// Finds a value in a JSON object by a dotted path.
pub fn find_value_by_dotted_path(path: &str, state: &Value) -> Option<Value> {
    let parts: Vec<&str> = path.split('.').collect();
    let mut current_value: &Value = state;

    for part in parts.iter() {
        match current_value {
            Value::Object(map) => {
                current_value = map.get(*part)?;
            }
            Value::Array(arr) if *part == "length" => {
                return Some(Value::Number(serde_json::Number::from(arr.len())));
            }
            Value::String(s) if *part == "length" => {
                return Some(Value::Number(serde_json::Number::from(s.len())));
            }
            _ => return None,
        }
    }

    Some(current_value.clone())
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
        let data = test_json!({ "double_value": 3.14159 });
        let value = find_value_by_dotted_path("double_value", &data);
        assert_eq!(
            value,
            Some(Value::Number(
                serde_json::Number::from_f64(3.14159).unwrap()
            ))
        );
    }
}

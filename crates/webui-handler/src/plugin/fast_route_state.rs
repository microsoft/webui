// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

use crate::{ResponseWriter, Result};
use serde_json::Value;
use std::borrow::Cow;
use webui_protocol::attrs::camel_to_kebab;

pub(crate) fn write_fast_route_component_state(
    state: &Value,
    writer: &mut dyn ResponseWriter,
) -> Result<()> {
    let map = match state.as_object() {
        Some(map) => map,
        None => return Ok(()),
    };

    for (key, value) in map {
        let val_str = match value {
            Value::String(s) => Cow::Borrowed(s.as_str()),
            Value::Number(n) => Cow::Owned(n.to_string()),
            Value::Bool(true) => Cow::Borrowed("true"),
            Value::Bool(false) => Cow::Borrowed("false"),
            _ => continue,
        };

        let attr_name = camel_to_kebab(key);
        writer.write(" ")?;
        writer.write(&attr_name)?;
        writer.write("=\"")?;
        crate::route_renderer::write_escaped_state_attr(writer, val_str.as_ref())?;
        writer.write("\"")?;
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    #![allow(clippy::disallowed_methods)]

    use super::*;
    use crate::plugin::{
        fast_v2::FastV2HydrationPlugin, fast_v3::FastV3HydrationPlugin, HandlerPlugin,
    };

    struct TestWriter {
        output: String,
    }

    impl TestWriter {
        fn new() -> Self {
            Self {
                output: String::new(),
            }
        }
    }

    impl ResponseWriter for TestWriter {
        fn write(&mut self, content: &str) -> Result<()> {
            self.output.push_str(content);
            Ok(())
        }

        fn end(&mut self) -> Result<()> {
            Ok(())
        }
    }

    fn render_route_state(plugin: &dyn HandlerPlugin, state: &Value) -> String {
        let mut writer = TestWriter::new();
        plugin
            .write_route_component_state(state, &mut writer)
            .unwrap();
        writer.output
    }

    #[test]
    fn fast_route_component_state_emits_scalar_attrs_only() {
        let state = serde_json::json!({
            "title": "A&B\"<>",
            "count": 42,
            "enabled": true,
            "items": [{"name": "ignored"}],
            "meta": {"nested": "ignored"},
        });
        let fast_v2 = FastV2HydrationPlugin::new();
        let fast_v3 = FastV3HydrationPlugin::new();

        for plugin in [
            &fast_v2 as &dyn HandlerPlugin,
            &fast_v3 as &dyn HandlerPlugin,
        ] {
            let output = render_route_state(plugin, &state);
            assert!(
                !output.contains("data-state="),
                "data-state must be omitted: {output}"
            );
            assert!(
                output.contains(r#" title="A&amp;B&quot;&lt;&gt;""#),
                "escaped scalar string attr missing: {output}"
            );
            assert!(
                output.contains(r#" count="42""#),
                "numeric scalar attr missing: {output}"
            );
            assert!(
                output.contains(r#" enabled="true""#),
                "boolean scalar attr missing: {output}"
            );
            assert!(
                !output.contains("items="),
                "complex array attr must be omitted: {output}"
            );
            assert!(
                !output.contains("meta="),
                "complex object attr must be omitted: {output}"
            );
        }
    }

    #[test]
    fn fast_route_component_state_ignores_non_object_values() {
        let state = serde_json::json!(["not", "an", "object"]);
        let fast_v2 = FastV2HydrationPlugin::new();
        let fast_v3 = FastV3HydrationPlugin::new();

        for plugin in [
            &fast_v2 as &dyn HandlerPlugin,
            &fast_v3 as &dyn HandlerPlugin,
        ] {
            assert_eq!(render_route_state(plugin, &state), "");
        }
    }
}

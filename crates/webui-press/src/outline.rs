// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

use serde_json::Value;

use crate::error::{Error, Result};
use crate::types::{DocsConfig, PageDescriptor, ShowMode};

pub(crate) const REGION: &str = "page.outline";

pub(crate) fn parse_override(value: &serde_yaml::Value) -> Result<bool> {
    value.as_bool().ok_or_else(invalid_override)
}

#[cold]
#[inline(never)]
fn invalid_override() -> Error {
    Error::Markdown(
        "outline must be a boolean.\nhelp: Use outline: true or outline: false, without quotes."
            .to_string(),
    )
}

pub(crate) fn enabled(state: &Value) -> bool {
    state["page"]["outline"].as_bool() == Some(true)
}

pub(crate) fn omitted(config: &DocsConfig, active: bool) -> Option<&'static str> {
    (!active && !replaced(config)).then_some(REGION)
}

fn replaced(config: &DocsConfig) -> bool {
    config
        .regions
        .get(REGION)
        .is_some_and(|region| region.html.is_some() || region.html_file.is_some())
}

pub(crate) fn prepare(
    pages: &mut [PageDescriptor],
    config: &DocsConfig,
    has_region: bool,
) -> Result<()> {
    let replaced = replaced(config);
    for page in pages {
        let active = !replaced
            && config.show == ShowMode::All
            && !page.is_home
            && enabled(&page.state)
            && matches!(page.state["page"]["layout"].as_str(), Some("doc" | "page"))
            && has_sections(&page.state["page"]["headings"]);
        if active && !has_region {
            return Err(missing_region());
        }
        page.state["page"]["outline"] = Value::Bool(active);
    }
    Ok(())
}

fn has_sections(headings: &Value) -> bool {
    headings.as_array().is_some_and(|headings| {
        headings
            .iter()
            .filter(|heading| matches!(heading["level"].as_u64(), Some(2..=6)))
            .take(2)
            .count()
            == 2
    })
}

#[cold]
#[inline(never)]
fn missing_region() -> Error {
    Error::Build(
        "Page outlines need the 'page.outline' template region.\n\
         help: Use the bundled Press template, add its page.outline region to your custom template, \
         or disable outline."
            .to_string(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    type TestResult = std::result::Result<(), Box<dyn std::error::Error>>;

    fn config() -> Result<DocsConfig> {
        serde_json::from_str(
            r#"{"site":{"title":"Docs"},"basePath":"/","contentDir":".","nav":[],"sidebar":[]}"#,
        )
        .map_err(|error| Error::Build(error.to_string()))
    }

    fn page(layout: &str, requested: bool, count: usize) -> PageDescriptor {
        let headings: Vec<Value> = (0..count)
            .map(|_| {
                Value::Object(
                    [("level".to_string(), Value::from(2))]
                        .into_iter()
                        .collect(),
                )
            })
            .collect();
        PageDescriptor {
            path: "/guide/".to_string(),
            is_home: layout == "home",
            state: Value::Object(
                [(
                    "page".to_string(),
                    Value::Object(
                        [
                            ("layout".to_string(), Value::String(layout.to_string())),
                            ("outline".to_string(), Value::Bool(requested)),
                            ("headings".to_string(), Value::Array(headings)),
                        ]
                        .into_iter()
                        .collect(),
                    ),
                )]
                .into_iter()
                .collect(),
            ),
        }
    }

    #[test]
    fn selection_requires_opt_in_two_sections_and_a_reading_layout() -> TestResult {
        let config = config()?;
        for (layout, requested, count, expected) in [
            ("doc", true, 2, true),
            ("page", true, 2, true),
            ("doc", false, 10, false),
            ("doc", true, 1, false),
            ("doc", true, 0, false),
            ("home", true, 10, false),
            ("full", true, 10, false),
        ] {
            let mut pages = [page(layout, requested, count)];
            prepare(&mut pages, &config, true)?;
            assert_eq!(enabled(&pages[0].state), expected);
            assert_eq!(
                omitted(&config, expected),
                if expected { None } else { Some(REGION) }
            );
        }
        Ok(())
    }

    #[test]
    fn content_mode_does_not_enable_outline_assets() -> TestResult {
        let mut config = config()?;
        config.show = ShowMode::Content;
        let mut pages = [page("doc", true, 10)];
        prepare(&mut pages, &config, false)?;
        assert!(!enabled(&pages[0].state));
        Ok(())
    }

    #[test]
    fn replacing_the_outline_region_owns_its_own_presentation() -> TestResult {
        let mut config = config()?;
        config
            .regions
            .insert(REGION.to_string(), serde_json::from_str(r#"{"html":""}"#)?);
        let mut pages = [page("doc", true, 10)];
        prepare(&mut pages, &config, true)?;
        assert!(!enabled(&pages[0].state));
        assert_eq!(omitted(&config, false), None);
        Ok(())
    }

    #[test]
    fn missing_custom_template_region_is_actionable() -> TestResult {
        let mut pages = [page("doc", true, 2)];
        let result = prepare(&mut pages, &config()?, false);
        assert!(matches!(result, Err(Error::Build(message)) if
            message.contains("page.outline") && message.contains("help:")));
        Ok(())
    }

    #[test]
    fn h1_is_not_an_outline_section() -> TestResult {
        let headings: Value = serde_json::from_str(r#"[{"level":1},{"level":2}]"#)?;
        assert!(!has_sections(&headings));
        let headings: Value = serde_json::from_str(r#"[{"level":2},{"level":6}]"#)?;
        assert!(has_sections(&headings));
        Ok(())
    }

    #[test]
    fn frontmatter_override_requires_a_boolean() -> TestResult {
        assert!(parse_override(&serde_yaml::Value::Bool(true))?);
        assert!(!parse_override(&serde_yaml::Value::Bool(false))?);
        for input in ["null", "\"false\"", "1", "[]", "{}"] {
            let value = serde_yaml::from_str(input)?;
            assert!(
                matches!(parse_override(&value), Err(Error::Markdown(message)) if
                message.contains("outline") && message.contains("help:"))
            );
        }
        Ok(())
    }
}

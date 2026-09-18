// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

use std::cell::Cell;
use std::rc::Rc;

use crate::plugin::{ComponentBuildContext, ParserPlugin};
use crate::{ComponentRegistration, CssStrategy, HtmlParser, ParserError, Result};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct ContextObservation {
    calls: usize,
    html: *const u8,
    css: Option<*const u8>,
}

struct ContextPlugin {
    observed: Rc<Cell<ContextObservation>>,
}

impl ParserPlugin for ContextPlugin {
    fn component_built(&mut self, context: ComponentBuildContext<'_>) -> Result<()> {
        assert_eq!(context.component.tag_name, "x-context");
        assert!(context.template.contains("Hello"));
        self.observed.set(ContextObservation {
            calls: self.observed.get().calls + 1,
            html: context.component.html_content.as_ptr(),
            css: context
                .component
                .css_content
                .as_ref()
                .map(|css| css.as_ptr()),
        });
        Ok(())
    }
}

fn assert_borrowed_context(entry: &str, css_strategy: CssStrategy) -> Result<()> {
    let observed = Rc::new(Cell::new(ContextObservation {
        calls: 0,
        html: std::ptr::null(),
        css: None,
    }));
    let mut parser = HtmlParser::with_plugin_options(
        Box::new(ContextPlugin {
            observed: Rc::clone(&observed),
        }),
        css_strategy,
    );
    parser
        .component_registry_mut()
        .register_component(ComponentRegistration::new(
            "x-context",
            "<p>Hello</p>",
            Some(":host { display: block; }"),
            true,
        ))?;
    parser.parse("index.html", entry)?;
    let component = parser
        .component_registry()
        .get("x-context")
        .ok_or_else(|| ParserError::NotFound("registered test component disappeared".to_owned()))?;
    assert!(component.css_content.is_some());
    assert_eq!(
        observed.get(),
        ContextObservation {
            calls: 1,
            html: component.html_content.as_ptr(),
            css: component.css_content.as_ref().map(|css| css.as_ptr()),
        }
    );
    Ok(())
}

#[test]
fn component_callback_borrows_registered_definition() -> Result<()> {
    for css in [CssStrategy::Link, CssStrategy::Style, CssStrategy::Module] {
        assert_borrowed_context("<x-context></x-context>", css)?;
    }
    Ok(())
}

#[test]
fn route_callback_borrows_registered_definition() -> Result<()> {
    for css in [CssStrategy::Link, CssStrategy::Style, CssStrategy::Module] {
        assert_borrowed_context(r#"<route path="/" component="x-context" exact />"#, css)?;
    }
    Ok(())
}

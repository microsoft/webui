// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

use rayon::prelude::*;
use std::borrow::Cow;
use webui_protocol::{CssStrategy, WebUIProtocol};

use super::graph::AssetGraphPlan;
use super::json::encode_json_string;
use crate::WebUIError;

pub(super) struct RenderedComponent<'a> {
    pub resource: Option<RenderedStyleResource<'a>>,
    pub template: Cow<'a, str>,
    pub functions: Option<&'a str>,
}

pub(super) enum RenderedStyleResource<'a> {
    Link(&'a str),
    Style(&'a str),
    Module(&'a str),
}

pub(super) fn render_component_payloads<'a>(
    protocol: &'a WebUIProtocol,
    plan: &AssetGraphPlan,
) -> Result<Vec<Option<RenderedComponent<'a>>>, WebUIError> {
    let mut payloads: Vec<Option<RenderedComponent<'a>>> = std::iter::repeat_with(|| None)
        .take(plan.component_names.len())
        .collect();
    let strategy = protocol.css_strategy();
    if plan.roots.len() == 1 {
        for component in &plan.emitted_components {
            payloads[*component] = Some(render_component(
                protocol,
                plan.component_names[*component],
                strategy,
            )?);
        }
        return Ok(payloads);
    }

    let rendered: Vec<Result<(usize, RenderedComponent<'a>), WebUIError>> = plan
        .emitted_components
        .par_iter()
        .map(|component| {
            render_component(protocol, plan.component_names[*component], strategy)
                .map(|payload| (*component, payload))
        })
        .collect();
    for payload in rendered {
        let (component, payload) = payload?;
        payloads[component] = Some(payload);
    }
    Ok(payloads)
}

fn render_component<'a>(
    protocol: &'a WebUIProtocol,
    tag: &str,
    strategy: CssStrategy,
) -> Result<RenderedComponent<'a>, WebUIError> {
    let component = protocol.components.get(tag).ok_or_else(|| {
        WebUIError::InvalidBuildOptions(format!(
            "component asset payload for <{tag}> is missing protocol metadata"
        ))
    })?;
    let resource_value = match strategy {
        CssStrategy::Link => component.css_href.as_str(),
        CssStrategy::Style | CssStrategy::Module => component.css.as_str(),
    };
    let resource = (!resource_value.is_empty()).then_some(match strategy {
        CssStrategy::Link => RenderedStyleResource::Link(resource_value),
        CssStrategy::Style => RenderedStyleResource::Style(resource_value),
        CssStrategy::Module => RenderedStyleResource::Module(resource_value),
    });
    let template = if component.template_json.is_empty() {
        Cow::Owned(encode_json_string(
            &component.template,
            "component template",
        )?)
    } else {
        Cow::Borrowed(component.template_json.as_str())
    };
    let functions =
        (!component.template_functions.is_empty()).then_some(component.template_functions.as_str());
    Ok(RenderedComponent {
        resource,
        template,
        functions,
    })
}

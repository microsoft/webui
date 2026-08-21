// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

use std::collections::HashMap;
use webui_protocol::{web_ui_fragment::Fragment, WebUIProtocol};

use crate::WebUIError;

pub(super) struct GraphIndex<'a> {
    pub fragment_names: Vec<&'a str>,
    fragment_ids: HashMap<&'a str, usize>,
    pub component_names: Vec<&'a str>,
    component_ids: HashMap<&'a str, usize>,
}

impl<'a> GraphIndex<'a> {
    pub fn new(protocol: &'a WebUIProtocol) -> Self {
        let mut fragment_names: Vec<&str> = protocol.fragments.keys().map(String::as_str).collect();
        fragment_names.sort_unstable();
        let mut fragment_ids = HashMap::with_capacity(fragment_names.len());
        for (id, name) in fragment_names.iter().copied().enumerate() {
            fragment_ids.insert(name, id);
        }

        let mut component_names: Vec<&str> = protocol
            .components
            .iter()
            .filter_map(|(name, component)| {
                has_template_payload(component).then_some(name.as_str())
            })
            .collect();
        component_names.sort_unstable();
        let mut component_ids = HashMap::with_capacity(component_names.len());
        for (id, name) in component_names.iter().copied().enumerate() {
            component_ids.insert(name, id);
        }

        Self {
            fragment_names,
            fragment_ids,
            component_names,
            component_ids,
        }
    }
}

pub(super) struct CollectedClosure {
    pub components: Vec<usize>,
    pub component_order: Vec<usize>,
    pub fragments: Vec<usize>,
}

pub(super) struct TraversalScratch {
    marks: Vec<u32>,
    generation: u32,
    stack: Vec<usize>,
    components: Vec<usize>,
    fragments: Vec<usize>,
}

impl TraversalScratch {
    pub fn new(fragment_count: usize) -> Self {
        Self {
            marks: vec![0; fragment_count],
            generation: 0,
            stack: Vec::with_capacity(fragment_count.min(256)),
            components: Vec::new(),
            fragments: Vec::new(),
        }
    }

    pub fn collect(
        &mut self,
        protocol: &WebUIProtocol,
        index: &GraphIndex<'_>,
        root: &str,
    ) -> Result<CollectedClosure, WebUIError> {
        self.begin();
        let Some(&root_id) = index.fragment_ids.get(root) else {
            return Ok(CollectedClosure {
                components: Vec::new(),
                component_order: Vec::new(),
                fragments: Vec::new(),
            });
        };
        self.stack.push(root_id);

        while let Some(fragment_id) = self.stack.pop() {
            if self.marks[fragment_id] == self.generation {
                continue;
            }
            self.marks[fragment_id] = self.generation;
            self.fragments.push(fragment_id);

            let fragment_name = index.fragment_names[fragment_id];
            if let Some(&component_id) = index.component_ids.get(fragment_name) {
                self.components.push(component_id);
            }

            let Some(fragment_list) = protocol.fragments.get(fragment_name) else {
                continue;
            };
            for fragment in fragment_list.fragments.iter().rev() {
                enqueue_dependency(
                    fragment.fragment.as_ref(),
                    fragment_name,
                    index,
                    &mut self.stack,
                )?;
            }
        }

        let component_order = self.components.clone();
        self.components.sort_unstable();
        self.fragments.sort_unstable();
        Ok(CollectedClosure {
            components: std::mem::take(&mut self.components),
            component_order,
            fragments: std::mem::take(&mut self.fragments),
        })
    }

    fn begin(&mut self) {
        if self.generation == u32::MAX {
            self.marks.fill(0);
            self.generation = 1;
        } else {
            self.generation += 1;
        }
        self.stack.clear();
        self.components.clear();
        self.fragments.clear();
    }
}

pub(super) fn has_template_payload(component: &webui_protocol::ComponentData) -> bool {
    !component.template_json.is_empty() || !component.template.is_empty()
}

fn enqueue_dependency(
    fragment: Option<&Fragment>,
    owner: &str,
    index: &GraphIndex<'_>,
    stack: &mut Vec<usize>,
) -> Result<(), WebUIError> {
    let dependency = match fragment {
        Some(Fragment::Component(component)) => Some(component.fragment_id.as_str()),
        Some(Fragment::ForLoop(for_loop)) => Some(for_loop.fragment_id.as_str()),
        Some(Fragment::IfCond(if_cond)) => Some(if_cond.fragment_id.as_str()),
        Some(Fragment::Attribute(attribute)) if !attribute.template.is_empty() => {
            Some(attribute.template.as_str())
        }
        Some(Fragment::Route(_)) => return Err(routes_unsupported(owner)),
        _ => None,
    };
    if let Some(dependency) = dependency {
        if let Some(&fragment_id) = index.fragment_ids.get(dependency) {
            stack.push(fragment_id);
        }
    }
    Ok(())
}

#[cold]
#[inline(never)]
fn routes_unsupported(owner: &str) -> WebUIError {
    let diagnostic = webui_parser::Diagnostic::error(
        "routes cannot be combined with static component assets",
    )
    .code(webui_parser::codes::COMPONENT_ASSETS_WITH_ROUTES)
    .component(owner)
    .element("route")
    .help(
        "remove --emit-component-assets and use @microsoft/webui-router, or remove <route> and load deferred roots with defineComponentAssets()",
    );
    WebUIError::ComponentAssets(Box::new(diagnostic))
}

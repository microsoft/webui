// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

use std::collections::BTreeMap;

use webui_protocol::{ComponentData, StateProjectionMode};

use crate::{projection, BuildOptions, Diagnostic, RawBuildOutput};

/// Warning that a component's authored source cannot prove exact state.
pub const FULL_STATE_WARNING: &str = "PROJ-B003";

pub(crate) fn apply(raw: &mut RawBuildOutput, options: &BuildOptions) {
    let candidates: BTreeMap<_, _> = raw
        .component_templates
        .iter()
        .filter(|artifact| {
            artifact.is_scripted || options.component_modules.contains_key(&artifact.tag_name)
        })
        .filter(|artifact| raw.protocol.fragments.contains_key(&artifact.tag_name))
        .map(|artifact| (artifact.tag_name.as_str(), artifact))
        .collect();
    if candidates.is_empty() {
        return;
    }
    let mut surfaces = webui_projection::analyze_sources(
        &options.app_dir,
        candidates.keys().filter_map(|&tag| {
            if options.component_modules.contains_key(tag) {
                return None;
            }
            let path = raw.component_scripts.get(tag)?.as_deref()?;
            Some((tag, path))
        }),
    );
    for (tag, artifact) in candidates {
        let Some(component) = raw.protocol.components.get_mut(tag) else {
            continue;
        };
        match surfaces.remove(tag) {
            Some(Ok(surface)) => {
                let hydration = surface.hydration_keys.into_iter().collect();
                let navigation = surface.navigation_keys.into_iter().collect::<Vec<_>>();
                set_exact(
                    component,
                    hydration,
                    projection::union_keys(&navigation, &artifact.template_roots),
                );
            }
            rejected => {
                component.hydration_mode = StateProjectionMode::All as i32;
                component.hydration_keys.clear();
                component.navigation_mode = Some(StateProjectionMode::All as i32);
                component.navigation_keys.clear();
                raw.warnings.push(full_state_warning(
                    tag,
                    rejected.and_then(Result::err),
                    options.component_modules.contains_key(tag),
                ));
            }
        }
    }
}

fn set_exact(component: &mut ComponentData, hydration: Vec<String>, navigation: Vec<String>) {
    let mode = |keys: &[String]| {
        (if keys.is_empty() {
            StateProjectionMode::None
        } else {
            StateProjectionMode::Keys
        }) as i32
    };
    component.hydration_mode = mode(&hydration);
    component.hydration_keys = hydration;
    component.navigation_mode = Some(mode(&navigation));
    component.navigation_keys = navigation;
}

#[cold]
#[inline(never)]
fn full_state_warning(
    tag: &str,
    diagnostic: Option<webui_projection::ProjectionDiagnostic>,
    mapped: bool,
) -> Diagnostic {
    let (reason, help) = if let Some(diagnostic) = diagnostic {
        (
            format!("{} ({})", diagnostic.module, diagnostic.code),
            format!(
                "{} Use the canonical framework import and unambiguous relative .ts or .js imports for state declarations.",
                diagnostic.help
            ),
        )
    } else if mapped {
        (
            "an explicit component module overrides discovery".into(),
            "Use a same-named component script instead of --component-module to enable automatic state projection.".into(),
        )
    } else {
        (
            "no discovered component source".into(),
            "Place the defining class in the component's same-named .ts or .js module.".into(),
        )
    };
    Diagnostic::warning(format!("sending full state for <{tag}>: {reason}"))
        .code(FULL_STATE_WARNING)
        .component(tag)
        .help(help)
}

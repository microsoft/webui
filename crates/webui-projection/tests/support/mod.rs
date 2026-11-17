// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

use std::collections::{BTreeMap, BTreeSet};

use webui_projection::{analyze, analyze_with_report, Import, Module, ModuleGraph};

pub const FRAMEWORK: &str = "@microsoft/webui-framework";

pub fn edge(specifier: &str, resolved: &str) -> Import {
    Import {
        specifier: specifier.into(),
        resolved: resolved.into(),
        dynamic: false,
        external: false,
        package_name: None,
    }
}

pub fn framework() -> Import {
    Import {
        external: true,
        package_name: Some(FRAMEWORK.into()),
        ..edge(FRAMEWORK, FRAMEWORK)
    }
}

pub fn module(source: &str, imports: Vec<Import>) -> Module {
    Module {
        source: Some(source.into()),
        imports,
        package_name: None,
        ..Default::default()
    }
}

pub fn graph(modules: impl IntoIterator<Item = (&'static str, Module)>) -> ModuleGraph {
    let modules: BTreeMap<String, Module> = modules
        .into_iter()
        .map(|(id, module)| (id.into(), module))
        .collect();
    ModuleGraph {
        entries: modules.keys().cloned().collect(),
        modules,
    }
}

pub fn single(source: &str) -> ModuleGraph {
    graph([("/app/card.ts", module(source, vec![framework()]))])
}

pub fn keys(names: &[&str]) -> BTreeSet<String> {
    names.iter().map(|name| (*name).into()).collect()
}

/// Assert the complete surface, not just one component or a subset of its keys.
/// A second invocation protects deterministic, build-local analysis.
pub fn success(graph: &ModuleGraph, expected: &[(&str, &str, &[&str])]) {
    for _ in 0..2 {
        let report = analyze_with_report(graph);
        assert!(report.diagnostics.is_empty(), "{:?}", report.diagnostics);
        assert_eq!(
            report
                .components
                .keys()
                .map(String::as_str)
                .collect::<Vec<_>>(),
            expected.iter().map(|(tag, _, _)| *tag).collect::<Vec<_>>()
        );
        for (tag, module, names) in expected {
            let component = &report.components[*tag];
            assert_eq!(component.module, *module, "{tag}");
            assert_eq!(component.hydration_keys, keys(names), "{tag}");
            assert_eq!(component.navigation_keys, keys(names), "{tag}");
        }
    }
}

/// Error codes are a multiset: duplicate diagnostics cannot disappear unnoticed.
pub fn failure(graph: &ModuleGraph, expected: &[&str]) {
    let report = analyze_with_report(graph);
    let mut actual: Vec<_> = report.diagnostics.iter().map(|error| error.code).collect();
    let mut expected = expected.to_vec();
    actual.sort_unstable();
    expected.sort_unstable();
    assert_eq!(actual, expected, "{:?}", report.diagnostics);
    for error in &report.diagnostics {
        assert!(!error.help.is_empty(), "{error:?}");
        assert!(!error.help.contains('\u{1b}'), "{error:?}");
    }
    assert!(
        analyze(graph).is_err(),
        "strict API must reject failed proof"
    );
}

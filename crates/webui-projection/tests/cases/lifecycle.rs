// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

//! Native counterparts of projection-parser.test.ts's semantic/laziness tests.
//! Snapshot prepare/close and TypeScript API mocks are deliberately not ported.

use std::collections::BTreeSet;

use webui_projection::{analyze_with_report, AnalysisReport, ModuleGraph};

use crate::support::{edge, failure, framework, graph, keys, module, single, success};

fn parsed_once(report: &AnalysisReport, expected: BTreeSet<String>) {
    assert_eq!(report.parsed_modules.len(), expected.len());
    assert_eq!(
        report
            .parsed_modules
            .iter()
            .cloned()
            .collect::<BTreeSet<_>>(),
        expected
    );
}

fn add_card(graph: &mut ModuleGraph, index: usize, base: &str, key: &str) {
    graph.modules.insert(
        format!("/app/card-{index}.ts"),
        module(
            &format!("import {{ Base }} from './base.js'; class Card extends Base {{}} Card.define('card-{index}');"),
            vec![edge("./base.js", base)],
        ),
    );
    graph.modules.entry(base.into()).or_insert_with(|| module(
        &format!("import {{ WebUIElement, observable }} from '@microsoft/webui-framework'; export class Base extends WebUIElement {{ @observable {key} = ''; }}"),
        vec![framework()],
    ));
}

#[test]
fn candidates_and_shared_dependencies_parse_once_but_unused_invalid_source_stays_lazy() {
    let mut graph = ModuleGraph::default();
    for index in 0..20 {
        add_card(&mut graph, index, "/app/base.ts", "value");
    }
    graph.entries.insert("/app/card-0.ts".into());
    graph
        .modules
        .insert("/app/unused.ts".into(), module("invalid syntax {", vec![]));
    let report = analyze_with_report(&graph);
    assert!(report.diagnostics.is_empty(), "{:?}", report.diagnostics);
    assert_eq!(report.components.len(), 20);
    for component in report.components.values() {
        assert_eq!(component.hydration_keys, keys(&["value"]));
        assert_eq!(component.navigation_keys, component.hydration_keys);
    }
    let expected = (0..20)
        .map(|index| format!("/app/card-{index}.ts"))
        .chain(["/app/base.ts".into()])
        .collect();
    parsed_once(&report, expected);
}

#[test]
fn graph_without_definition_candidates_never_parses() {
    let graph = graph([("/app/data.ts", module("export const value = 1;", vec![]))]);
    let report = analyze_with_report(&graph);
    assert!(report.diagnostics.is_empty());
    assert!(report.components.is_empty());
    assert!(report.parsed_modules.is_empty());
}

#[test]
fn each_distinct_base_parses_once_without_parsing_unrelated_side_effect_imports() {
    let mut graph = ModuleGraph::default();
    for index in 0..20 {
        add_card(
            &mut graph,
            index,
            &format!("/app/base-{index}.ts"),
            &format!("value{index}"),
        );
        let card = graph.modules.get_mut(&format!("/app/card-{index}.ts"));
        let Some(card) = card else {
            panic!("missing card fixture")
        };
        if let Some(source) = card.source.as_mut() {
            source.push_str("import './unused.js';");
        }
        card.imports.push(edge("./unused.js", "/app/unused.ts"));
    }
    graph
        .modules
        .insert("/app/unused.ts".into(), module("invalid syntax {", vec![]));
    let report = analyze_with_report(&graph);
    assert!(report.diagnostics.is_empty(), "{:?}", report.diagnostics);
    assert_eq!(report.components.len(), 20);
    for index in 0..20 {
        let component = &report.components[&format!("card-{index}")];
        assert_eq!(component.hydration_keys, keys(&[&format!("value{index}")]));
        assert_eq!(component.navigation_keys, component.hydration_keys);
    }
    let expected = graph
        .modules
        .keys()
        .filter(|id| id.as_str() != "/app/unused.ts")
        .cloned()
        .collect();
    parsed_once(&report, expected);
}

#[test]
fn syntax_diagnostics_from_a_deferred_dependency_are_not_replaced_by_a_generic_failure() {
    let graph = graph([
        ("/app/card.ts", module("import { observable } from '@microsoft/webui-framework'; import { Base } from './broken.js'; class Card extends Base { @observable value = ''; } Card.define('broken-card');", vec![framework(), edge("./broken.js", "/app/broken.ts")])),
        ("/app/broken.ts", module("export class Base {", vec![])),
    ]);
    failure(&graph, &["PROJ-C001", "PROJ-C006"]);
    let report = analyze_with_report(&graph);
    parsed_once(
        &report,
        ["/app/card.ts".into(), "/app/broken.ts".into()].into(),
    );
    assert_eq!(report.diagnostics[0].module, "/app/broken.ts");
    assert!(report.components.is_empty());
}

#[test]
fn export_fallbacks_do_not_cache_a_result_before_deferred_modules_are_known() {
    success(
        &graph([
            ("/app/card.ts", module("import { observable } from './barrel.js'; import { WebUIElement } from '@microsoft/webui-framework'; class Card extends WebUIElement { @observable value = ''; } Card.define('fallback-card');", vec![framework(), edge("./barrel.js", "/app/barrel.ts")])),
            ("/app/barrel.ts", module("export * from './fallback.js'; export * from './preferred.js'; // define", vec![edge("./fallback.js", "/app/fallback.ts"), edge("./preferred.js", "/app/preferred.ts")])),
            ("/app/fallback.ts", module("export { observable } from '@microsoft/webui-framework'; // define", vec![framework()])),
            ("/app/preferred.ts", module("export function observable() {}", vec![])),
        ]),
        &[("fallback-card", "/app/card.ts", &[])],
    );
}

#[test]
fn same_module_identity_uses_fresh_source_and_symbols_on_every_build() {
    for field in ["before", "after", "before"] {
        let graph = single(&format!("import {{ WebUIElement, observable }} from '@microsoft/webui-framework'; class Card extends WebUIElement {{ @observable {field} = ''; }} Card.define('changing-card');"));
        success(&graph, &[("changing-card", "/app/card.ts", &[field])]);
        parsed_once(&analyze_with_report(&graph), ["/app/card.ts".into()].into());
    }
}

#[test]
fn failed_parse_does_not_poison_a_fresh_build_at_the_same_identity() {
    let broken = single("class Broken { define( ");
    failure(&broken, &["PROJ-C001"]);
    parsed_once(
        &analyze_with_report(&broken),
        ["/app/card.ts".into()].into(),
    );
    let fresh = single("import { WebUIElement, observable } from '@microsoft/webui-framework'; class Card extends WebUIElement { @observable current = ''; } Card.define('fresh-card');");
    success(&fresh, &[("fresh-card", "/app/card.ts", &["current"])]);
    parsed_once(&analyze_with_report(&fresh), ["/app/card.ts".into()].into());
    failure(&broken, &["PROJ-C001"]);
}

#[test]
fn simultaneous_builds_with_identical_module_ids_are_isolated() {
    std::thread::scope(|scope| {
        for field in ["first", "second"] {
            scope.spawn(move || {
                let graph = single(&format!("import {{ WebUIElement, observable }} from '@microsoft/webui-framework'; class Card extends WebUIElement {{ @observable {field} = ''; }} Card.define('concurrent-card');"));
                success(&graph, &[("concurrent-card", "/app/card.ts", &[field])]);
            });
        }
    });
}

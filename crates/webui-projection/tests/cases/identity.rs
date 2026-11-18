// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

use webui_projection::{analyze_with_report, Import, Module};

use crate::support::{edge, failure, framework, graph, module, single, success, FRAMEWORK};

const CARD: &str = "
    import { WebUIElement, observable } from '@microsoft/webui-framework';
    class Card extends WebUIElement { @observable value = ''; }
    Card.define('identity-card');";

#[test]
fn package_spelling_is_never_framework_identity_without_authoritative_evidence() {
    for imports in [
        vec![],
        vec![Import {
            package_name: None,
            ..framework()
        }],
        vec![Import {
            dynamic: true,
            ..framework()
        }],
    ] {
        success(&graph([("/app/card.ts", module(CARD, imports))]), &[]);
    }
}

#[test]
fn same_named_symbols_in_a_resolved_nonframework_package_are_unrelated() {
    // This is the semantic half of the esbuild fake-framework alias regression.
    for package in [None, Some("not-the-framework".into())] {
        let target = Module {
            package_name: package,
            ..module(
                "export function observable(): void {} export class WebUIElement {}",
                vec![],
            )
        };
        success(
            &graph([
                (
                    "/app/card.ts",
                    module(CARD, vec![edge(FRAMEWORK, "/vendor/fake.ts")]),
                ),
                ("/vendor/fake.ts", target),
            ]),
            &[],
        );
    }
}

#[test]
fn target_package_identity_can_authoritatively_prove_an_aliased_import() {
    let target = Module {
        package_name: Some(FRAMEWORK.into()),
        ..module("export {};", vec![])
    };
    success(
        &graph([
            ("/app/card.ts", module(
                "import { WebUIElement, observable } from '#ui'; class Card extends WebUIElement { @observable value = ''; } Card.define('identity-card');",
                vec![edge("#ui", "/vendor/ui/index.ts")],
            )),
            ("/vendor/ui/index.ts", target),
        ]),
        &[("identity-card", "/app/card.ts", &["value"])],
    );
}

#[test]
fn identical_static_edges_are_not_a_conflict_and_dynamic_edges_do_not_override_them() {
    let mut dynamic = edge(FRAMEWORK, "/vendor/dynamic.ts");
    dynamic.dynamic = true;
    for imports in [
        vec![framework(), framework(), dynamic],
        vec![
            Import {
                dynamic: true,
                ..edge(FRAMEWORK, "/vendor/dynamic.ts")
            },
            framework(),
            framework(),
        ],
    ] {
        success(
            &graph([
                ("/app/card.ts", module(CARD, imports)),
                ("/vendor/dynamic.ts", module("export {};", vec![])),
            ]),
            &[("identity-card", "/app/card.ts", &["value"])],
        );
    }
}

#[test]
fn conflicting_exact_static_edges_fail_before_parsing_in_both_orders() {
    // Isolate each dimension; a different resolution is not required for an
    // external/internal or package-identity conflict to invalidate the graph.
    for conflict in [
        Import {
            resolved: "/vendor/framework.ts".into(),
            ..framework()
        },
        Import {
            external: false,
            ..framework()
        },
        Import {
            package_name: Some("other-package".into()),
            ..framework()
        },
        Import {
            package_name: None,
            ..framework()
        },
    ] {
        for reverse in [false, true] {
            let mut imports = vec![
                framework(),
                Import {
                    specifier: conflict.specifier.clone(),
                    resolved: conflict.resolved.clone(),
                    dynamic: conflict.dynamic,
                    external: conflict.external,
                    package_name: conflict.package_name.clone(),
                },
            ];
            if reverse {
                imports.reverse();
            }
            let graph = graph([
                ("/app/card.ts", module("class Broken { define(", imports)),
                (FRAMEWORK, module("export {};", vec![])),
                ("/vendor/framework.ts", module("export {};", vec![])),
            ]);
            failure(&graph, &["PROJ-C013"]);
            let report = analyze_with_report(&graph);
            assert!(report.components.is_empty());
            assert!(report.parsed_modules.is_empty());
            assert_eq!(report.diagnostics[0].module, "/app/card.ts");
        }
    }
}

#[test]
fn edge_and_target_package_disagreement_is_rejected_before_parsing() {
    let target = Module {
        package_name: Some("not-the-framework".into()),
        ..module("export {};", vec![])
    };
    let graph = graph([
        (
            "/app/card.ts",
            module(
                CARD,
                vec![Import {
                    resolved: "/vendor/framework.ts".into(),
                    external: false,
                    ..framework()
                }],
            ),
        ),
        ("/vendor/framework.ts", target),
    ]);
    failure(&graph, &["PROJ-C013"]);
    assert!(analyze_with_report(&graph).parsed_modules.is_empty());
}

#[test]
fn graph_integrity_rejects_missing_sources_targets_and_entries_before_parse() {
    let mut no_source = single(CARD);
    no_source
        .modules
        .insert("/app/card.ts".into(), Module::default());
    let mut no_target = single(CARD);
    no_target.modules.insert(
        "/app/card.ts".into(),
        module(CARD, vec![edge("./missing.js", "/app/missing.ts")]),
    );
    let mut no_entry = single(CARD);
    no_entry.entries.insert("/app/missing.ts".into());
    for graph in [no_source, no_target, no_entry] {
        failure(&graph, &["PROJ-C013"]);
        let report = analyze_with_report(&graph);
        assert!(report.parsed_modules.is_empty());
        assert!(report.components.is_empty());
    }
}

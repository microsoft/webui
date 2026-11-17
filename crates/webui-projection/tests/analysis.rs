// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

use std::collections::{BTreeMap, BTreeSet};

use webui_projection::{analyze, analyze_with_report, Import, Module, ModuleGraph};

fn graph(source: &str) -> ModuleGraph {
    ModuleGraph {
        entries: BTreeSet::from(["/app/index.ts".into()]),
        modules: BTreeMap::from([(
            "/app/index.ts".into(),
            Module {
                source: Some(source.into()),
                imports: vec![Import {
                    specifier: "@microsoft/webui-framework".into(),
                    resolved: "@microsoft/webui-framework".into(),
                    dynamic: false,
                    external: true,
                    package_name: Some("@microsoft/webui-framework".into()),
                }],
                package_name: None,
                ..Default::default()
            },
        )]),
    }
}

#[test]
fn proves_original_reactive_properties_and_attr_options() -> Result<(), Box<dyn std::error::Error>>
{
    let result = analyze(&graph("
        import { WebUIElement as Element, observable as state, attr } from '@microsoft/webui-framework';
        class Counter extends Element { @state count = 0; @attr({attribute:'data-label'}) label = ''; }
        Counter.define('app-counter');
    "))?;
    let counter = result.get("app-counter").ok_or("missing counter")?;
    assert_eq!(
        counter.hydration_keys,
        BTreeSet::from(["count".into(), "label".into()])
    );
    assert_eq!(counter.navigation_keys, counter.hydration_keys);
    Ok(())
}

#[test]
fn preserves_namespaces_reexports_and_cross_module_inheritance(
) -> Result<(), Box<dyn std::error::Error>> {
    let mut graph = graph(
        "
        import { Base } from './base.js';
        class Counter extends Base { }
        customElements.define('app-counter', Counter);
    ",
    );
    graph
        .modules
        .get_mut("/app/index.ts")
        .ok_or("entry")?
        .imports = vec![Import {
        specifier: "./base.js".into(),
        resolved: "/app/base.ts".into(),
        dynamic: false,
        external: false,
        package_name: None,
    }];
    graph.modules.insert("/app/base.ts".into(), Module {
        source: Some("import * as ui from '@microsoft/webui-framework'; export class Base extends ui.WebUIElement { @ui.observable count = 0; }".into()),
        imports: vec![Import {
            specifier: "@microsoft/webui-framework".into(), resolved: "framework".into(), dynamic: false, external: true, package_name: Some("@microsoft/webui-framework".into()),
        }],
        package_name: None,
        ..Default::default()
    });
    let result = analyze(&graph)?;
    assert_eq!(
        result.get("app-counter").ok_or("counter")?.hydration_keys,
        BTreeSet::from(["count".into()])
    );
    Ok(())
}

#[test]
fn instantiates_nested_class_factories_without_executing_them(
) -> Result<(), Box<dyn std::error::Error>> {
    let result = analyze(&graph("
        import { WebUIElement, observable, attr } from '@microsoft/webui-framework';
        function State(Base, configuration) { return class extends Base { @observable count = 0; }; }
        function Labels(Base) { class Labelled extends Base { @attr label = ''; } return Labelled; }
        class Counter extends Labels(State(WebUIElement, { option: true })) { }
        Counter.define('app-counter');
    "))?;
    assert_eq!(
        result.get("app-counter").ok_or("counter")?.hydration_keys,
        BTreeSet::from(["count".into(), "label".into()])
    );
    Ok(())
}

#[test]
fn rejects_mutation_computed_properties_and_factory_decorator_calls() {
    for (source, code) in [
        ("import {WebUIElement,observable} from '@microsoft/webui-framework'; let C = class extends WebUIElement { @observable count=0; }; C.define('app-counter');", "PROJ-C009"),
        ("import {WebUIElement,observable} from '@microsoft/webui-framework'; class C extends WebUIElement { @observable ['count']=0; } C.define('app-counter');", "PROJ-C007"),
        ("import {WebUIElement,observable} from '@microsoft/webui-framework'; class C extends WebUIElement { @observable() count=0; } C.define('app-counter');", "PROJ-C007"),
        ("import {WebUIElement} from '@microsoft/webui-framework'; class C extends WebUIElement {} C.define(tag);", "PROJ-C008"),
    ] {
        let Err(error) = analyze(&graph(source)) else {
            panic!("invalid declaration must fail");
        };
        assert!(error.0.iter().any(|error| error.code == code), "{error:?}");
    }
}

#[test]
fn lexical_shadowing_does_not_borrow_framework_decorator_identity(
) -> Result<(), Box<dyn std::error::Error>> {
    let result = analyze(&graph(
        "
        import { WebUIElement, observable } from '@microsoft/webui-framework';
        function unrelated(observable) {
            class C { @observable count=0; }
            C.define('not-webui');
        }
        class Counter extends WebUIElement { @observable actual=0; }
        Counter.define('app-counter');
    ",
    ))?;
    assert!(!result.contains_key("not-webui"));
    assert_eq!(
        result.get("app-counter").ok_or("counter")?.hydration_keys,
        BTreeSet::from(["actual".into()])
    );
    Ok(())
}

#[test]
fn import_proof_requires_an_edge_and_authoritative_package_identity(
) -> Result<(), Box<dyn std::error::Error>> {
    let source = "import { WebUIElement, observable } from '@microsoft/webui-framework'; class Counter extends WebUIElement { @observable count=0; } Counter.define('app-counter');";
    let mut missing = graph(source);
    missing
        .modules
        .get_mut("/app/index.ts")
        .ok_or("entry")?
        .imports
        .clear();
    assert!(analyze_with_report(&missing).components.is_empty());
    let mut unrelated = graph(source);
    let edge = unrelated
        .modules
        .get_mut("/app/index.ts")
        .ok_or("entry")?
        .imports
        .first_mut()
        .ok_or("edge")?;
    edge.external = false;
    edge.resolved = "/app/unrelated.ts".into();
    edge.package_name = None;
    unrelated.modules.insert(
        "/app/unrelated.ts".into(),
        Module {
            source: Some("export class WebUIElement {} export function observable() {}".into()),
            ..Default::default()
        },
    );
    assert!(analyze_with_report(&unrelated).components.is_empty());
    Ok(())
}

#[test]
fn conflicting_static_resolutions_fail_before_analysis_in_either_order(
) -> Result<(), Box<dyn std::error::Error>> {
    for reverse in [false, true] {
        let mut graph = graph("import { WebUIElement, observable } from '@microsoft/webui-framework'; class Counter extends WebUIElement { @observable count=0; } Counter.define('app-counter');");
        let imports = &mut graph
            .modules
            .get_mut("/app/index.ts")
            .ok_or("entry")?
            .imports;
        imports.push(Import {
            specifier: "@microsoft/webui-framework".into(),
            resolved: "/app/unrelated.ts".into(),
            dynamic: false,
            external: false,
            package_name: Some("unrelated-package".into()),
        });
        if reverse {
            imports.reverse();
        }
        graph.modules.insert(
            "/app/unrelated.ts".into(),
            Module {
                source: Some("export class WebUIElement {} export function observable() {}".into()),
                package_name: Some("unrelated-package".into()),
                ..Default::default()
            },
        );
        let report = analyze_with_report(&graph);
        assert!(report.components.is_empty());
        assert!(report
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == "PROJ-C013"));
        assert!(report.parsed_modules.is_empty());
    }
    Ok(())
}

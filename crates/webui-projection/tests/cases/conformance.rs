// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

//! Semantic cases from ALL_CASES in the former Node conformance oracle.
//! Output membership, hashing and publication belong to webui::browser_build.

use crate::support::{edge, failure, framework, graph, module, single, success};

#[test]
fn aliased_and_namespace_decorators_preserve_framework_identity() {
    for (source, tag, keys) in [
        ("import { observable as obs, WebUIElement } from '@microsoft/webui-framework'; class Counter extends WebUIElement { @obs count = 0; } Counter.define('my-counter');", "my-counter", &["count"][..]),
        ("import * as webui from '@microsoft/webui-framework'; class Widget extends webui.WebUIElement { @webui.observable value = ''; } Widget.define('my-widget');", "my-widget", &["value"][..]),
    ] {
        success(&single(source), &[(tag, "/app/card.ts", keys)]);
    }
}

#[test]
fn named_and_star_reexport_chain_follows_exact_edges() {
    success(
        &graph([
            (
                "/app/framework-barrel.ts",
                module(
                    "export { observable } from '@microsoft/webui-framework';",
                    vec![framework()],
                ),
            ),
            (
                "/app/app-barrel.ts",
                module(
                    "export * from './framework-barrel.ts';",
                    vec![edge("./framework-barrel.ts", "/app/framework-barrel.ts")],
                ),
            ),
            (
                "/app/card.ts",
                module(
                    "
                import { observable } from './app-barrel.ts';
                import { WebUIElement } from '@microsoft/webui-framework';
                class Card extends WebUIElement { @observable title = ''; }
                Card.define('my-card');",
                    vec![framework(), edge("./app-barrel.ts", "/app/app-barrel.ts")],
                ),
            ),
        ]),
        &[("my-card", "/app/card.ts", &["title"])],
    );
}

#[test]
fn single_and_multihop_inheritance_union_keys_without_losing_module_ownership() {
    let mut graph = graph([
        ("/app/a.ts", module("import { observable, WebUIElement } from '@microsoft/webui-framework'; export class A extends WebUIElement { @observable aKey = ''; }", vec![framework()])),
        ("/app/b.ts", module("import { observable } from '@microsoft/webui-framework'; import { A } from './a.ts'; export class B extends A { @observable bKey = ''; } B.define('my-b');", vec![framework(), edge("./a.ts", "/app/a.ts")])),
    ]);
    success(&graph, &[("my-b", "/app/b.ts", &["aKey", "bKey"])]);
    graph.modules.insert("/app/c.ts".into(), module(
        "import { observable } from '@microsoft/webui-framework'; import { B } from './b.ts'; class C extends B { @observable cKey = ''; } C.define('my-c');",
        vec![framework(), edge("./b.ts", "/app/b.ts")],
    ));
    success(
        &graph,
        &[
            ("my-b", "/app/b.ts", &["aKey", "bKey"]),
            ("my-c", "/app/c.ts", &["aKey", "bKey", "cKey"]),
        ],
    );
}

#[test]
fn component_in_a_dynamic_chunk_is_still_discovered() {
    let mut import = edge("./detail.ts", "/app/detail.ts");
    import.dynamic = true;
    let mut graph = graph([
        ("/app/index.ts", module("import('./detail.ts');", vec![import])),
        ("/app/detail.ts", module("import { observable, WebUIElement } from '@microsoft/webui-framework'; class DetailPane extends WebUIElement { @observable item = null; } DetailPane.define('detail-pane');", vec![framework()])),
    ]);
    graph.entries = ["/app/index.ts".into()].into();
    success(&graph, &[("detail-pane", "/app/detail.ts", &["item"])]);
}

#[test]
fn two_entries_sharing_one_component_do_not_duplicate_its_definition() {
    let mut graph = graph([
        ("/app/shared.ts", module("import { attr, WebUIElement } from '@microsoft/webui-framework'; export class Shared extends WebUIElement { @attr label = ''; } Shared.define('shared-btn');", vec![framework()])),
        ("/app/a.ts", module("import './shared.ts';", vec![edge("./shared.ts", "/app/shared.ts")])),
        ("/app/b.ts", module("import './shared.ts';", vec![edge("./shared.ts", "/app/shared.ts")])),
    ]);
    graph.entries = ["/app/a.ts".into(), "/app/b.ts".into()].into();
    success(&graph, &[("shared-btn", "/app/shared.ts", &["label"])]);
}

#[test]
fn custom_elements_imported_class_reports_the_defining_not_registration_module() {
    success(
        &graph([
            ("/app/card.ts", module("import { observable, WebUIElement } from '@microsoft/webui-framework'; export class Card extends WebUIElement { @observable value = ''; }", vec![framework()])),
            ("/app/register.ts", module("import { Card as ImportedCard } from './card.ts'; customElements.define('imported-card', ImportedCard);", vec![edge("./card.ts", "/app/card.ts")])),
        ]),
        &[("imported-card", "/app/card.ts", &["value"])],
    );
}

#[test]
fn unrelated_define_api_does_not_make_a_dynamic_tag_a_projection_error() {
    success(
        &single("class Schema {} const dynamicName = getName(); Schema.define(dynamicName);"),
        &[],
    );
}

#[test]
fn nul_prefixed_virtual_module_needs_no_physical_source() {
    success(
        &graph([
            ("\0virtual:polyfill", webui_projection::Module::default()),
            (
                "/app/index.ts",
                module(
                    "import 'virtual:polyfill';",
                    vec![edge("virtual:polyfill", "\0virtual:polyfill")],
                ),
            ),
        ]),
        &[],
    );
}

#[test]
fn duplicate_tags_are_rejected_not_merged() {
    failure(
        &graph([
            ("/app/a.ts", module("import { WebUIElement } from '@microsoft/webui-framework'; class A extends WebUIElement {} A.define('duplicate-card');", vec![framework()])),
            ("/app/b.ts", module("import { WebUIElement } from '@microsoft/webui-framework'; class B extends WebUIElement {} B.define('duplicate-card');", vec![framework()])),
        ]),
        &["PROJ-C010"],
    );
}

#[test]
fn unresolved_base_cannot_publish_just_the_local_keys() {
    failure(
        &single("import { observable } from '@microsoft/webui-framework'; import { MissingBase } from './missing.ts'; class Derived extends MissingBase { @observable value = ''; } Derived.define('derived-card');"),
        &["PROJ-C005"],
    );
}

#[test]
fn dynamic_tag_requires_a_literal_even_if_the_binding_is_const() {
    failure(
        &single("import { WebUIElement } from '@microsoft/webui-framework'; const tag = 'dynamic-card'; class DynamicCard extends WebUIElement {} DynamicCard.define(tag);"),
        &["PROJ-C008"],
    );
}

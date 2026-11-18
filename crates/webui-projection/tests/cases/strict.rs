// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

use crate::support::{edge, failure, framework, graph, module, single, success};

#[test]
fn known_nonframework_decorators_do_not_contribute_keys() {
    success(
        &single(
            "
            import { observable, WebUIElement } from '@microsoft/webui-framework';
            function localDecorator(_target: object, _name: string): void {}
            class Decorated extends WebUIElement {
              @localDecorator ordinary = '';
              @localDecorator @observable value = '';
            }
            Decorated.define('decorated-card');",
        ),
        &[("decorated-card", "/app/card.ts", &["value"])],
    );
}

#[test]
fn locally_shadowed_custom_elements_registry_is_not_the_global_registry() {
    success(
        &single(
            "
            import { WebUIElement } from '@microsoft/webui-framework';
            class Card extends WebUIElement {}
            const customElements = { define() {} };
            customElements.define('not-global-card', Card);",
        ),
        &[],
    );
}

#[test]
fn comments_and_interpolated_templates_do_not_hide_definition_calls() {
    for (source, tag) in [
        ("import { WebUIElement } from '@microsoft/webui-framework'; class Card extends WebUIElement {} Card.define /* keep this comment */ ('commented-card');", "commented-card"),
        ("import { WebUIElement } from '@microsoft/webui-framework'; const label = `card-${String(1)}`; class Card extends WebUIElement {} Card.define('template-card'); void label;", "template-card"),
    ] {
        success(&single(source), &[(tag, "/app/card.ts", &[])]);
    }
}

#[test]
fn immutable_class_and_decorator_aliases_keep_original_identity() {
    success(
        &single(
            "
            import { WebUIElement, observable } from '@microsoft/webui-framework';
            const obs = observable;
            class Card extends WebUIElement { @obs value = ''; }
            const Alias = Card;
            Alias.define('aliased-card');",
        ),
        &[("aliased-card", "/app/card.ts", &["value"])],
    );
}

#[test]
fn adapter_target_is_authoritative_even_when_extension_and_filename_differ() {
    success(
        &graph([
            ("/app/models/actual.ts", module("import { WebUIElement, observable } from '@microsoft/webui-framework'; export class Base extends WebUIElement { @observable baseValue = ''; }", vec![framework()])),
            ("/app/base.ts", module("export class Base {}", vec![])),
            ("/app/derived.ts", module("import { Base } from './base.js'; class Derived extends Base {} Derived.define('resolved-card');", vec![edge("./base.js", "/app/models/actual.ts")])),
        ]),
        &[("resolved-card", "/app/derived.ts", &["baseValue"])],
    );
}

#[test]
fn mutable_class_expression_is_not_an_exact_constructor_reference() {
    failure(
        &single("import { WebUIElement, observable } from '@microsoft/webui-framework'; let Card = class extends WebUIElement { @observable stale = ''; }; Card.define('mutable-card');"),
        &["PROJ-C009"],
    );
}

#[test]
fn reactive_properties_require_plain_identifiers_not_computed_or_literal_names() {
    for field in [
        "@observable ['count'] = 0;",
        "@observable [name] = 0;",
        "@observable 'count' = 0;",
        "@observable 1 = 0;",
    ] {
        failure(
            &single(&format!("import {{ WebUIElement, observable }} from '@microsoft/webui-framework'; class Card extends WebUIElement {{ {field} }} Card.define('strict-card');")),
            &["PROJ-C007"],
        );
    }
}

#[test]
fn ambiguous_decorators_and_observable_factory_calls_fail_without_partial_keys() {
    for (field, code) in [
        ("@observable() value = '';", "PROJ-C007"),
        ("@observable({}) value = '';", "PROJ-C007"),
        ("@(ui['observable']) value = '';", "PROJ-C007"),
        ("@((() => observable)()) value = '';", "PROJ-C007"),
    ] {
        let graph = single(&format!(
            "
            import {{ WebUIElement, observable }} from '@microsoft/webui-framework';
            import * as ui from '@microsoft/webui-framework';
            class Card extends WebUIElement {{ @observable valid = ''; {field} }}
            Card.define('strict-card');"
        ));
        failure(&graph, &[code]);
        assert!(webui_projection::analyze_with_report(&graph)
            .components
            .is_empty());
    }
}

#[test]
fn unresolved_bare_decorator_uses_the_canonical_identity_diagnostic() {
    failure(
        &single("import { WebUIElement } from '@microsoft/webui-framework'; class Card extends WebUIElement { @missing value = ''; } Card.define('unresolved-card');"),
        &["PROJ-C004"],
    );
}

#[test]
fn exact_keys_are_deduplicated_and_keep_original_unicode_identifiers() {
    success(
        &single(
            "
            import { WebUIElement, observable, attr } from '@microsoft/webui-framework';
            class Base extends WebUIElement { @observable shared = ''; }
            class Card extends Base {
              @observable shared = '';
              @attr({ attribute: 'data-renamed' }) firstName = '';
              @observable é = '';
              @observable $value = '';
              @observable _value = '';
              ordinary = '';
            }
            Card.define('exact-card');",
        ),
        &[(
            "exact-card",
            "/app/card.ts",
            &["$value", "_value", "firstName", "shared", "é"],
        )],
    );
}

#[test]
fn attr_options_and_initializers_are_not_executed_to_prove_property_names() {
    success(
        &single(
            "
            import { WebUIElement, observable, attr } from '@microsoft/webui-framework';
            class Card extends WebUIElement {
              @attr({ attribute: failIfExecuted() }) label = failIfExecuted();
              @observable count = failIfExecuted();
            }
            Card.define('inert-card');",
        ),
        &[("inert-card", "/app/card.ts", &["count", "label"])],
    );
}

#[test]
fn malformed_definition_arity_does_not_publish_a_surface() {
    for definition in [
        "Card.define('arity-card', {});",
        "customElements.define('arity-card', Card, {});",
    ] {
        failure(
            &single(&format!("import {{ WebUIElement }} from '@microsoft/webui-framework'; class Card extends WebUIElement {{}} {definition}")),
            &["PROJ-C011"],
        );
    }
}

#[test]
fn inheritance_cycles_fail_with_the_canonical_cycle_diagnostic() {
    failure(
        &single(
            "
            import { observable } from '@microsoft/webui-framework';
            class A extends B { @observable a = ''; }
            class B extends A { @observable b = ''; }
            A.define('cycle-card');",
        ),
        &["PROJ-C012"],
    );
}

#[test]
fn cyclic_reexports_cannot_borrow_a_decorator_identity() {
    failure(
        &graph([
            ("/app/a.ts", module("export { observable } from './b.js';", vec![edge("./b.js", "/app/b.ts")])),
            ("/app/b.ts", module("export { observable } from './a.js';", vec![edge("./a.js", "/app/a.ts")])),
            ("/app/card.ts", module("import { WebUIElement } from '@microsoft/webui-framework'; import { observable } from './a.js'; class Card extends WebUIElement { @observable value = ''; } Card.define('cycle-card');", vec![framework(), edge("./a.js", "/app/a.ts")])),
        ]),
        &["PROJ-C012"],
    );
}

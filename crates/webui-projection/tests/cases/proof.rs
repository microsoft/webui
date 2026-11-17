// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

//! Semantic halves of projection-proof.test.ts and projection-mixins.test.ts.
//! Manifest coverage and rendered state assertions remain consumer contracts.

use crate::support::{edge, failure, framework, graph, module, single, success};

const FACTORY: &str = include_str!("../fixtures/proof_factory.ts");

#[test]
fn imported_constructor_assignments_cannot_claim_the_preassignment_base_keys() {
    for before in ["", "const Alias = Base;"] {
        let constructor = if before.is_empty() { "Base" } else { "Alias" };
        failure(
            &graph([
                (
                    "/app/models.ts",
                    module(
                        include_str!("../fixtures/proof_models.ts"),
                        vec![framework()],
                    ),
                ),
                (
                    "/app/card.ts",
                    module(
                        &format!(
                            "
                    import {{ First, Other, mix }} from './models.js';
                    class Base extends First {{}}
                    {before}
                    class Derived extends mix(Base = Other, {constructor}) {{}}
                    Derived.define('probe-card');"
                        ),
                        vec![edge("./models.js", "/app/models.ts")],
                    ),
                ),
            ]),
            &["PROJ-C005"],
        );
    }
}

#[test]
fn shadowed_registration_factory_does_not_claim_a_module_factory_surface() {
    // The full build's PROJ-B001 is missing-coverage validation, not an analyzer
    // diagnostic. At this boundary the critical invariant is no false proof.
    success(
        &single(&format!(
            "{FACTORY}
            function other(Base) {{
              class Other extends Base {{ @observable actual = ''; }}
              return Other;
            }}
            function register(mix) {{
              customElements.define('probe-card', class extends mix(WebUIElement) {{}});
            }}
            register(other);"
        )),
        &[],
    );
}

#[test]
fn module_mutation_in_evaluation_scopes_rejects_stale_factory_keys() {
    for statement in [
        "function replace(value = (mix = other)) { var mix; } replace();",
        "class Effects { [(mix = other, 'method')](mix) {} }",
        "(mix as any) = other;",
        "mix! = other;",
    ] {
        failure(
            &single(&format!(
                "{FACTORY}
                function other(Base) {{
                  class Other extends Base {{ @observable actual = ''; }}
                  return Other;
                }}
                {statement}
                class Derived extends mix(WebUIElement) {{ @observable own = ''; }}
                Derived.define('probe-card');"
            )),
            &["PROJ-C005"],
        );
    }
}

#[test]
fn reads_and_unrelated_mutations_keep_precise_proof_surfaces() {
    for statement in [
        "for (const { factory = mix } of [{}]) { void factory; }",
        "for (const { mix: selected } of [{ mix }]) { void selected; }",
        "function unrelated(mix) { mix = null; }",
        "class Holder { static { var mix; mix = null; } }",
    ] {
        success(
            &single(&format!(
                "{FACTORY}
                {statement}
                class Derived extends mix(WebUIElement) {{ @observable own = ''; }}
                Derived.define('probe-card');"
            )),
            &[("probe-card", "/app/card.ts", &["inherited", "own"])],
        );
    }
}

#[test]
fn imported_generic_mixins_include_only_reactive_inherited_and_local_state() {
    for (imports, base, expected) in [
        ("", "WebUIElement", &["label", "ownValue"][..]),
        (
            "import { withCounter } from './mixins.js';",
            "withCounter(WebUIElement)",
            &["inheritedValue", "label", "ownValue"][..],
        ),
        (
            "import { withCaption, withCounter } from './mixins.js';",
            "withCaption(withCounter(WebUIElement), { caption: 'Default caption' })",
            &["inheritedLabel", "inheritedValue", "label", "ownValue"][..],
        ),
    ] {
        let mut imports_graph = vec![framework()];
        if !imports.is_empty() {
            imports_graph.push(edge("./mixins.js", "/app/mixins.ts"));
        }
        success(
            &graph([
                (
                    "/app/mixins.ts",
                    module(include_str!("../fixtures/mixins.ts"), vec![framework()]),
                ),
                (
                    "/app/probe-mixin.ts",
                    module(
                        &format!(
                            "
                    import {{ WebUIElement, attr, observable }} from '@microsoft/webui-framework';
                    {imports}
                    class ProbeMixin extends {base} {{
                      @observable ownValue = 0;
                      @attr label = '';
                      increment(): void {{ this.ownValue += 1; }}
                    }}
                    ProbeMixin.define('probe-mixin');"
                        ),
                        imports_graph,
                    ),
                ),
            ]),
            &[("probe-mixin", "/app/probe-mixin.ts", expected)],
        );
    }
}

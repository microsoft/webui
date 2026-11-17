// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

//! The 71 source scenarios from projection-class-factory-cases.ts. This harness
//! constructs resolved graphs only; no source execution or resolution is used.

use crate::support::{self, edge, framework, graph, module};

const NAMED: &str = include_str!("../fixtures/named_factory.ts");
const DIRECT: &str = "export function mix(Base) { return class extends Base {}; }";
const NONFIRST: &str = "export function mix(config, Base) { return class Named extends Base {}; }";
const FULL_KEYS: &[&str] = &["base", "inherited", "own"];

struct Case<'a> {
    name: &'a str,
    source: &'a str,
    tail: &'a str,
    base: &'a str,
    fields: &'a str,
    before: &'a str,
    definition: Option<&'a str>,
    keys: Option<&'a [&'a str]>,
    first_keys: Option<&'a [&'a str]>,
}

impl<'a> Case<'a> {
    fn new(name: &'a str, source: &'a str) -> Self {
        Self {
            name,
            source,
            tail: "",
            base: "mix(WebUIElement)",
            fields: "@observable own = '';",
            before: "",
            definition: None,
            keys: None,
            first_keys: None,
        }
    }

    fn verify(&self) {
        let definition = self.definition.map(str::to_owned).unwrap_or_else(|| {
            format!(
                "class Derived extends {} {{ {} }} Derived.define('mixin-card');",
                self.base, self.fields
            )
        });
        let graph = graph([
            (
                "/app/factory.ts",
                module(
                    &format!(
                        "import {{ WebUIElement, observable, attr }} from '@microsoft/webui-framework';\n{}\n{}",
                        self.source, self.tail
                    ),
                    vec![framework()],
                ),
            ),
            (
                "/app/barrel.ts",
                module(
                    "export { mix } from './factory.js';",
                    vec![edge("./factory.js", "/app/factory.ts")],
                ),
            ),
            (
                "/app/derived.ts",
                module(
                    &format!(
                        "import {{ WebUIElement, observable }} from '@microsoft/webui-framework';
                        import * as factories from './barrel.js';
                        const mix = factories.mix;
                        class Base extends WebUIElement {{ @observable base = ''; }}
                        class Other extends WebUIElement {{ @observable actual = ''; }}
                        {}\n{definition}", self.before
                    ),
                    vec![framework(), edge("./barrel.js", "/app/barrel.ts")],
                ),
            ),
        ]);
        if let Some(keys) = self.keys {
            let mut expected = Vec::new();
            if let Some(first) = self.first_keys {
                expected.push(("first-card", "/app/derived.ts", first));
            }
            expected.push(("mixin-card", "/app/derived.ts", keys));
            support::success(&graph, &expected);
        } else {
            let report = webui_projection::analyze_with_report(&graph);
            assert!(!report.diagnostics.is_empty(), "{} accepted", self.name);
            assert!(
                report
                    .diagnostics
                    .iter()
                    .all(|error| matches!(error.code, "PROJ-C004" | "PROJ-C005")),
                "{}: {:?}",
                self.name,
                report.diagnostics
            );
            assert!(
                !report.components.contains_key("mixin-card"),
                "{}: partial proof",
                self.name
            );
            assert!(webui_projection::analyze(&graph).is_err(), "{}", self.name);
        }
    }
}

#[test]
fn factory_shapes_configuration_and_repeated_instantiations() {
    for (name, source, base, fields, keys) in [
        (
            "named reexport, namespace and immutable alias",
            NAMED,
            "mix(Base, { value: 'initial' })",
            "@observable own = '';",
            FULL_KEYS,
        ),
        (
            "inherited-only state",
            NAMED,
            "factories.mix(WebUIElement, { value: 'initial' })",
            "",
            &["inherited"],
        ),
        (
            "repeated factory retains the supplied base",
            NAMED,
            "mix(mix(Base, { value: 'inner' }), { value: 'outer' })",
            "@observable own = '';",
            FULL_KEYS,
        ),
        (
            "direct returned class expression",
            DIRECT,
            "mix(Base)",
            "@observable own = '';",
            &["base", "own"],
        ),
        ("empty state", DIRECT, "mix(WebUIElement)", "", &[]),
        (
            "inline base class",
            NAMED,
            "mix(class extends Base {}, { value: '' })",
            "@observable own = '';",
            FULL_KEYS,
        ),
        (
            "nonfirst constructor parameter",
            NONFIRST,
            "mix({ value: 'initial' }, Base)",
            "@observable own = '';",
            &["base", "own"],
        ),
    ] {
        Case {
            base,
            fields,
            keys: Some(keys),
            ..Case::new(name, source)
        }
        .verify();
    }
}

#[test]
fn reads_and_writes_of_unrelated_shadow_bindings_preserve_factory_identity() {
    for tail in [
        "let other; ({ other = mix } = {});",
        "for (const { factory = mix } of [{}]) { void factory; }",
        "for (const { mix: selected } of []) { void selected; }",
        "for (const [selected = mix] of []) { void selected; }",
        "for (const { mix } of []) { void mix; }",
        "function local(mix) { mix = null; }",
        "{ let mix; mix = null; }",
        "try {} catch (mix) { mix = null; }",
        "class Holder { static { var mix; mix = replacement; } }",
        "function local(mix = replacement) { var mix; mix = null; }",
    ] {
        Case {
            tail,
            base: "mix(Base, { value: 'initial' })",
            keys: Some(FULL_KEYS),
            ..Case::new(tail, NAMED)
        }
        .verify();
    }
}

#[test]
fn module_factory_mutations_invalidate_proof_including_erased_typescript_targets() {
    for tail in [
        "mix = replacement;",
        "export function replace() { mix = replacement; }",
        "({ factory: mix } = replacement);",
        "({ mix } = replacement);",
        "[mix] = replacements;",
        "mix++;",
        "for (mix of replacements) {}",
        "for ({ factory: mix } of replacements) {}",
        "for (var mix of replacements) {}",
        "for (const { factory = (mix = replacement) } of [{}]) {}",
        "(mix as any) = replacement;",
        "mix! = replacement;",
        "(<any>mix) = replacement;",
        "(mix satisfies unknown) = replacement;",
        "[(mix as any)] = replacements;",
        "function replace(value = (mix = replacement)) { var mix; } replace();",
        "class Effects { [(mix = replacement, 'method')](mix) {} }",
    ] {
        Case {
            tail,
            ..Case::new(tail, NAMED)
        }
        .verify();
    }
}

#[test]
fn unsupported_factory_shapes_never_publish_partial_state() {
    for source in [
        "export function mix(Base) { if (flag) return Base; return class extends Base {}; }",
        "export function mix(Base) { Base = Other; return class extends Base {}; }",
        "export function mix(Base, config) { return class extends config.base {}; }",
        "export function mix(Base, observable) { class Mixed extends Base { @observable value = ''; } return Mixed; }",
        "export function mix(Base) { class observable extends Base { @observable value = ''; } return observable; }",
        "export function mix(Base) { @replace class Mixed extends Base {} return Mixed; }",
        "export function mix(Base) { return class extends Base { static { replace(); } }; }",
        "export function mix(Base) { return class extends Base { static value = replace(); }; }",
        "export function mix(Base = replacement()) { return class extends Base {}; }",
        "export function mix(...Base) { return class extends Base {}; }",
        "export function mix({ Base }) { return class extends Base {}; }",
        "export async function mix(Base) { return class extends Base {}; }",
        "export function* mix(Base) { return class extends Base {}; }",
        "export function mix(Base) { return mix(Base); }",
        "export function mix(Base) { return externalFactory(Base); }",
        "export class mix extends WebUIElement {}",
    ] {
        Case::new(source, source).verify();
    }
}

#[test]
fn missing_spread_and_optional_factory_arguments_are_unproven() {
    for base in ["mix()", "mix(...bases)", "mix?.(WebUIElement)"] {
        Case {
            base,
            ..Case::new(base, NAMED)
        }
        .verify();
    }
}

#[test]
fn constructor_mutations_in_other_arguments_aliases_and_closures_are_unproven() {
    for (before, base) in [
        ("", "mix(Base = Other, Base)"),
        ("const Alias = Base;", "mix(Base = Other, Alias)"),
        ("function change() { Base = Other; }", "mix(change(), Base)"),
        ("Base = Other;", "mix({}, Base)"),
    ] {
        Case {
            before,
            base,
            ..Case::new(base, NONFIRST)
        }
        .verify();
    }
}

#[test]
fn call_site_scope_never_borrows_a_module_factory_or_decorator() {
    for wrapper in [
        "function register(mix) { BODY }",
        "const register = (mix) => { BODY };",
        "const register = function mix() { BODY };",
        "{ BODY let mix = replacement; }",
        "function register() { BODY var mix; }",
        "try {} catch (mix) { BODY }",
        "for (const mix of replacements) { BODY }",
        "function register({ mix }) { BODY }",
        "function register(factories) { BODY }",
        "function register(WebUIElement) { BODY }",
        "function register(observable) { BODY }",
    ] {
        let callee = if wrapper.contains("(factories)") {
            "factories.mix"
        } else {
            "mix"
        };
        let body = format!(
            "customElements.define('mixin-card', class extends {callee}(WebUIElement, {{ value: '' }}) {{ @observable own = ''; }});"
        );
        let definition = wrapper.replace("BODY", &body);
        Case {
            definition: Some(&definition),
            ..Case::new(wrapper, NAMED)
        }
        .verify();
    }
}

#[test]
fn unshadowed_registration_in_a_function_is_proven() {
    Case {
        definition: Some("function register() { customElements.define('mixin-card', class extends mix(WebUIElement, { value: '' }) {}); }"),
        keys: Some(&["inherited"]), ..Case::new("unshadowed registration inside function", NAMED)
    }.verify();
}

#[test]
fn shared_factory_properties_do_not_cache_the_supplied_base() {
    Case {
        definition: Some(
            "
            class First extends mix(Base, { value: '' }) {}
            First.define('first-card');
            class Second extends mix(Other, { value: '' }) { @observable own = ''; }
            Second.define('mixin-card');",
        ),
        keys: Some(&["actual", "inherited", "own"]),
        first_keys: Some(&["base", "inherited"]),
        ..Case::new("per-instantiation base", NAMED)
    }
    .verify();
}

#[test]
fn resolved_module_identity_does_not_bypass_later_lexical_shadowing() {
    Case {
        definition: Some("
            class First extends mix(WebUIElement, { value: '' }) {}
            First.define('first-card');
            function register(mix) {
              customElements.define('mixin-card', class extends mix(WebUIElement) { @observable own = ''; });
            }"),
        ..Case::new("cached module identity does not bypass lexical shadowing", NAMED)
    }.verify();
}

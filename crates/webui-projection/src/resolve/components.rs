// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

use super::*;
use std::path::PathBuf;

struct DecoratorUse {
    identity: Identity,
    called: bool,
    literal_arguments: bool,
}

impl DecoratorUse {
    fn is_neutral(&self, method: bool) -> bool {
        matches!(&self.identity, Identity::NeutralDecorator(effect)
            if effect.factory == self.called
                && (!self.called || self.literal_arguments)
                && (method || !effect.method_only))
    }
}

impl<'a> Compiler<'a> {
    pub(crate) fn source_component(
        &mut self,
        scope: PathBuf,
        module: &str,
        tag: &str,
    ) -> Result<ComponentSurface, ProjectionDiagnostic> {
        let Evidence::Source(evidence) = &mut self.evidence else {
            return Err(unproven_definition(module));
        };
        evidence.scope = scope;
        self.diagnostics.clear();
        self.environments.clear();
        if !self.prepare(module) {
            return Err(self
                .diagnostics
                .pop()
                .unwrap_or_else(|| unproven_definition(module)));
        }
        let mut sites = self.analyses[module].definitions.iter();
        let Some(site) = sites.find(|site| site.tag.as_deref() == Some(tag)) else {
            return Err(unproven_definition(module));
        };
        if !site.top_level
            || !site.arguments_valid
            || sites.any(|site| site.tag.as_deref() == Some(tag))
            || self.analyses[module]
                .definitions
                .iter()
                .any(|site| site.tag.is_none())
        {
            return Err(unproven_definition(module));
        }
        let target = match self.resolve(Query::Value(Target {
            module: module.to_owned(),
            value: site.class,
            environment: None,
        })) {
            Identity::Class(target) if target.module == module => target,
            _ => return Err(unproven_definition(module)),
        };
        let (is_webui, keys, errors) = self.lineage(&target);
        if let Some(error) = self.diagnostics.pop().or_else(|| errors.into_iter().next()) {
            return Err(error);
        }
        if !is_webui {
            return Err(unproven_definition(module));
        }
        Ok(ComponentSurface {
            module: target.module,
            hydration_keys: keys.clone(),
            navigation_keys: keys,
        })
    }

    pub(super) fn component(
        &mut self,
        module: &str,
        site: analysis::Definition,
        components: &mut BTreeMap<String, ComponentSurface>,
    ) {
        let identity = self.resolve(Query::Value(Target {
            module: module.to_owned(),
            value: site.class,
            environment: None,
        }));
        let target = match identity {
            Identity::Class(target) => target,
            Identity::Mutable => {
                self.diagnostics.push(diagnostic(
                    module,
                    "PROJ-C009",
                    "Use an immutable class reference in define().",
                ));
                return;
            }
            _ => return,
        };
        let (is_webui, keys, errors) = self.lineage(&target);
        if !is_webui {
            return;
        }
        if !site.arguments_valid {
            self.diagnostics.push(diagnostic(module, "PROJ-C011", "Use Class.define(tag) or customElements.define(tag, Class) with exactly their required arguments."));
            return;
        }
        let Some(tag) = site.tag else {
            self.diagnostics.push(diagnostic(
                module,
                "PROJ-C008",
                "Define custom elements with a literal UTF-8 string tag.",
            ));
            return;
        };
        if target.module.starts_with('\0') {
            self.diagnostics.push(diagnostic(
                module,
                "PROJ-C006",
                "A shipped WebUI class must have a physical defining module.",
            ));
            return;
        }
        if components.contains_key(&tag) {
            self.diagnostics.push(diagnostic(
                module,
                "PROJ-C010",
                "Define each custom element tag only once.",
            ));
            return;
        }
        if !errors.is_empty() {
            self.diagnostics.extend(errors);
            return;
        }
        components.insert(
            tag,
            ComponentSurface {
                module: target.module,
                hydration_keys: keys.clone(),
                navigation_keys: keys,
            },
        );
    }

    fn lineage(&mut self, start: &Target) -> (bool, BTreeSet<String>, Vec<ProjectionDiagnostic>) {
        let source_only = matches!(self.evidence, Evidence::Source(_));
        let mut current = Some(start.clone());
        let mut seen = HashSet::new();
        let mut keys = BTreeSet::new();
        let mut is_webui = false;
        let mut errors = Vec::new();
        while let Some(target) = current.take() {
            if !seen.insert(target.clone()) {
                errors.push(diagnostic(
                    &target.module,
                    "PROJ-C012",
                    "Remove circular class inheritance.",
                ));
                break;
            }
            let Some(class) = self
                .analyses
                .get(&target.module)
                .and_then(|analysis| analysis.classes.get(&target.value))
            else {
                break;
            };
            if source_only && !class.source_identity_preserved {
                errors.push(diagnostic(
                    &target.module,
                    "PROJ-C007",
                    "Avoid class, accessor, parameter and static decorators, static blocks, and define overrides in the projected class chain.",
                ));
                break;
            }
            let base = class.base;
            let properties: Vec<_> = class
                .properties
                .iter()
                .map(|property| (property.name.clone(), property.decorators.clone()))
                .collect();
            if source_only {
                let methods = class.method_decorators.clone();
                for decorator in methods {
                    if !self
                        .decorator_use(&target, decorator)
                        .is_some_and(|used| used.is_neutral(true))
                    {
                        errors.push(unproven_decorator(&target.module));
                    }
                }
            }
            for (name, decorators) in properties {
                for decorator in decorators {
                    let Some(used) = self.decorator_use(&target, decorator) else {
                        errors.push(diagnostic(&target.module, "PROJ-C007", "Use a bare framework decorator, namespace member or attr options call."));
                        continue;
                    };
                    if source_only && name.is_some() && used.is_neutral(false) {
                        continue;
                    }
                    match used.identity {
                        Identity::Observable | Identity::Attr => {
                            is_webui |= !source_only;
                            if used.called && used.identity == Identity::Observable {
                                errors.push(diagnostic(
                                    &target.module,
                                    "PROJ-C007",
                                    "observable is not a decorator factory.",
                                ));
                                continue;
                            }
                            if let Some(name) = &name {
                                keys.insert(name.clone());
                            } else {
                                errors.push(diagnostic(
                                    &target.module,
                                    "PROJ-C007",
                                    "Reactive properties must use plain identifier names.",
                                ));
                            }
                        }
                        Identity::Other | Identity::Factory(_, _) if !source_only => {}
                        Identity::Invalid("PROJ-C012") => errors.push(diagnostic(
                            &target.module,
                            "PROJ-C012",
                            "Remove circular decorator imports or re-exports.",
                        )),
                        _ => errors.push(unproven_decorator(&target.module)),
                    }
                }
            }
            if let Some(base) = base {
                match self.resolve(Query::Value(Target {
                    value: base,
                    ..target.clone()
                })) {
                    Identity::Element => is_webui = true,
                    Identity::Class(next) => current = Some(next),
                    Identity::Invalid(code) => errors.push(diagnostic(
                        &target.module,
                        if matches!(code, "PROJ-C006" | "PROJ-C012") {
                            code
                        } else {
                            "PROJ-C005"
                        },
                        "Use a resolvable immutable base class or supported class factory.",
                    )),
                    _ => errors.push(diagnostic(
                        &target.module,
                        "PROJ-C005",
                        "Use a resolvable immutable base class or supported class factory.",
                    )),
                }
            }
        }
        (is_webui, keys, errors)
    }

    fn decorator_use(&mut self, target: &Target, decorator: NodeId) -> Option<DecoratorUse> {
        let analysis = self.analyses.get(&target.module)?;
        let (callee, called, literal_arguments) = match analysis.values.get(&decorator)? {
            Value::Call {
                callee,
                valid,
                literal_arguments,
                ..
            } if *valid || matches!(self.evidence, Evidence::Bundle(_)) => {
                (*callee, true, *literal_arguments)
            }
            Value::Symbol(_) | Value::Member(_, _) | Value::Global => (decorator, false, false),
            _ => return None,
        };
        let identity = self.resolve(Query::Value(Target {
            value: callee,
            ..target.clone()
        }));
        Some(DecoratorUse {
            identity,
            called,
            literal_arguments,
        })
    }
}

#[cold]
#[inline(never)]
fn unproven_decorator(module: &str) -> ProjectionDiagnostic {
    diagnostic(
        module,
        "PROJ-C004",
        "Resolve decorators to framework observable/attr or a statically proven state-neutral function.",
    )
}

#[cold]
#[inline(never)]
fn unproven_definition(module: &str) -> ProjectionDiagnostic {
    diagnostic(
        module,
        "PROJ-C014",
        "Define the WebUIElement class in its same-named module and register it once with a top-level literal Class.define('tag') or customElements.define('tag', Class).",
    )
}

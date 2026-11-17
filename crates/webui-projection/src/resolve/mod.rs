// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

mod components;
mod graph;

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

use oxc::syntax::{node::NodeId, symbol::SymbolId};

use crate::analysis::{self, Analysis, Binding, Export, Value};
use crate::{diagnostic, AnalysisReport, ComponentSurface, ModuleGraph, ProjectionDiagnostic};

const FRAMEWORK: &str = "\0framework:@microsoft/webui-framework";

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
struct Target {
    module: String,
    value: NodeId,
    environment: Option<usize>,
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
enum Query {
    Value(Target),
    Symbol(String, SymbolId, Option<usize>),
    Export(String, String),
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum Identity {
    Class(Target),
    Factory(String, SymbolId),
    Namespace(String),
    Framework,
    Element,
    Observable,
    Attr,
    Mutable,
    Other,
    Invalid(&'static str),
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
enum Continuation {
    Member(String),
    Call(Target, Vec<NodeId>),
}

struct Frame {
    parent: Option<usize>,
    bindings: HashMap<(String, SymbolId), Target>,
}

pub(super) struct Compiler<'a> {
    graph: &'a ModuleGraph,
    analyses: BTreeMap<String, Analysis>,
    environments: Vec<Frame>,
    diagnostics: Vec<ProjectionDiagnostic>,
    attempted: BTreeSet<String>,
}

impl<'a> Compiler<'a> {
    pub(super) fn new(graph: &'a ModuleGraph) -> Self {
        Self {
            graph,
            analyses: BTreeMap::new(),
            environments: Vec::new(),
            diagnostics: Vec::new(),
            attempted: BTreeSet::new(),
        }
    }

    fn prepare(&mut self, module: &str) -> bool {
        if self.analyses.contains_key(module) {
            return true;
        }
        let syntax = self
            .graph
            .modules
            .get(module)
            .and_then(|module| module.syntax);
        if syntax.is_none() && !analysis::supported(module) {
            return false;
        }
        if !self.attempted.insert(module.to_owned()) {
            return false;
        }
        let Some(source) = self
            .graph
            .modules
            .get(module)
            .filter(|module| !module.opaque)
            .and_then(|module| module.source.as_deref())
        else {
            return false;
        };

        match analysis::parse(module, source, syntax) {
            Ok(analysis) => {
                self.analyses.insert(module.to_owned(), analysis);
                true
            }
            Err(error) => {
                self.diagnostics.push(error);
                false
            }
        }
    }

    pub(super) fn analyze(mut self) -> AnalysisReport {
        self.diagnostics = graph::validate(self.graph);
        if !self.diagnostics.is_empty() {
            return AnalysisReport {
                components: BTreeMap::new(),
                diagnostics: self.diagnostics,
                parsed_modules: Vec::new(),
            };
        }
        let modules: Vec<_> = self
            .graph
            .modules
            .iter()
            .filter(|(id, module)| {
                !module.opaque
                    && (module.syntax.is_some() || analysis::supported(id))
                    && module
                        .source
                        .as_deref()
                        .is_some_and(|source| source.contains("define"))
            })
            .map(|(id, _)| id.clone())
            .collect();
        for module in modules {
            self.prepare(&module);
        }
        let sites: Vec<_> = self
            .analyses
            .iter()
            .flat_map(|(module, analysis)| {
                analysis
                    .definitions
                    .iter()
                    .cloned()
                    .map(move |site| (module.clone(), site))
            })
            .collect();
        let mut components = BTreeMap::new();
        for (module, site) in sites {
            self.component(&module, site, &mut components);
        }
        AnalysisReport {
            components,
            diagnostics: self.diagnostics,
            parsed_modules: self.attempted.into_iter().collect(),
        }
    }

    fn imported(
        &self,
        module: &str,
        specifier: &str,
        name: Option<&str>,
    ) -> Result<Query, Identity> {
        let edge = self
            .graph
            .modules
            .get(module)
            .and_then(|module| {
                module
                    .imports
                    .iter()
                    .find(|edge| !edge.dynamic && edge.specifier == specifier)
            })
            .ok_or(Identity::Invalid("PROJ-C002"))?;
        if graph::package_name(self.graph, edge) == Some("@microsoft/webui-framework") {
            return Ok(Query::Export(
                FRAMEWORK.to_owned(),
                name.unwrap_or("").to_owned(),
            ));
        }
        if edge.external {
            return Err(Identity::Invalid("PROJ-C006"));
        }
        Ok(name.map_or_else(
            || Query::Export(edge.resolved.clone(), String::new()),
            |name| Query::Export(edge.resolved.clone(), name.to_owned()),
        ))
    }

    fn substituted(
        &self,
        module: &str,
        symbol: SymbolId,
        mut environment: Option<usize>,
    ) -> Option<Target> {
        while let Some(index) = environment {
            let frame = &self.environments[index];
            if let Some(target) = frame.bindings.get(&(module.to_owned(), symbol)) {
                return Some(target.clone());
            }
            environment = frame.parent;
        }
        None
    }

    fn resolve(&mut self, query: Query) -> Identity {
        let mut pending = vec![(query, Vec::<Continuation>::new())];
        let mut seen = HashSet::new();
        let mut failure = Identity::Other;
        while let Some((query, mut continuations)) = pending.pop() {
            if seen.len() >= 65_535 || !seen.insert((query.clone(), continuations.clone())) {
                failure = Identity::Invalid("PROJ-C012");
                continue;
            }
            let step = self.step(query);
            let mut identity = match step {
                Step::Next(query) => {
                    pending.push((query, continuations));
                    continue;
                }
                Step::Continue(query, continuation) => {
                    continuations.push(continuation);
                    pending.push((query, continuations));
                    continue;
                }
                Step::Branches(queries) => {
                    for query in queries {
                        pending.push((query, continuations.clone()));
                    }
                    continue;
                }
                Step::Done(identity) => identity,
            };
            let mut deferred = false;
            while let Some(continuation) = continuations.pop() {
                match self.continue_with(identity, continuation) {
                    Step::Next(query) => {
                        pending.push((query, continuations));
                        deferred = true;
                        identity = Identity::Other;
                        break;
                    }
                    Step::Done(value) => identity = value,
                    _ => {
                        identity = Identity::Invalid("PROJ-C013");
                        break;
                    }
                }
            }
            if deferred {
                continue;
            }
            if matches!(identity, Identity::Invalid(_)) {
                failure = identity;
                continue;
            }
            return identity;
        }
        failure
    }

    fn step(&mut self, query: Query) -> Step {
        if let Query::Export(module, name) = &query {
            if module == FRAMEWORK {
                return Step::Done(if name.is_empty() {
                    Identity::Framework
                } else {
                    framework_name(name)
                });
            }
        }
        let module = match &query {
            Query::Value(target) => &target.module,
            Query::Symbol(module, ..) | Query::Export(module, _) => module,
        };
        if !self.prepare(module) {
            return Step::Done(Identity::Invalid("PROJ-C006"));
        }
        let analysis = &self.analyses[module];
        match query {
            Query::Value(target) => match analysis.values.get(&target.value) {
                Some(Value::Symbol(symbol)) => {
                    Step::Next(Query::Symbol(target.module, *symbol, target.environment))
                }
                Some(Value::Member(base, name)) => Step::Continue(
                    Query::Value(Target {
                        value: *base,
                        ..target
                    }),
                    Continuation::Member(name.clone()),
                ),
                Some(Value::Call {
                    callee,
                    arguments,
                    valid: true,
                }) => Step::Continue(
                    Query::Value(Target {
                        value: *callee,
                        ..target.clone()
                    }),
                    Continuation::Call(target, arguments.clone()),
                ),
                Some(Value::Class) => Step::Done(Identity::Class(target)),
                Some(Value::Alias(value)) => Step::Next(Query::Value(Target {
                    value: *value,
                    ..target
                })),
                Some(Value::Global) => Step::Done(Identity::Invalid("PROJ-C004")),
                _ => Step::Done(Identity::Other),
            },
            Query::Symbol(module, symbol, environment) => {
                if let Some(target) = self.substituted(&module, symbol, environment) {
                    return Step::Next(Query::Value(target));
                }
                match analysis.bindings.get(&symbol) {
                    Some(Binding::Value(value)) => Step::Next(Query::Value(Target {
                        module,
                        value: *value,
                        environment,
                    })),
                    Some(Binding::Import { specifier, name }) => self
                        .imported(&module, specifier, name.as_deref())
                        .map_or_else(Step::Done, Step::Next),
                    Some(Binding::Factory(_)) => Step::Done(Identity::Factory(module, symbol)),
                    Some(Binding::Mutable) => Step::Done(Identity::Mutable),
                    Some(Binding::Other) => Step::Done(Identity::Other),
                    None => Step::Done(Identity::Invalid("PROJ-C004")),
                }
            }
            Query::Export(module, name) if name.is_empty() => {
                Step::Done(Identity::Namespace(module))
            }
            Query::Export(module, name) => match analysis.exports.get(&name) {
                Some(Export::Symbol(symbol)) => Step::Next(Query::Symbol(module, *symbol, None)),
                Some(Export::Value(value)) => Step::Next(Query::Value(Target {
                    module,
                    value: *value,
                    environment: None,
                })),
                Some(Export::Forward { specifier, name }) => self
                    .imported(&module, specifier, name.as_deref())
                    .map_or_else(Step::Done, Step::Next),
                None => {
                    let queries: Vec<_> = analysis
                        .stars
                        .iter()
                        .filter_map(|specifier| self.imported(&module, specifier, Some(&name)).ok())
                        .collect();
                    if queries.is_empty() {
                        Step::Done(Identity::Invalid("PROJ-C003"))
                    } else {
                        Step::Branches(queries)
                    }
                }
            },
        }
    }

    fn continue_with(&mut self, identity: Identity, continuation: Continuation) -> Step {
        match continuation {
            Continuation::Member(name) => match identity {
                Identity::Namespace(module) => Step::Next(Query::Export(module, name)),
                Identity::Framework => Step::Done(framework_name(&name)),
                _ => Step::Done(Identity::Other),
            },
            Continuation::Call(site, arguments) => {
                let Identity::Factory(module, symbol) = identity else {
                    return Step::Done(Identity::Other);
                };
                let Some(analysis) = self.analyses.get(&module) else {
                    return Step::Done(Identity::Invalid("PROJ-C013"));
                };
                let Some(Binding::Factory(factory)) = analysis.bindings.get(&symbol) else {
                    return Step::Done(Identity::Other);
                };
                let Some(class) = analysis.classes.get(&factory.class) else {
                    return Step::Done(Identity::Other);
                };
                let valid_base = class.base.and_then(|id| analysis.values.get(&id))
                    .is_some_and(|base| matches!(base, Value::Symbol(symbol) if factory.parameters.contains(symbol)));
                if !class.valid_factory || !valid_base {
                    return Step::Done(Identity::Invalid("PROJ-C005"));
                }
                if self.environments.len() >= 65_535 {
                    return Step::Done(Identity::Invalid("PROJ-C012"));
                }
                let bindings = factory
                    .parameters
                    .iter()
                    .zip(arguments)
                    .map(|(parameter, value)| {
                        (
                            (module.clone(), *parameter),
                            Target {
                                value,
                                ..site.clone()
                            },
                        )
                    })
                    .collect();
                let value = factory.class;
                let environment = Some(self.environments.len());
                self.environments.push(Frame {
                    parent: site.environment,
                    bindings,
                });
                Step::Done(Identity::Class(Target {
                    module,
                    value,
                    environment,
                }))
            }
        }
    }
}

enum Step {
    Next(Query),
    Continue(Query, Continuation),
    Branches(Vec<Query>),
    Done(Identity),
}

fn framework_name(name: &str) -> Identity {
    match name {
        "WebUIElement" => Identity::Element,
        "observable" => Identity::Observable,
        "attr" => Identity::Attr,
        _ => Identity::Invalid("PROJ-C003"),
    }
}

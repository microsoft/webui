// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

//! Build-only state analysis over a completed bundle graph or the restricted
//! source contract used by the template compiler.

mod analysis;
mod resolve;
mod source;

pub use source::analyze_sources;

use std::collections::{BTreeMap, BTreeSet};

const MAX_MODULES: usize = 65_535;
const MAX_MODULE_BYTES: usize = 16 * 1024 * 1024;
const MAX_SOURCE_BYTES: usize = 64 * 1024 * 1024;

/// One actionable, color-free state-analysis error.
#[derive(Debug)]
pub struct ProjectionDiagnostic {
    /// Stable projection error code.
    pub code: &'static str,
    /// Original module that could not be proven.
    pub module: String,
    /// Actionable explanation.
    pub help: String,
}

/// Exact state analysis could not be proven for the supplied graph.
#[derive(Debug, thiserror::Error)]
#[error("native state projection failed: {0:?}")]
pub struct ProjectionError(pub Vec<ProjectionDiagnostic>);

#[cold]
#[inline(never)]
fn diagnostic(module: &str, code: &'static str, help: impl Into<String>) -> ProjectionDiagnostic {
    ProjectionDiagnostic {
        code,
        module: module.to_owned(),
        help: help.into(),
    }
}

/// Exact public state surface of one proven WebUI component.
#[derive(Debug)]
pub struct ComponentSurface {
    /// Physical module defining the class.
    pub module: String,
    /// Property names required for initial hydration.
    pub hydration_keys: BTreeSet<String>,
    /// Property names required for client navigation.
    pub navigation_keys: BTreeSet<String>,
}

/// Prove component declarations using only the supplied bundler graph.
///
/// Unsupported or ambiguous WebUI declarations return stable diagnostics,
/// never partial key lists or a full-state fallback.
pub fn analyze(graph: &ModuleGraph) -> Result<BTreeMap<String, ComponentSurface>, ProjectionError> {
    let report = analyze_with_report(graph);
    if report.diagnostics.is_empty() {
        Ok(report.components)
    } else {
        Err(ProjectionError(report.diagnostics))
    }
}

/// Analysis results and the exact modules parsed for build profiling.
pub struct AnalysisReport {
    /// Proven component declarations.
    pub components: BTreeMap<String, ComponentSurface>,
    /// Actionable failures; components must not be published when nonempty.
    pub diagnostics: Vec<ProjectionDiagnostic>,
    /// Modules whose original source was parsed, including invalid attempts.
    pub parsed_modules: Vec<String>,
}

/// Analyze a graph while reporting actual parser work without retaining ASTs.
pub fn analyze_with_report(graph: &ModuleGraph) -> AnalysisReport {
    resolve::Compiler::new(graph).analyze()
}

/// An authored specifier paired with the bundler's actual resolution.
#[derive(Debug)]
pub struct Import {
    /// Literal specifier in the original module.
    pub specifier: String,
    /// Canonical bundler module identity.
    pub resolved: String,
    /// Whether this edge is a dynamic import.
    pub dynamic: bool,
    /// Whether the bundler left this dependency external.
    pub external: bool,
    /// Owning package established by the adapter, including aliased imports.
    pub package_name: Option<String>,
}

/// Original module content and its resolved edges.
#[derive(Debug, Default)]
pub struct Module {
    /// Original text before TypeScript transformation.
    pub source: Option<String>,
    /// Edges reported by the bundler, not a second resolver.
    pub imports: Vec<Import>,
    /// Package identity established by the adapter.
    pub package_name: Option<String>,
    /// Non-JavaScript input (including explicit text/asset loader overrides).
    /// Such modules are graph evidence, never semantic JavaScript candidates.
    pub opaque: bool,
    /// Actual script loader when it differs from filename inference.
    pub syntax: Option<ScriptSyntax>,
}

/// JavaScript/TypeScript grammar selected by the native bundler.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ScriptSyntax {
    /// JavaScript.
    Js,
    /// JavaScript with JSX.
    Jsx,
    /// TypeScript.
    Ts,
    /// TypeScript with JSX.
    Tsx,
}

/// Evidence for one completed native bundle.
#[derive(Debug, Default)]
pub struct ModuleGraph {
    /// Canonical entry module identities.
    pub entries: BTreeSet<String>,
    /// Reachable physical and virtual modules keyed by bundler identity.
    pub modules: BTreeMap<String, Module>,
}

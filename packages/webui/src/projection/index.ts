// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

/**
 * `@microsoft/webui/projection.js` — build-only state projection compiler subpath.
 *
 * This subpath is intentionally separate from the root `@microsoft/webui`
 * entry so that render/build consumers do not load compiler or adapter code.
 *
 * Peer dependencies required by this subpath:
 *   - `typescript` ^6.0.3  (for TypeScript AST analysis)
 *   - `esbuild` ^0.28.1    (for the esbuild adapter only; other adapters have
 *                            their own optional peer requirements)
 *
 * Both peers are optional in `package.json` so users of the root build/render
 * API do not receive peer-missing warnings. Importing this subpath without the
 * required peer produces `PROJ-P001` or `PROJ-P002`.
 *
 * See DESIGN.md §"Bundler-Neutral State Projection Compiler" for the
 * authoritative specification.
 */

// Module graph types and adapter SPI
export type {
  ModuleKind,
  ResolvedImport,
  ModuleNode,
  ModuleGraph,
  OutputMembership,
  AdapterContext,
} from "./graph.js";

// Manifest schema types and validation
export type {
  ProjectionManifest,
  ProducerInfo,
  AdapterInfo,
  ComponentEntry,
} from "./manifest.js";
export { MANIFEST_SCHEMA, validateManifestSchema } from "./manifest.js";

// Diagnostic codes and error types
export type {
  ProjectionCode,
  DiagnosticSeverity,
  ProjectionDiagnostic,
} from "./diagnostics.js";
export {
  PROJECTION_CODES,
  CODE_SEVERITY,
  ProjectionError,
} from "./diagnostics.js";

// Conformance fixtures and test helpers
export type {
  ConformanceCase,
  ConformanceReport,
  ConformanceFailure,
  AdapterFactory,
  ConformanceSuiteOptions,
} from "./fixtures/conformance.js";
export { runConformanceSuite, ALL_CASES } from "./fixtures/conformance.js";

// Manifest serialization/hash/build-ID utilities
export {
  VIRTUAL_HASH,
  hashContent,
  computeBuildId,
  serializeManifestCanonical,
} from "./manifest.js";

/**
 * Compile an exact state-projection manifest from an adapter-resolved graph.
 *
 * The implementation is loaded lazily so importing the subpath without the
 * optional TypeScript peer produces a structured `PROJ-P001` diagnostic
 * instead of Node's generic module-resolution error.
 */
export { compileProjection } from "./loader.js";

// Official adapters
export type { EsbuildProjectionOptions } from "./adapters/esbuild.js";
export { esbuildProjection } from "./adapters/esbuild.js";

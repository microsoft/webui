// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

/**
 * Bundler-neutral state projection compiler and finalization API.
 *
 * This entry does not import any bundler adapter, so consumers only need the
 * application-owned TypeScript peer when they invoke compilation.
 */

export type {
  ModuleKind,
  ResolvedImport,
  ModuleNode,
  ModuleGraph,
  OutputMembership,
  AdapterContext,
} from "./graph.js";

export type {
  ProjectionManifest,
  ProducerInfo,
  AdapterInfo,
  ComponentEntry,
} from "./manifest.js";
export {
  MANIFEST_SCHEMA,
  VIRTUAL_HASH,
  hashContent,
  computeBuildId,
  serializeManifestCanonical,
  validateManifestSchema,
} from "./manifest.js";

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

export { compileProjection } from "./loader.js";
export { ProjectionSession } from "./session.js";

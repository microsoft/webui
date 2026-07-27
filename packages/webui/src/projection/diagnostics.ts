// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

/**
 * Stable diagnostic codes for the WebUI state projection compiler.
 *
 * All codes are machine-readable and stable across versions. They appear in
 * the `code` field of a `Diagnostic` object alongside `title`, `location`,
 * `snippet`, and `help`. No color in diagnostic data; color is added only
 * by the `webui-cli` output layer.
 *
 * See DESIGN.md §"Projection diagnostic codes" for the authoritative list
 * and conditions.
 */

/** All stable projection diagnostic codes grouped by category. */
export const PROJECTION_CODES = {
  // Compiler diagnostics
  /** TypeScript parse error in source file. */
  C001: "PROJ-C001",
  /** Import specifier does not resolve to any module in the graph. */
  C002: "PROJ-C002",
  /** Named import not found in the resolved module's exports. */
  C003: "PROJ-C003",
  /**
   * Decorator cannot be resolved to `observable` or `attr` from
   * `@microsoft/webui-framework`; cannot prove exact keys.
   */
  C004: "PROJ-C004",
  /**
   * Base class cannot be resolved to a class declaration in the graph;
   * cannot prove exact inherited keys.
   */
  C005: "PROJ-C005",
  /**
   * Class uses `@observable`/`@attr` decorators but its module source is
   * unavailable (external/virtual module).
   */
  C006: "PROJ-C006",
  /**
   * Unsupported decorator form (computed property key, call-chain decorator,
   * reflection-based).
   */
  C007: "PROJ-C007",
  /** `define()` tag argument is not a string literal. */
  C008: "PROJ-C008",
  /** `define()` class argument cannot be resolved to a class in the graph. */
  C009: "PROJ-C009",
  /** Duplicate `define()` for the same tag within one adapter context. */
  C010: "PROJ-C010",
  /** `Class.define(...)` called with wrong argument count. */
  C011: "PROJ-C011",
  /** Circular import detected during symbol resolution. */
  C012: "PROJ-C012",
  /** Adapter graph is internally inconsistent or omits a resolved edge. */
  C013: "PROJ-C013",
  /** Adapter omitted exact bytes for a physical emitted output. */
  C014: "PROJ-C014",

  // Peer dependency diagnostics
  /**
   * Required peer `typescript` is absent or below the supported range.
   */
  P001: "PROJ-P001",
  /**
   * Required peer `esbuild` is absent or below the supported range
   * (esbuild adapter only).
   */
  P002: "PROJ-P002",
  /** Peer is present but above the tested range; results may differ. */
  P003: "PROJ-P003",

  // Manifest diagnostics
  /** Manifest file is missing or unreadable. */
  M001: "PROJ-M001",
  /** Manifest schema version is not `webui.state-projection/v1`. */
  M002: "PROJ-M002",
  /** Declared input file hash does not match current file content (stale). */
  M003: "PROJ-M003",
  /** Declared output file hash does not match current file content (stale). */
  M004: "PROJ-M004",
  /** Manifest `buildId` does not match recomputed build ID. */
  M005: "PROJ-M005",
  /** Same component tag owned by two or more manifests (duplicate ownership). */
  M006: "PROJ-M006",
  /**
   * Same path key in merged `inputs` or `outputs` has conflicting hashes
   * across two manifests.
   */
  M007: "PROJ-M007",
  /** Manifest JSON is syntactically invalid. */
  M008: "PROJ-M008",
  /** Required manifest field is missing or has wrong type. */
  M009: "PROJ-M009",

  // Build validation diagnostics
  /**
   * Compiled scripted component has no manifest entry (missing coverage).
   * Every component compiled into the protocol must have an entry.
   */
  B001: "PROJ-B001",
  /** `--projection-manifest` supplied with a non-WebUI plugin. */
  B002: "PROJ-B002",

  // Security and resource diagnostics
  /** Manifest file exceeds the 16 MiB size limit. */
  S001: "PROJ-S001",
  /** Manifest `components` count exceeds 65,535. */
  S002: "PROJ-S002",
  /**
   * Normalized path traverses outside the project root
   * (path traversal attempt).
   */
  S003: "PROJ-S003",
  /**
   * Hash format is invalid (not `"sha256:<64 lowercase hex>"` or
   * `"virtual"`).
   */
  S004: "PROJ-S004",
} as const;

/** Union of all projection diagnostic code strings. */
export type ProjectionCode = (typeof PROJECTION_CODES)[keyof typeof PROJECTION_CODES];

/** Severity level for a projection diagnostic. */
export type DiagnosticSeverity = "error" | "warning";

/** A single projection diagnostic with stable code and actionable help. */
export interface ProjectionDiagnostic {
  /** Stable machine-readable code, e.g. `"PROJ-C004"`. */
  readonly code: ProjectionCode;
  /** Human-readable title. */
  readonly title: string;
  /** Severity. */
  readonly severity: DiagnosticSeverity;
  /**
   * Source location as `"owner:line:column"` when the exact byte is known,
   * or `"in module <id>"` for module-level errors.
   * Optional; absent for diagnostics that have no single source location.
   */
  readonly location?: string;
  /** The offending source snippet (a few lines of context). Optional. */
  readonly snippet?: string;
  /**
   * Actionable fix suggestion. May include "did you mean …?" for typos.
   * Should always be present for errors.
   */
  readonly help?: string;
}

/** Thrown when the projection compiler encounters one or more hard errors. */
export class ProjectionError extends Error {
  readonly diagnostics: ReadonlyArray<ProjectionDiagnostic>;

  constructor(diagnostics: ReadonlyArray<ProjectionDiagnostic>) {
    const first = diagnostics[0];
    super(first ? `${first.code}: ${first.title}` : "Projection failed");
    this.name = "ProjectionError";
    this.diagnostics = diagnostics;
  }
}

/** Severity table for every code (used by the CLI output layer). */
export const CODE_SEVERITY: Readonly<Record<ProjectionCode, DiagnosticSeverity>> = {
  "PROJ-C001": "error",
  "PROJ-C002": "error",
  "PROJ-C003": "error",
  "PROJ-C004": "error",
  "PROJ-C005": "error",
  "PROJ-C006": "error",
  "PROJ-C007": "error",
  "PROJ-C008": "error",
  "PROJ-C009": "error",
  "PROJ-C010": "error",
  "PROJ-C011": "error",
  "PROJ-C012": "error",
  "PROJ-C013": "error",
  "PROJ-C014": "error",
  "PROJ-P001": "error",
  "PROJ-P002": "error",
  "PROJ-P003": "warning",
  "PROJ-M001": "error",
  "PROJ-M002": "error",
  "PROJ-M003": "error",
  "PROJ-M004": "error",
  "PROJ-M005": "error",
  "PROJ-M006": "error",
  "PROJ-M007": "error",
  "PROJ-M008": "error",
  "PROJ-M009": "error",
  "PROJ-B001": "error",
  "PROJ-B002": "error",
  "PROJ-S001": "error",
  "PROJ-S002": "error",
  "PROJ-S003": "error",
  "PROJ-S004": "error",
};

/** Stable human-readable titles for every code, used by `createDiagnostic`. */
export const CODE_TITLES: Readonly<Record<ProjectionCode, string>> = {
  "PROJ-C001": "TypeScript parse error in source file",
  "PROJ-C002": "Import specifier does not resolve to any module in the graph",
  "PROJ-C003": "Named import not found in the resolved module's exports",
  "PROJ-C004":
    "Decorator cannot be resolved to observable or attr from @microsoft/webui-framework",
  "PROJ-C005": "Base class cannot be resolved to a class declaration in the graph",
  "PROJ-C006": "Class uses decorators but its module source is unavailable",
  "PROJ-C007": "Unsupported decorator form",
  "PROJ-C008": "define() tag argument is not a string literal",
  "PROJ-C009": "define() class argument cannot be resolved to a class in the graph",
  "PROJ-C010": "Duplicate define() for the same tag",
  "PROJ-C011": "define() called with the wrong argument count",
  "PROJ-C012": "Circular import detected during symbol resolution",
  "PROJ-C013": "Adapter module graph is incomplete or inconsistent",
  "PROJ-C014": "Adapter omitted bytes for a physical emitted output",
  "PROJ-P001": "Required peer 'typescript' is absent or below the supported range",
  "PROJ-P002": "Required peer 'esbuild' is absent or below the supported range",
  "PROJ-P003": "Peer is present but above the tested range",
  "PROJ-M001": "Manifest file is missing or unreadable",
  "PROJ-M002": "Manifest schema version is unsupported",
  "PROJ-M003": "Declared input file hash does not match current file content",
  "PROJ-M004": "Declared output file hash does not match current file content",
  "PROJ-M005": "Manifest buildId does not match recomputed build ID",
  "PROJ-M006": "Same component tag owned by two or more manifests",
  "PROJ-M007": "Same path key in merged inputs/outputs has conflicting hashes",
  "PROJ-M008": "Manifest JSON is syntactically invalid",
  "PROJ-M009": "Required manifest field is missing or has the wrong type",
  "PROJ-B001": "Compiled scripted component has no manifest entry",
  "PROJ-B002": "--projection-manifest supplied with a non-WebUI plugin",
  "PROJ-S001": "Manifest file exceeds the 16 MiB size limit",
  "PROJ-S002": "Manifest components count exceeds 65,535",
  "PROJ-S003": "Normalized path traverses outside the project root",
  "PROJ-S004": "Hash format is invalid",
};

/**
 * Builds a `ProjectionDiagnostic` for a stable code, filling in the title and
 * severity from the stable tables and allowing callers to attach a
 * location/snippet/help specific to the failure site.
 */
export function createDiagnostic(
  code: ProjectionCode,
  overrides: { location?: string; snippet?: string; help?: string } = {}
): ProjectionDiagnostic {
  return {
    code,
    title: CODE_TITLES[code],
    severity: CODE_SEVERITY[code],
    ...(overrides.location !== undefined ? { location: overrides.location } : {}),
    ...(overrides.snippet !== undefined ? { snippet: overrides.snippet } : {}),
    ...(overrides.help !== undefined ? { help: overrides.help } : {}),
  };
}

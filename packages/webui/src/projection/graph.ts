// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

/**
 * Normalized module graph and adapter SPI for the bundler-neutral
 * state projection compiler.
 *
 * The adapter SPI isolates bundler-specific semantics. The projection compiler
 * (`compiler.ts`) consumes only these interfaces, never bundler-specific APIs.
 *
 * See DESIGN.md §"Bundler-Neutral State Projection Compiler" for the
 * authoritative specification.
 */

/** Physical/virtual classification used for hashing and path validation. */
export type ModuleKind = "file" | "virtual";

/** One authored module specifier paired with the bundler-resolved target. */
export interface ResolvedImport {
  /** Specifier exactly as authored in the importing source file. */
  readonly specifier: string;

  /**
   * Canonical target module ID. Undefined only when `external` is true.
   *
   * The compiler never reconstructs this value from `specifier`.
   */
  readonly resolvedId: string | undefined;

  /** Whether the bundler left this dependency external. */
  readonly external: boolean;

  /** Static imports/re-exports participate in symbol resolution. */
  readonly kind: "static" | "dynamic";

  /**
   * Owning package when the adapter can prove it.
   *
   * This lets aliases that resolve to `@microsoft/webui-framework` retain the
   * same semantic identity without coupling the compiler to node_modules path
   * layouts.
   */
  readonly packageName?: string;
}

/** A single resolved module in the build graph. */
export interface ModuleNode {
  /**
   * Canonical absolute path on disk (always forward-slash, no query/hash).
   *
   * Virtual modules (no physical file) have an `id` beginning with `\0`
   * (null byte). Their `source` may be provided but their physical `inputs`
   * hash is `"virtual"`.
   */
  readonly id: string;

  /** Whether `id` identifies a physical file or a bundler-owned virtual module. */
  readonly kind: ModuleKind;

  /** Owning package identity, when proven by the adapter. */
  readonly packageName?: string;

  /**
   * Raw UTF-8 source text.
   *
   * Required for file modules. Virtual modules may omit source when they carry
   * no analyzable WebUI code.
   */
  readonly source: string | undefined;

  /**
   * Authored specifiers paired with their exact bundler-resolved targets.
   *
   * Keeping both values is essential: aliases, extension substitution,
   * package exports, virtual namespaces, and plugin resolution cannot be
   * reconstructed from source text.
   */
  readonly imports: ReadonlyArray<ResolvedImport>;
}

/** The resolved input module graph as seen by the bundler. */
export interface ModuleGraph {
  /**
   * All modules reachable from the entry set, keyed by canonical ID.
   *
   * The compiler walks this map following `imports` edges. Modules absent from
   * this map are not part of this build.
   */
  readonly modules: ReadonlyMap<string, ModuleNode>;

  /** Canonical entry module IDs; a subset of `modules.keys()`. */
  readonly entries: ReadonlyArray<string>;
}

/**
 * Maps each emitted output path to the input module IDs that contribute to it.
 *
 * This is the authoritative source for whether a component class was
 * tree-shaken from the final bundle. A class whose defining module does not
 * appear in any output's contributing set is excluded from the manifest.
 */
export interface OutputMembership {
  /**
   * Key: canonical absolute output ID, or a virtual ID beginning with `\0`.
   *
   * Value: set of canonical input module IDs (`ModuleNode.id`) whose code
   *   appears in this output after bundling and tree shaking.
   */
  readonly outputs: ReadonlyMap<string, ReadonlySet<string>>;
}

/**
 * Context passed from the bundler adapter to the projection compiler.
 *
 * The adapter constructs this from bundler-native structures and passes it
 * to `compileProjection()`. The compiler never calls bundler APIs directly.
 */
export interface AdapterContext {
  /** Resolved input module graph. */
  readonly graph: ModuleGraph;

  /** Final output→input membership from the bundler's tree shaking. */
  readonly membership: OutputMembership;

  /**
   * Canonical absolute build root containing every physical input, output,
   * and the manifest itself.
   *
   * Manifest file keys are relative to this root. The serialized manifest
   * stores the root relative to its own directory (normally `".."` for a
   * manifest under `dist/`).
   */
  readonly rootDir: string;

  /**
   * Absolute disk path where the manifest will be written.
   *
   * Used by the compiler to compute canonical relative paths for all
   * `outputs`, `inputs`, and `ComponentEntry.module` fields.
   */
  readonly manifestPath: string;

  /**
   * Bundler adapter name, e.g. `"esbuild"`.
   *
   * Written to `manifest.adapter.name`.
   */
  readonly bundlerName: string;

  /**
   * Bundler version string, e.g. `"0.28.1"`.
   *
   * Written to `manifest.adapter.bundler` as `"<bundlerName>@<bundlerVersion>"`.
   */
  readonly bundlerVersion: string;

  /**
   * Exact bytes for emitted physical output files, keyed by the same absolute
   * output ID used in `membership.outputs`.
   *
   * The compiler never reads files from disk. When an adapter can supply the
   * Adapters must provide every physical output. Omitting one is a hard
   * adapter-contract diagnostic; a disk output can never be marked virtual to
   * bypass stale validation.
   */
  readonly outputContents: ReadonlyMap<string, string | Uint8Array>;
}

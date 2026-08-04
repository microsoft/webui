// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

/**
 * Official esbuild adapter for bundler-neutral state projection.
 *
 * This module uses esbuild types only. It never imports the esbuild runtime;
 * the application-owned esbuild instance supplies the plugin API and version.
 */

import {
  readFile,
} from "node:fs/promises";
import * as path from "node:path";
import type {
  BuildResult,
  Metafile,
  PartialMessage,
  Plugin,
  PluginBuild,
} from "esbuild";
import type {
  AdapterContext,
  ModuleKind,
  ModuleNode,
  ResolvedImport,
} from "../graph.js";
import {
  ProjectionError,
  createDiagnostic,
} from "../diagnostics.js";
import type { ProjectionDiagnostic } from "../diagnostics.js";
import { compareUtf8 } from "../manifest.js";
import { ProjectionSession } from "../session.js";
import {
  adapterError,
  commonAncestor,
  comparePaths,
  nearestPackageName,
  packageNameFromSpecifier,
  parseVersion,
  pathKey,
  readPhysicalSource,
  samePath,
} from "./shared.js";

/** Configuration for the official esbuild projection adapter. */
export interface EsbuildProjectionOptions {
  /**
   * Manifest path. Relative values resolve from esbuild's `absWorkingDir`.
   *
   * Defaults to `<outdir>/webui-projection.json` or the directory containing
   * `outfile`.
   */
  readonly manifest?: string;
}

interface InputRecord {
  readonly metafileId: string;
  readonly moduleId: string;
  readonly kind: ModuleKind;
  readonly source: string | undefined;
  readonly packageName: string | undefined;
}

/** Create the official esbuild projection plugin. */
export function esbuildProjection(
  options: EsbuildProjectionOptions = {}
): Plugin {
  return {
    name: "webui-state-projection",
    setup(build) {
      build.initialOptions.metafile = true;
      const versionError = validateEsbuildVersion(build.esbuild.version);
      const session = versionError
        ? undefined
        : new ProjectionSession();
      if (versionError) {
        build.onStart(() => ({
          errors: [diagnosticMessage(versionError)],
        }));
      }

      build.onEnd(async (result) => {
        if (result.errors.length > 0 || versionError || !session) return;
        try {
          await emitProjectionManifest(build, result, options, session);
        } catch (error: unknown) {
          return {
            errors: errorMessages(error),
          };
        }
      });
    },
  };
}

async function emitProjectionManifest(
  build: PluginBuild,
  result: BuildResult,
  options: EsbuildProjectionOptions,
  session: ProjectionSession
): Promise<void> {
  const profile = process.env["WEBUI_PROJECTION_PROFILE"] === "1";
  const profileStart = profile ? performance.now() : 0;
  const metafile = result.metafile;
  if (!metafile) {
    throw adapterError(
      "esbuild did not return a metafile",
      "Do not disable metafile after esbuildProjection() configures the build."
    );
  }

  const workingDirectory = path.resolve(
    build.initialOptions.absWorkingDir ?? process.cwd()
  );
  const manifestPath = resolveManifestPath(
    workingDirectory,
    build,
    options
  );
  const outputIds = outputPaths(workingDirectory, metafile);
  if (
    [...outputIds.values()].some((outputId) =>
      samePath(outputId, manifestPath)
    )
  ) {
    throw adapterError(
      "the projection manifest path collides with an esbuild output",
      "Choose a distinct manifest filename such as dist/webui-projection.json."
    );
  }

  const packageCache = new Map<string, string | undefined>();
  const records = await loadInputRecords(
    workingDirectory,
    build,
    metafile,
    packageCache
  );
  const graphReady = profile ? performance.now() : 0;
  const recordByMetafileId = new Map(
    records.map((record) => [record.metafileId, record])
  );
  const graph = buildModuleGraph(metafile, records, recordByMetafileId);
  const membership = buildMembership(
    metafile,
    outputIds,
    recordByMetafileId
  );
  const outputContents = await loadOutputContents(
    result,
    outputIds
  );
  const rootDir = commonAncestor([
    manifestPath,
    ...records
      .filter((record) => record.kind === "file")
      .map((record) => record.moduleId),
    ...outputIds.values(),
  ]);

  const context: AdapterContext = {
    graph,
    membership,
    outputContents,
    entryClosures: buildEntryClosures(
      metafile,
      outputIds,
      build.initialOptions.publicPath
    ),
    rootDir,
    manifestPath,
    bundlerName: "esbuild",
    bundlerVersion: build.esbuild.version,
  };
  await session.finalize(context);
  if (profile) {
    const finished = performance.now();
    console.error(
      `[webui-projection] graph=${(graphReady - profileStart).toFixed(1)}ms finalize=${(finished - graphReady).toFixed(1)}ms total=${(finished - profileStart).toFixed(1)}ms`
    );
  }
}

function resolveManifestPath(
  workingDirectory: string,
  build: PluginBuild,
  options: EsbuildProjectionOptions
): string {
  if (options.manifest) {
    return path.resolve(workingDirectory, options.manifest);
  }
  const outdir = build.initialOptions.outdir;
  if (outdir) {
    return path.resolve(
      workingDirectory,
      outdir,
      "webui-projection.json"
    );
  }
  const outfile = build.initialOptions.outfile;
  if (outfile) {
    return path.join(
      path.dirname(path.resolve(workingDirectory, outfile)),
      "webui-projection.json"
    );
  }
  throw adapterError(
    "esbuildProjection() requires outdir, outfile, or an explicit manifest path",
    "Configure an emitted file location so projection output hashes can be validated."
  );
}

function outputPaths(
  workingDirectory: string,
  metafile: Metafile
): Map<string, string> {
  const result = new Map<string, string>();
  for (const outputPath of Object.keys(metafile.outputs)) {
    result.set(
      outputPath,
      path.resolve(workingDirectory, outputPath)
    );
  }
  return result;
}

async function loadInputRecords(
  workingDirectory: string,
  build: PluginBuild,
  metafile: Metafile,
  packageCache: Map<string, string | undefined>
): Promise<InputRecord[]> {
  const entries = Object.keys(metafile.inputs);
  const records = await Promise.all(
    entries.map(async (metafileId) => {
      const stdinSource = sourceForStdin(
        metafileId,
        build
      );
      if (stdinSource !== undefined) {
        return {
          metafileId,
          moduleId: virtualModuleId(metafileId),
          kind: "virtual" as const,
          source: stdinSource,
          packageName: undefined,
        };
      }

      const filePath = path.resolve(workingDirectory, metafileId);
      const source = await readPhysicalSource(filePath);
      if (source !== undefined) {
        return {
          metafileId,
          moduleId: filePath,
          kind: "file" as const,
          source,
          packageName: undefined,
        };
      }

      return {
        metafileId,
        moduleId: virtualModuleId(metafileId),
        kind: "virtual" as const,
        source: undefined,
        packageName: undefined,
      };
    })
  );
  const resolved: InputRecord[] = [];
  for (const record of records) {
    resolved.push(
      record.kind === "file"
        ? {
            ...record,
            packageName: await nearestPackageName(
              record.moduleId,
              packageCache
            ),
          }
        : record
    );
  }
  return resolved;
}

function sourceForStdin(
  metafileId: string,
  build: PluginBuild
): string | undefined {
  const stdin = build.initialOptions.stdin;
  if (!stdin) return undefined;
  const sourcefile = stdin.sourcefile ?? "<stdin>";
  if (metafileId !== sourcefile && metafileId !== "<stdin>") {
    return undefined;
  }
  return typeof stdin.contents === "string"
    ? stdin.contents
    : Buffer.from(stdin.contents).toString("utf8");
}

function buildModuleGraph(
  metafile: Metafile,
  records: ReadonlyArray<InputRecord>,
  recordByMetafileId: ReadonlyMap<string, InputRecord>
): AdapterContext["graph"] {
  const modules = new Map<string, ModuleNode>();
  for (const record of records) {
    const input = metafile.inputs[record.metafileId]!;
    const imports = input.imports.map((entry) =>
      resolvedImport(entry, recordByMetafileId)
    );
    modules.set(record.moduleId, {
      id: record.moduleId,
      kind: record.kind,
      ...packageNameProperty(record.packageName),
      source: record.source,
      imports,
    });
  }

  const entries = new Set<string>();
  for (const output of Object.values(metafile.outputs)) {
    if (!output.entryPoint) continue;
    const record = recordByMetafileId.get(output.entryPoint);
    if (record) entries.add(record.moduleId);
  }
  return {
    modules,
    entries: [...entries].sort(comparePaths),
  };
}

function resolvedImport(
  entry: Metafile["inputs"][string]["imports"][number],
  recordByMetafileId: ReadonlyMap<string, InputRecord>
): ResolvedImport {
  const target = entry.external
    ? undefined
    : recordByMetafileId.get(entry.path);
  return {
    specifier: entry.original ?? entry.path,
    resolvedId: target?.moduleId,
    external: entry.external === true,
    kind:
      entry.kind === "dynamic-import" ? "dynamic" : "static",
    ...packageNameProperty(
      target?.packageName ??
        (entry.external ? packageNameFromSpecifier(entry.path) : undefined)
    ),
  };
}

function packageNameProperty(
  packageName: string | undefined
): { readonly packageName?: string } {
  return packageName === undefined ? {} : { packageName };
}

function buildMembership(
  metafile: Metafile,
  outputIds: ReadonlyMap<string, string>,
  recordByMetafileId: ReadonlyMap<string, InputRecord>
): AdapterContext["membership"] {
  const outputs = new Map<string, ReadonlySet<string>>();
  for (const [outputPath, metadata] of Object.entries(
    metafile.outputs
  )) {
    const outputId = outputIds.get(outputPath);
    if (!outputId) continue;
    const members = new Set<string>();
    for (const inputPath of Object.keys(metadata.inputs)) {
      const record = recordByMetafileId.get(inputPath);
      if (record) members.add(record.moduleId);
    }
    outputs.set(outputId, members);
  }
  return { outputs };
}

/**
 * Computes each entry output's transitive static import closure, largest-first.
 *
 * A browser that fetches an entry module must also fetch every chunk the entry
 * reaches through static `import` statements before it can execute, and those
 * chunks are invisible to the preload scanner because they are named only
 * inside the entry's own bytes. Recording the closure here lets the handler
 * emit `modulepreload` hints without a second bundler pass.
 *
 * Ordering is the point, not a detail: preloads are issued in document order
 * over a shared connection, so a small chunk listed ahead of a large one
 * delays the long pole. Only esbuild knows the byte counts, so it sorts.
 */
function buildEntryClosures(
  metafile: Metafile,
  outputIds: ReadonlyMap<string, string>,
  publicPath: string | undefined
): ReadonlyMap<string, ReadonlyArray<string>> {
  const closures = new Map<string, ReadonlyArray<string>>();
  for (const [outputPath, metadata] of Object.entries(metafile.outputs)) {
    if (metadata.entryPoint === undefined) continue;
    const entryId = outputIds.get(outputPath);
    if (!entryId) continue;
    if (publicPath) {
      // A public path changes the URL written into emitted import specifiers,
      // but the metafile still exposes local output paths. Until the manifest
      // carries served URLs, retaining an empty owner is safer than synthesizing
      // same-origin hrefs for potentially cross-origin chunks.
      closures.set(entryId, []);
      continue;
    }

    // Iterative worklist: an output import graph may contain cycles, and the
    // repo bans recursion in graph walks.
    const reached = new Set<string>([outputPath]);
    const pending = [outputPath];
    const members: string[] = [];
    while (pending.length > 0) {
      const current = pending.pop()!;
      const imports = metafile.outputs[current]?.imports;
      if (!imports) continue;
      for (const edge of imports) {
        if (edge.kind !== "import-statement" || edge.external === true) {
          continue;
        }
        if (reached.has(edge.path)) continue;
        reached.add(edge.path);
        pending.push(edge.path);
        members.push(edge.path);
      }
    }
    members.sort((left, right) => {
      const bySize =
        (metafile.outputs[right]?.bytes ?? 0) -
        (metafile.outputs[left]?.bytes ?? 0);
      return bySize !== 0 ? bySize : compareUtf8(left, right);
    });

    const resolved: string[] = [];
    for (const member of members) {
      const memberId = outputIds.get(member);
      if (memberId) resolved.push(memberId);
    }
    closures.set(entryId, resolved);
  }
  return closures;
}

async function loadOutputContents(
  result: BuildResult,
  outputIds: ReadonlyMap<string, string>
): Promise<ReadonlyMap<string, Uint8Array>> {
  const inMemory = new Map<string, Uint8Array>();
  for (const outputFile of result.outputFiles ?? []) {
    inMemory.set(pathKey(outputFile.path), outputFile.contents);
  }

  const contents = new Map<string, Uint8Array>();
  for (const outputId of outputIds.values()) {
    const memory = inMemory.get(pathKey(outputId));
    contents.set(
      outputId,
      memory ?? (await readFile(outputId))
    );
  }
  return contents;
}

function virtualModuleId(metafileId: string): string {
  return `\0esbuild:${metafileId}`;
}

function validateEsbuildVersion(
  version: string
): ProjectionDiagnostic | undefined {
  const parsed = parseVersion(version);
  if (
    parsed === undefined ||
    parsed.major !== 0 ||
    parsed.minor !== 28 ||
    parsed.patch < 1
  ) {
    return createDiagnostic("PROJ-P002", {
      help: `Install an application-owned esbuild peer compatible with ^0.28.1; found ${version}.`,
    });
  }
  return undefined;
}

function errorMessages(error: unknown): PartialMessage[] {
  if (error instanceof ProjectionError) {
    return error.diagnostics.map(diagnosticMessage);
  }
  return [
    diagnosticMessage(
      createDiagnostic("PROJ-C013", {
        help:
          error instanceof Error
            ? error.message
            : String(error),
      })
    ),
  ];
}

function diagnosticMessage(
  diagnostic: ProjectionDiagnostic
): PartialMessage {
  return {
    id: diagnostic.code,
    pluginName: "webui-state-projection",
    text: `${diagnostic.code}: ${diagnostic.title}`,
    ...(diagnostic.location === undefined
      ? {}
      : {
          location: {
            file: diagnostic.location,
          },
        }),
    ...(diagnostic.help === undefined
      ? {}
      : {
          notes: [{ text: `help: ${diagnostic.help}` }],
        }),
    detail: diagnostic,
  };
}

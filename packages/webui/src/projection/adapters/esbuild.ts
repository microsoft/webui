// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

/**
 * Official esbuild adapter for bundler-neutral state projection.
 *
 * This module uses esbuild types only. It never imports the esbuild runtime;
 * the application-owned esbuild instance supplies the plugin API and version.
 */

import {
  mkdir,
  open,
  readFile,
  rename,
  rm,
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
import {
  serializeManifestCanonical,
} from "../manifest.js";
import {
  compileProjection,
  preloadProjectionCompiler,
} from "../loader.js";

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

let temporaryFileSequence = 0;

/** Create the official esbuild projection plugin. */
export function esbuildProjection(
  options: EsbuildProjectionOptions = {}
): Plugin {
  return {
    name: "webui-state-projection",
    setup(build) {
      build.initialOptions.metafile = true;
      const versionError = validateEsbuildVersion(build.esbuild.version);
      const compilerLoad = versionError
        ? Promise.resolve(undefined)
        : preloadProjectionCompiler().then(
            () => undefined,
            (error: unknown) => error
          );
      if (versionError) {
        build.onStart(() => ({
          errors: [diagnosticMessage(versionError)],
        }));
      }

      build.onEnd(async (result) => {
        if (result.errors.length > 0 || versionError) return;
        try {
          const compilerError = await compilerLoad;
          if (compilerError !== undefined) throw compilerError;
          await emitProjectionManifest(build, result, options);
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
  options: EsbuildProjectionOptions
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
    rootDir,
    manifestPath,
    bundlerName: "esbuild",
    bundlerVersion: build.esbuild.version,
  };
  const manifest = await compileProjection(context);
  const compiled = profile ? performance.now() : 0;
  await writeAtomic(
    manifestPath,
    serializeManifestCanonical(manifest)
  );
  if (profile) {
    const finished = performance.now();
    console.error(
      `[webui-projection] graph=${(graphReady - profileStart).toFixed(1)}ms compile=${(compiled - graphReady).toFixed(1)}ms write=${(finished - compiled).toFixed(1)}ms total=${(finished - profileStart).toFixed(1)}ms`
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

      const stdinSource = sourceForStdin(
        metafileId,
        build
      );
      return {
        metafileId,
        moduleId: virtualModuleId(metafileId),
        kind: "virtual" as const,
        source: stdinSource,
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

async function readPhysicalSource(
  filePath: string
): Promise<string | undefined> {
  try {
    const bytes = await readFile(filePath);
    return bytes.toString("utf8");
  } catch (error: unknown) {
    if (isMissingFile(error)) return undefined;
    throw error;
  }
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

async function nearestPackageName(
  filePath: string,
  cache: Map<string, string | undefined>
): Promise<string | undefined> {
  let directory = path.dirname(filePath);
  const visited: string[] = [];
  while (true) {
    if (cache.has(directory)) {
      const cached = cache.get(directory);
      for (const value of visited) cache.set(value, cached);
      return cached;
    }
    visited.push(directory);
    const packagePath = path.join(directory, "package.json");
    try {
      const text = await readFile(packagePath, "utf8");
      const parsed = JSON.parse(text) as { name?: unknown };
      const name =
        typeof parsed.name === "string" && parsed.name.length > 0
          ? parsed.name
          : undefined;
      for (const value of visited) cache.set(value, name);
      return name;
    } catch (error: unknown) {
      if (!isMissingFile(error)) {
        throw adapterError(
          `could not read package identity from '${packagePath}'`,
          error instanceof Error ? error.message : String(error)
        );
      }
    }
    const parent = path.dirname(directory);
    if (parent === directory) {
      for (const value of visited) cache.set(value, undefined);
      return undefined;
    }
    directory = parent;
  }
}

function packageNameFromSpecifier(
  specifier: string
): string | undefined {
  if (
    specifier.length === 0 ||
    specifier.startsWith(".") ||
    specifier.startsWith("/") ||
    path.isAbsolute(specifier)
  ) {
    return undefined;
  }
  const segments = specifier.split("/");
  if (specifier.startsWith("@")) {
    return segments.length >= 2
      ? `${segments[0]}/${segments[1]}`
      : undefined;
  }
  return segments[0];
}

function virtualModuleId(metafileId: string): string {
  return `\0esbuild:${metafileId}`;
}

function commonAncestor(paths: ReadonlyArray<string>): string {
  if (paths.length === 0) {
    throw adapterError(
      "esbuild produced no physical projection artifacts",
      "Provide at least one physical output file."
    );
  }
  let ancestor = path.dirname(path.resolve(paths[0]!));
  for (let index = 1; index < paths.length; index++) {
    const directory = path.dirname(path.resolve(paths[index]!));
    while (!isWithin(ancestor, directory)) {
      const parent = path.dirname(ancestor);
      if (parent === ancestor) {
        throw adapterError(
          "projection inputs and outputs do not share a filesystem root",
          "Keep one bundler invocation on a single filesystem volume."
        );
      }
      ancestor = parent;
    }
  }
  return ancestor;
}

function isWithin(root: string, candidate: string): boolean {
  const relative = path.relative(root, candidate);
  return (
    relative.length === 0 ||
    (relative !== ".." &&
      !relative.startsWith(`..${path.sep}`) &&
      !path.isAbsolute(relative))
  );
}

function comparePaths(left: string, right: string): number {
  return Buffer.compare(
    Buffer.from(left, "utf8"),
    Buffer.from(right, "utf8")
  );
}

function samePath(left: string, right: string): boolean {
  return pathKey(left) === pathKey(right);
}

function pathKey(value: string): string {
  const resolved = path.resolve(value);
  return process.platform === "win32"
    ? resolved.toLowerCase()
    : resolved;
}

async function writeAtomic(
  manifestPath: string,
  contents: string
): Promise<void> {
  await mkdir(path.dirname(manifestPath), { recursive: true });
  const sequence = temporaryFileSequence++;
  const temporaryPath = `${manifestPath}.tmp-${process.pid}-${sequence}`;
  try {
    const handle = await open(temporaryPath, "wx");
    try {
      await handle.writeFile(contents, "utf8");
      await handle.sync();
    } finally {
      await handle.close();
    }
    await rename(temporaryPath, manifestPath);
  } finally {
    await rm(temporaryPath, { force: true });
  }
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

function parseVersion(
  value: string
): { major: number; minor: number; patch: number } | undefined {
  const segments = value.split(".");
  if (segments.length < 3) return undefined;
  const major = Number(segments[0]);
  const minor = Number(segments[1]);
  const patchText = segments[2]!;
  let end = 0;
  while (
    end < patchText.length &&
    patchText.charCodeAt(end) >= 48 &&
    patchText.charCodeAt(end) <= 57
  ) {
    end++;
  }
  if (end === 0) return undefined;
  const patch = Number(patchText.slice(0, end));
  return Number.isInteger(major) &&
    Number.isInteger(minor) &&
    Number.isInteger(patch)
    ? { major, minor, patch }
    : undefined;
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

function adapterError(
  title: string,
  help: string
): ProjectionError {
  return new ProjectionError([
    createDiagnostic("PROJ-C013", { help: `${title}. ${help}` }),
  ]);
}

function isMissingFile(error: unknown): boolean {
  if (!(error instanceof Error)) return false;
  const code = (error as Error & { code?: unknown }).code;
  return code === "ENOENT" || code === "ENOTDIR";
}

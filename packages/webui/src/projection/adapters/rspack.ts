// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { readFile } from "node:fs/promises";
import * as path from "node:path";
import type {
  AssetInfo,
  AsyncDependenciesBlock,
  Compilation,
  Compiler,
  Dependency,
  Module,
} from "@rspack/core";
import type {
  AdapterContext,
  ModuleKind,
  ModuleNode,
  ResolvedImport,
} from "../graph.js";
import type { ProjectionManifest } from "../manifest.js";
import { ProjectionError, createDiagnostic } from "../diagnostics.js";
import type { ProjectionDiagnostic } from "../diagnostics.js";
import { ProjectionSession } from "../session.js";
import {
  adapterError,
  commonAncestor,
  comparePaths,
  isMissingFile,
  isWithin,
  nearestPackageName,
  packageNameFromSpecifier,
  parseVersion,
  readPhysicalSource,
  samePath,
} from "./shared.js";

const PLUGIN_NAME = "webui-state-projection";
const MANIFEST_FILENAME = "webui-projection.json";
const MAX_CONCATENATION_DEPTH = 64;
const PRESERVED_ASSET = "webuiProjectionPreserved";

/** Configuration for the official Rspack projection adapter. */
export interface RspackProjectionOptions {
  /**
   * Manifest path. Relative values resolve from `compiler.context`.
   *
   * Defaults to `<output.path>/webui-projection.json`.
   */
  readonly manifest?: string;

  /**
   * Runs after a valid manifest is atomically installed.
   *
   * Rspack awaits this callback, so it can rebuild `protocol.bin` before a
   * dependent SSR compilation starts. Rejection fails the compilation.
   */
  readonly afterManifest?: (
    result: RspackProjectionResult
  ) => void | Promise<void>;
}

/** Values supplied to `RspackProjectionOptions.afterManifest`. */
export interface RspackProjectionResult {
  readonly manifest: ProjectionManifest;
  readonly context: AdapterContext;
  readonly manifestPath: string;
  readonly compiler: Compiler;
  readonly compilation: Compilation;
}

/** A plugin object accepted by Rspack's `plugins` array. */
export interface RspackProjectionPlugin {
  apply(compiler: Compiler): void;
}

/** Create the official Rspack projection plugin. */
export function rspackProjection(
  options: RspackProjectionOptions = {}
): RspackProjectionPlugin {
  return {
    apply(compiler): void {
      const version = compiler.rspack.rspackVersion;
      const versionError = validateRspackVersion(version);
      if (versionError) {
        compiler.hooks.thisCompilation.tap(PLUGIN_NAME, (compilation) => {
          compilation.errors.push(
            toCompilationError(new ProjectionError([versionError]))
          );
        });
        return;
      }

      const manifestPath = resolveManifestPath(compiler, options);
      const session = new ProjectionSession();
      compiler.hooks.thisCompilation.tap(PLUGIN_NAME, (compilation) => {
        compilation.hooks.processAssets.tapPromise(
          {
            name: PLUGIN_NAME,
            stage:
              compiler.rspack.Compilation
                .PROCESS_ASSETS_STAGE_ADDITIONAL,
          },
          () =>
            preservePreviousManifest(
              compiler,
              compilation,
              manifestPath
            )
        );
      });
      compiler.hooks.afterEmit.tapPromise(
        PLUGIN_NAME,
        async (compilation) => {
          removePreservedManifestAsset(
            compiler,
            compilation,
            manifestPath
          );
          if (compilation.errors.length > 0) return;
          try {
            const context = await buildAdapterContext(
              compiler,
              compilation,
              manifestPath,
              version
            );
            const manifest = await session.finalize(context);
            await options.afterManifest?.({
              manifest,
              context,
              manifestPath,
              compiler,
              compilation,
            });
          } catch (error: unknown) {
            compilation.errors.push(toCompilationError(error));
          }
        }
      );
    },
  };
}

interface ModuleFact {
  readonly module: Module;
  readonly id: string;
  readonly kind: ModuleKind;
  readonly source: string | undefined;
  readonly packageName: string | undefined;
}

interface ModuleRecord {
  readonly id: string;
  readonly kind: ModuleKind;
  readonly source: string | undefined;
  readonly packageName: string | undefined;
  readonly instances: Set<Module>;
}

async function buildAdapterContext(
  compiler: Compiler,
  compilation: Compilation,
  manifestPath: string,
  version: string
): Promise<AdapterContext> {
  const context = path.resolve(compiler.context);
  const leaves = collectLeafModules(compiler, compilation);
  const packageCache = new Map<string, string | undefined>();
  const facts = await Promise.all(
    [...leaves].map((module) =>
      loadModuleFact(module, context, packageCache, compiler)
    )
  );
  const records = groupModuleFacts(facts);
  const idByModule = new Map<Module, string>();
  for (const fact of facts) idByModule.set(fact.module, fact.id);
  const resolveModuleId = (module: Module): string | undefined =>
    resolvedModuleId(compiler, module, idByModule);

  const modules = new Map<string, ModuleNode>();
  for (const record of records.values()) {
    modules.set(record.id, {
      id: record.id,
      kind: record.kind,
      ...(record.packageName === undefined
        ? {}
        : { packageName: record.packageName }),
      source: record.source,
      imports: collectImports(
        compiler,
        compilation,
        record,
        records,
        resolveModuleId
      ),
    });
  }

  const entries = collectEntries(
    compilation,
    modules,
    resolveModuleId
  );
  const outputPath = outputDirectory(compiler);
  const { outputs, outputContents } =
    buildMembershipAndContents(
      compiler,
      compilation,
      outputPath,
      manifestPath,
      idByModule
    );
  const rootDir = commonAncestor([
    manifestPath,
    ...[...records.values()]
      .filter((record) => record.kind === "file")
      .map((record) => record.id),
    ...outputs.keys(),
  ]);

  return {
    graph: { modules, entries },
    membership: { outputs },
    outputContents,
    rootDir,
    manifestPath,
    bundlerName: "rspack",
    bundlerVersion: version,
  };
}

function collectLeafModules(
  compiler: Compiler,
  compilation: Compilation
): Set<Module> {
  const leaves = new Set<Module>();
  const visited = new Set<Module>();
  const pending = [...compilation.modules];
  while (pending.length > 0) {
    const module = pending.pop()!;
    if (
      visited.has(module) ||
      module instanceof compiler.rspack.ExternalModule
    ) {
      continue;
    }
    visited.add(module);
    if (module instanceof compiler.rspack.ConcatenatedModule) {
      for (const nested of module.modules) pending.push(nested);
    } else {
      leaves.add(module);
    }
    forEachDependency(module, (dependency) => {
      const target = dependencyTarget(compilation, dependency);
      if (target) pending.push(target);
    });
  }
  return leaves;
}

async function loadModuleFact(
  module: Module,
  context: string,
  packageCache: Map<string, string | undefined>,
  compiler: Compiler
): Promise<ModuleFact> {
  if (module instanceof compiler.rspack.NormalModule) {
    const resource = physicalResource(module.resource);
    if (resource !== undefined) {
      const source = await readPhysicalSource(resource);
      if (source !== undefined) {
        const id = path.resolve(resource);
        return {
          module,
          id,
          kind: "file",
          source,
          packageName: await nearestPackageName(id, packageCache),
        };
      }
    }
  }
  return {
    module,
    id: virtualModuleId(module, context),
    kind: "virtual",
    source: originalSourceText(module),
    packageName: undefined,
  };
}

function groupModuleFacts(
  facts: ReadonlyArray<ModuleFact>
): Map<string, ModuleRecord> {
  const records = new Map<string, ModuleRecord>();
  for (const fact of facts) {
    const existing = records.get(fact.id);
    if (existing === undefined) {
      records.set(fact.id, {
        id: fact.id,
        kind: fact.kind,
        source: fact.source,
        packageName: fact.packageName,
        instances: new Set([fact.module]),
      });
      continue;
    }
    if (
      existing.kind !== fact.kind ||
      existing.source !== fact.source ||
      existing.packageName !== fact.packageName
    ) {
      throw adapterError(
        `Rspack exposed conflicting module instances for '${fact.id}'`,
        "Use one source and package identity for each normalized module."
      );
    }
    existing.instances.add(fact.module);
  }
  return records;
}

interface EdgeChoice {
  readonly edge: ResolvedImport;
  readonly priority: number;
}

function collectImports(
  compiler: Compiler,
  compilation: Compilation,
  record: ModuleRecord,
  records: ReadonlyMap<string, ModuleRecord>,
  resolveModuleId: (module: Module) => string | undefined
): ResolvedImport[] {
  const choices = new Map<string, EdgeChoice>();
  for (const module of record.instances) {
    forEachDependency(module, (dependency, fromBlock) => {
      const request = dependency.request;
      if (request === undefined || request.length === 0) return;
      const type = dependency.type;
      const kind: "static" | "dynamic" =
        fromBlock || type === "import()" ? "dynamic" : "static";
      const target = dependencyTarget(compilation, dependency);
      const external =
        target instanceof compiler.rspack.ExternalModule;
      const resolvedId =
        target === null || external
          ? undefined
          : resolveModuleId(target);
      const packageName = external
        ? packageNameFromSpecifier(target.userRequest ?? request)
        : resolvedId === undefined
          ? undefined
          : records.get(resolvedId)?.packageName;
      const edge: ResolvedImport = {
        specifier: request,
        resolvedId,
        external,
        kind,
        ...(packageName === undefined ? {} : { packageName }),
      };
      const hasTarget = external || resolvedId !== undefined;
      const priority =
        (hasTarget ? 0 : 2) + (type.includes("specifier") ? 1 : 0);
      const key = `${request}\0${kind}`;
      const previous = choices.get(key);
      if (previous === undefined || priority < previous.priority) {
        choices.set(key, { edge, priority });
      } else if (
        priority === previous.priority &&
        !sameImport(previous.edge, edge)
      ) {
        throw adapterError(
          `Rspack exposed conflicting resolutions for '${request}' in '${record.id}'`,
          "Ensure one authored import resolves to one module."
        );
      }
    });
  }

  const imports: ResolvedImport[] = [];
  for (const { edge } of choices.values()) {
    if (!edge.external && edge.resolvedId === undefined) {
      // Rspack retains authored dependency records after tree-shaking their
      // targets. Resolved duplicates already won above; a remaining targetless
      // record is not part of the compiled graph.
      continue;
    }
    imports.push(edge);
  }
  imports.sort((left, right) =>
    comparePaths(
      `${left.specifier}\0${left.kind}\0${left.resolvedId ?? ""}`,
      `${right.specifier}\0${right.kind}\0${right.resolvedId ?? ""}`
    )
  );
  return imports;
}

function sameImport(
  left: ResolvedImport,
  right: ResolvedImport
): boolean {
  return (
    left.resolvedId === right.resolvedId &&
    left.external === right.external &&
    left.packageName === right.packageName
  );
}

function forEachDependency(
  module: Module,
  visit: (dependency: Dependency, fromBlock: boolean) => void
): void {
  for (const dependency of module.dependencies) {
    visit(dependency, false);
  }
  const pending: AsyncDependenciesBlock[] = [...module.blocks];
  while (pending.length > 0) {
    const block = pending.pop()!;
    for (const dependency of block.dependencies) {
      visit(dependency, true);
    }
    for (const nested of block.blocks) pending.push(nested);
  }
}

function dependencyTarget(
  compilation: Compilation,
  dependency: Dependency
): Module | null {
  return (
    compilation.moduleGraph.getModule(dependency) ??
    compilation.moduleGraph.getResolvedModule(dependency)
  );
}

function collectEntries(
  compilation: Compilation,
  modules: ReadonlyMap<string, ModuleNode>,
  resolveModuleId: (module: Module) => string | undefined
): string[] {
  const entries = new Set<string>();
  for (const data of compilation.entries.values()) {
    for (const dependency of data.dependencies) {
      const target = dependencyTarget(compilation, dependency);
      const id =
        target === null ? undefined : resolveModuleId(target);
      if (id !== undefined && modules.has(id)) {
        entries.add(id);
        break;
      }
      const request = dependency.request;
      if (request !== undefined) {
        const physical = physicalResource(request);
        if (physical !== undefined && modules.has(physical)) {
          entries.add(physical);
          break;
        }
      }
    }
  }
  if (entries.size === 0) {
    throw adapterError(
      "Rspack exposed no projection entry modules",
      "Configure at least one physical or virtual client entry."
    );
  }
  return [...entries].sort(comparePaths);
}

function buildMembershipAndContents(
  compiler: Compiler,
  compilation: Compilation,
  outputPath: string,
  manifestPath: string,
  idByModule: ReadonlyMap<Module, string>
): {
  readonly outputs: ReadonlyMap<string, ReadonlySet<string>>;
  readonly outputContents: ReadonlyMap<string, Uint8Array>;
} {
  const outputs = new Map<string, Set<string>>();
  const assets = new Map<string, string>();
  for (const chunk of compilation.chunks) {
    const members = collectChunkMembers(
      compiler,
      compilation,
      chunk,
      idByModule
    );
    for (const file of chunk.files) {
      if (!isJavaScriptAsset(compilation, file)) continue;
      const outputId = path.resolve(outputPath, file);
      if (samePath(outputId, manifestPath)) {
        throw adapterError(
          "the projection manifest collides with a Rspack output",
          `Choose a manifest path other than '${file}'.`
        );
      }
      assets.set(outputId, file);
      const existing = outputs.get(outputId);
      if (existing === undefined) {
        outputs.set(outputId, new Set(members));
      } else {
        for (const member of members) existing.add(member);
      }
    }
  }
  if (outputs.size === 0) {
    throw adapterError(
      "Rspack emitted no JavaScript chunks",
      "Projection requires at least one emitted JavaScript asset."
    );
  }

  const outputContents = new Map<string, Uint8Array>();
  for (const [outputId, file] of assets) {
    const asset = compilation.getAsset(file);
    if (asset === undefined) {
      throw adapterError(
        `Rspack omitted final bytes for '${file}'`,
        "Projection requires every final JavaScript compilation asset."
      );
    }
    outputContents.set(outputId, asset.source.buffer());
  }
  return { outputs, outputContents };
}

function collectChunkMembers(
  compiler: Compiler,
  compilation: Compilation,
  chunk: Parameters<
    Compilation["chunkGraph"]["getChunkModulesIterable"]
  >[0],
  idByModule: ReadonlyMap<Module, string>
): Set<string> {
  const members = new Set<string>();
  const visited = new Set<Module>();
  const pending = [
    ...compilation.chunkGraph.getChunkModulesIterable(chunk),
  ];
  while (pending.length > 0) {
    const module = pending.pop()!;
    if (visited.has(module)) continue;
    visited.add(module);
    if (module instanceof compiler.rspack.ConcatenatedModule) {
      for (const nested of module.modules) pending.push(nested);
      continue;
    }
    const id = idByModule.get(module);
    if (id !== undefined) members.add(id);
  }
  return members;
}

function isJavaScriptAsset(
  compilation: Compilation,
  file: string
): boolean {
  const type = compilation.getAsset(file)?.info.assetType;
  if (type !== undefined) {
    return type === "javascript" || type === "js";
  }
  const clean = stripResourceSuffix(file);
  return (
    clean.endsWith(".js") ||
    clean.endsWith(".mjs") ||
    clean.endsWith(".cjs")
  );
}

function resolvedModuleId(
  compiler: Compiler,
  module: Module,
  idByModule: ReadonlyMap<Module, string>
): string | undefined {
  let current = module;
  for (let depth = 0; depth < MAX_CONCATENATION_DEPTH; depth++) {
    if (current instanceof compiler.rspack.ExternalModule) {
      return undefined;
    }
    const direct = idByModule.get(current);
    if (direct !== undefined) return direct;
    if (!(current instanceof compiler.rspack.ConcatenatedModule)) {
      return undefined;
    }
    current = current.rootModule;
  }
  return undefined;
}

function virtualModuleId(module: Module, context: string): string {
  const identifier =
    module.libIdent({ context }) ?? module.identifier();
  return `\0rspack:${stableVirtualIdentifier(identifier, context)}`;
}

function stableVirtualIdentifier(
  identifier: string,
  context: string
): string {
  const normalized = toPosix(identifier);
  let root = toPosix(path.resolve(context));
  while (root.endsWith("/")) root = root.slice(0, -1);
  const comparable =
    process.platform === "win32" ? normalized.toLowerCase() : normalized;
  const comparableRoot =
    process.platform === "win32" ? root.toLowerCase() : root;
  let offset = 0;
  let match = comparable.indexOf(comparableRoot);
  let stable = "";
  while (match >= 0) {
    stable += `${normalized.slice(offset, match)}<root>`;
    offset = match + root.length;
    match = comparable.indexOf(comparableRoot, offset);
  }
  stable += normalized.slice(offset);
  return stable.startsWith("./") ? stable.slice(2) : stable;
}

function physicalResource(value: string): string | undefined {
  const clean = stripResourceSuffix(value);
  return path.isAbsolute(clean) ? path.resolve(clean) : undefined;
}

function originalSourceText(module: Module): string | undefined {
  const source = module.originalSource();
  if (source === null) return undefined;
  const value = source.source();
  return typeof value === "string"
    ? value
    : value.toString("utf8");
}

async function preservePreviousManifest(
  compiler: Compiler,
  compilation: Compilation,
  manifestPath: string
): Promise<void> {
  const outputPath = outputDirectory(compiler);
  const name = manifestAssetName(outputPath, manifestPath);
  if (name === undefined) return;
  const existing = compilation.getAsset(name);
  if (existing !== undefined) {
    const preserved = existing.info[PRESERVED_ASSET];
    if (preserved !== true) {
      throw adapterError(
        "the projection manifest collides with a Rspack asset",
        `Choose a manifest path other than '${name}'.`
      );
    }
    return;
  }

  const previous = await readPreviousManifest(manifestPath);
  if (previous === undefined) return;
  const info: AssetInfo = { [PRESERVED_ASSET]: true };
  compilation.emitAsset(
    name,
    new compiler.rspack.sources.RawSource(previous),
    info
  );
}

function removePreservedManifestAsset(
  compiler: Compiler,
  compilation: Compilation,
  manifestPath: string
): void {
  const name = manifestAssetName(
    outputDirectory(compiler),
    manifestPath
  );
  if (
    name !== undefined &&
    compilation.getAsset(name)?.info[PRESERVED_ASSET] === true
  ) {
    compilation.deleteAsset(name);
  }
}

function manifestAssetName(
  outputPath: string,
  manifestPath: string
): string | undefined {
  if (!isWithin(outputPath, manifestPath)) return undefined;
  const name = toPosix(path.relative(outputPath, manifestPath));
  return name.length === 0 || name.startsWith("..")
    ? undefined
    : name;
}

async function readPreviousManifest(
  manifestPath: string
): Promise<Buffer | undefined> {
  try {
    return await readFile(manifestPath);
  } catch (error: unknown) {
    if (isMissingFile(error)) return undefined;
    throw error;
  }
}

function resolveManifestPath(
  compiler: Compiler,
  options: RspackProjectionOptions
): string {
  return options.manifest === undefined
    ? path.join(outputDirectory(compiler), MANIFEST_FILENAME)
    : path.resolve(compiler.context, options.manifest);
}

function outputDirectory(compiler: Compiler): string {
  const outputPath = compiler.outputPath || compiler.options.output.path;
  if (!outputPath) {
    throw adapterError(
      "the Rspack compiler did not define output.path",
      "Configure output.path so projection outputs can be validated."
    );
  }
  return path.resolve(outputPath);
}

function validateRspackVersion(
  version: string
): ProjectionDiagnostic | undefined {
  const parsed = parseVersion(version);
  if (
    parsed === undefined ||
    parsed.major !== 2 ||
    (parsed.minor === 0 && parsed.patch < 1)
  ) {
    return createDiagnostic("PROJ-P004", {
      help: `Install an application-owned @rspack/core peer compatible with ^2.0.1; found ${version}.`,
    });
  }
  return undefined;
}

function toCompilationError(error: unknown): Error {
  if (!(error instanceof ProjectionError)) {
    return error instanceof Error ? error : new Error(String(error));
  }
  const message = error.diagnostics
    .map((diagnostic) =>
      diagnostic.help === undefined
        ? `${diagnostic.code}: ${diagnostic.title}`
        : `${diagnostic.code}: ${diagnostic.title} - ${diagnostic.help}`
    )
    .join("\n");
  const result = new Error(message || error.message);
  result.name =
    error.diagnostics[0]?.code ?? "ProjectionError";
  return result;
}

function stripResourceSuffix(value: string): string {
  const query = value.indexOf("?");
  const hash = value.indexOf("#");
  if (query === -1) return hash === -1 ? value : value.slice(0, hash);
  if (hash === -1) return value.slice(0, query);
  return value.slice(0, Math.min(query, hash));
}

function toPosix(value: string): string {
  return value.split("\\").join("/");
}

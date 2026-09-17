// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import * as path from "node:path";
import type { ModuleNode } from "./graph.js";
import { supportedTypeScriptMajor } from "./typescript-version.js";

export type * from "typescript/unstable/ast";

type AstApi = typeof import("typescript/unstable/ast");
type NativeSyncApi = typeof import("typescript/unstable/sync");
type NativeFileSystemApi = typeof import("typescript/unstable/fs");
type NativeFileSystem = ReturnType<
  NativeFileSystemApi["createVirtualFileSystem"]
>;

interface ParseResult {
  readonly sourceFile: import("typescript/unstable/ast").SourceFile;
  readonly hasDiagnostics: boolean;
}

export interface ProjectionParser {
  parse(moduleId: string): ParseResult | undefined;
  close(): void;
}

export class TypeScriptApiUnavailableError extends Error {
  constructor(cause: unknown) {
    super(
      "Reinstall the application-owned TypeScript peer so its native compiler package is available.",
      { cause }
    );
    this.name = "TypeScriptApiUnavailableError";
  }
}

const EMPTY_PARSER: ProjectionParser = {
  parse: () => undefined,
  close: () => {},
};

const versionModule = await import("typescript");
const majorVersion = supportedTypeScriptMajor(versionModule.version);
const nativeAst =
  majorVersion === 7
    ? await import("typescript/unstable/ast")
    : undefined;
const astApi = (nativeAst ??
  (versionModule as unknown as AstApi));
const nativeSync =
  majorVersion === 7
    ? await import("typescript/unstable/sync")
    : undefined;
const nativeFileSystem =
  majorVersion === 7
    ? await import("typescript/unstable/fs")
    : undefined;
let nativeService: NativeService | undefined;
let nativeBuildId = 0;

interface NativeService {
  readonly api: InstanceType<NativeSyncApi["API"]>;
  readonly fileSystem: NativeFileSystem;
}

export const {
  NodeFlags,
  ScriptKind,
  ScriptTarget,
  SyntaxKind,
  isCallExpression,
  isClassDeclaration,
  isClassExpression,
  isExportAssignment,
  isExportDeclaration,
  isFunctionDeclaration,
  isIdentifier,
  isImportDeclaration,
  isNamedImports,
  isNamespaceExport,
  isNamespaceImport,
  isPropertyAccessExpression,
  isPropertyDeclaration,
  isStringLiteral,
  isVariableStatement,
} = astApi;

export function forEachChild(
  node: import("typescript/unstable/ast").Node,
  callback: (child: import("typescript/unstable/ast").Node) => void
): void {
  const nativeNode = node as typeof node & {
    forEachChild?: (visitor: typeof callback) => unknown;
  };
  if (nativeNode.forEachChild !== undefined) {
    nativeNode.forEachChild(callback);
    return;
  }
  const legacyApi = astApi as AstApi & {
    forEachChild(
      current: import("typescript/unstable/ast").Node,
      visitor: typeof callback
    ): unknown;
  };
  legacyApi.forEachChild(node, callback);
}

export function createProjectionParser(
  modules: ReadonlyMap<string, ModuleNode>
): ProjectionParser {
  if (modules.size === 0) return EMPTY_PARSER;
  if (nativeAst !== undefined) {
    return new NativeProjectionParser(
      modules,
      nativeSync as NativeSyncApi,
      nativeFileSystem as NativeFileSystemApi
    );
  }
  return new LegacyProjectionParser(modules, astApi);
}

class LegacyProjectionParser implements ProjectionParser {
  // TypeScript 6 exposes its JavaScript parser from the package root. Delete
  // this class and the fallback in createProjectionParser when support ends.
  constructor(
    private readonly modules: ReadonlyMap<string, ModuleNode>,
    private readonly api: AstApi
  ) {}

  parse(moduleId: string): ParseResult | undefined {
    const node = this.modules.get(moduleId);
    if (node?.source === undefined) return undefined;
    const legacyApi = this.api as AstApi & {
      createSourceFile(
        fileName: string,
        sourceText: string,
        languageVersion: import("typescript/unstable/ast").ScriptTarget,
        setParentNodes: boolean,
        scriptKind: import("typescript/unstable/ast").ScriptKind
      ): import("typescript/unstable/ast").SourceFile;
    };
    const sourceFile = legacyApi.createSourceFile(
      moduleId,
      node.source,
      ScriptTarget.Latest,
      false,
      scriptKindForExtension(path.extname(moduleId))
    );
    const withDiagnostics = sourceFile as typeof sourceFile & {
      readonly parseDiagnostics?: readonly unknown[];
    };
    return {
      sourceFile,
      hasDiagnostics: (withDiagnostics.parseDiagnostics?.length ?? 0) > 0,
    };
  }

  close(): void {}
}

class NativeProjectionParser implements ProjectionParser {
  private readonly service: NativeService;
  private snapshot: ReturnType<
    InstanceType<NativeSyncApi["API"]>["updateSnapshot"]
  > | undefined;
  private readonly paths = new Map<string, string>();
  private readonly openFiles: string[] = [];
  private readonly buildId: number;

  constructor(
    private readonly modules: ReadonlyMap<string, ModuleNode>,
    syncApi: NativeSyncApi,
    fileSystemApi: NativeFileSystemApi
  ) {
    this.service = getNativeService(syncApi, fileSystemApi);
    this.buildId = nativeBuildId;
    nativeBuildId++;
  }

  parse(moduleId: string): ParseResult | undefined {
    let virtualPath = this.paths.get(moduleId);
    if (virtualPath === undefined) {
      const node = this.modules.get(moduleId);
      if (node?.source === undefined) return undefined;
      const extension = path.extname(moduleId) || ".ts";
      virtualPath = normalizeVirtualPath(
        path.join(
          process.cwd(),
          ".webui-projection",
          `${this.buildId}-${this.paths.size}${extension}`
        )
      );
      this.service.fileSystem.writeFile?.(virtualPath, node.source);

      let nextSnapshot: ReturnType<
        InstanceType<NativeSyncApi["API"]>["updateSnapshot"]
      >;
      try {
        nextSnapshot = this.service.api.updateSnapshot({
          openFiles: [virtualPath],
        });
      } catch (error: unknown) {
        removeVirtualFiles(this.service.fileSystem, [virtualPath]);
        throw error;
      }

      const previousSnapshot = this.snapshot;
      this.snapshot = nextSnapshot;
      this.openFiles.push(virtualPath);
      this.paths.set(moduleId, virtualPath);
      previousSnapshot?.dispose();
    }

    const project = this.snapshot?.getDefaultProjectForFile(virtualPath);
    if (project === undefined) return undefined;
    const sourceFile = project.program.getSourceFile(virtualPath);
    if (sourceFile === undefined) return undefined;
    return {
      sourceFile,
      hasDiagnostics:
        project.program.getSyntacticDiagnostics(virtualPath).length > 0,
    };
  }

  close(): void {
    try {
      this.snapshot?.dispose();
    } finally {
      this.snapshot = undefined;
      try {
        if (this.openFiles.length === 0) return;
        const closed = this.service.api.updateSnapshot({
          closeFiles: this.openFiles,
        });
        closed.dispose();
      } finally {
        removeVirtualFiles(this.service.fileSystem, this.openFiles);
        this.openFiles.length = 0;
        this.paths.clear();
      }
    }
  }
}

function getNativeService(
  syncApi: NativeSyncApi,
  fileSystemApi: NativeFileSystemApi
): NativeService {
  if (nativeService !== undefined) return nativeService;
  const fileSystem = fileSystemApi.createVirtualFileSystem({});
  try {
    nativeService = {
      api: new syncApi.API({
        cwd: normalizeVirtualPath(process.cwd()),
        fs: fileSystem,
      }),
      fileSystem,
    };
    return nativeService;
  } catch (error: unknown) {
    throw new TypeScriptApiUnavailableError(error);
  }
}

function removeVirtualFiles(
  fileSystem: NativeFileSystem,
  paths: readonly string[]
): void {
  for (const virtualPath of paths) {
    fileSystem.removeFile?.(virtualPath);
  }
}

function normalizeVirtualPath(value: string): string {
  const normalized = value.replaceAll("\\", "/");
  return process.platform === "win32"
    ? normalized.toLowerCase()
    : normalized;
}

function scriptKindForExtension(
  extension: string
): import("typescript/unstable/ast").ScriptKind {
  switch (extension) {
    case ".tsx":
      return ScriptKind.TSX;
    case ".jsx":
      return ScriptKind.JSX;
    case ".js":
    case ".mjs":
    case ".cjs":
      return ScriptKind.JS;
    default:
      return ScriptKind.TS;
  }
}

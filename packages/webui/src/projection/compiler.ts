// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

/**
 * Bundler-neutral TypeScript AST semantic compiler for WebUI state
 * projection.
 *
 * `compileProjection()` consumes only the adapter-resolved `AdapterContext`
 * (see `graph.ts`) — it never performs filesystem or package resolution.
 * It parses supported source kinds with the TypeScript compiler API, builds a
 * deterministic per-file symbol graph, resolves `observable`/`attr`/
 * `WebUIElement` identities through imports/aliases/namespaces/re-exports,
 * associates component tags through the two supported `define()` forms,
 * proves exact reactive key sets by walking inheritance iteratively, and
 * emits a deterministic `ProjectionManifest`.
 *
 * See DESIGN.md §"Bundler-Neutral State Projection Compiler" for the
 * authoritative specification this module implements.
 */

import * as path from "node:path";
import { createRequire } from "node:module";
import * as ts from "typescript";
import type { AdapterContext, ModuleNode } from "./graph.js";
import type { ComponentEntry, ProjectionManifest } from "./manifest.js";
import {
  MANIFEST_SCHEMA,
  VIRTUAL_HASH,
  compareUtf8,
  computeBuildId,
  hashContent,
} from "./manifest.js";
import { ProjectionError, createDiagnostic } from "./diagnostics.js";
import type { ProjectionDiagnostic } from "./diagnostics.js";

/** The well-known framework package specifier recognized by literal text. */
const FRAMEWORK_SPECIFIER = "@microsoft/webui-framework";

/**
 * Sentinel "module id" representing the framework package. Never a key in
 * `ctx.graph.modules`; recognized structurally wherever a specifier equals
 * `FRAMEWORK_SPECIFIER`, independent of the adapter-resolved graph.
 */
const FRAMEWORK_SENTINEL = "\0framework:@microsoft/webui-framework";

const SUPPORTED_EXTENSIONS = new Set([
  ".ts",
  ".tsx",
  ".mts",
  ".cts",
  ".js",
  ".jsx",
  ".mjs",
  ".cjs",
]);

// ---------------------------------------------------------------------------
// Scope/export binding model (per-file symbol tables)
// ---------------------------------------------------------------------------

/** What a name bound in a module's local scope refers to. */
type ScopeBinding =
  | { readonly kind: "localClass"; readonly node: ts.ClassLikeDeclaration }
  | { readonly kind: "localOther" }
  | { readonly kind: "import"; readonly specifier: string; readonly importedName: string }
  | { readonly kind: "namespaceImport"; readonly specifier: string };

/** What a module's exported name forwards to. */
type ExportBinding =
  | { readonly kind: "localRef"; readonly localName: string }
  | { readonly kind: "class"; readonly node: ts.ClassLikeDeclaration }
  | { readonly kind: "reexport"; readonly specifier: string; readonly importedName: string }
  | { readonly kind: "namespace"; readonly specifier: string };

/** A `export * from '...'` forwarding candidate. */
type StarReexport = { readonly specifier: string };

interface DefineCallSite {
  readonly moduleId: string;
  readonly form: "class" | "customElements";
  readonly node: ts.CallExpression;
  readonly tagArg: ts.Expression | undefined;
  readonly classArg: ts.Expression;
  readonly argCountOk: boolean;
}

interface ModuleAnalysis {
  readonly moduleId: string;
  readonly scope: Map<string, ScopeBinding>;
  readonly exports: Map<string, ExportBinding>;
  readonly starReexports: StarReexport[];
  readonly defineCalls: DefineCallSite[];
}

interface ExactStateKeys {
  readonly hydrationKeys: string[];
  readonly navigationKeys: string[];
}

// ---------------------------------------------------------------------------
// Resolution result model
// ---------------------------------------------------------------------------

type Resolved =
  | { readonly kind: "class"; readonly moduleId: string; readonly node: ts.ClassLikeDeclaration }
  | { readonly kind: "frameworkObservable" }
  | { readonly kind: "frameworkAttr" }
  | { readonly kind: "frameworkElement" }
  | { readonly kind: "namespace"; readonly moduleId: string }
  | { readonly kind: "other" }
  | { readonly kind: "unresolved"; readonly reason: "no-module" | "no-source" | "no-export" | "circular" };

// ---------------------------------------------------------------------------
// Public entry point
// ---------------------------------------------------------------------------

/**
 * Compiles the projection manifest for a single bundler invocation.
 *
 * Pure function of `ctx`: never touches the filesystem, never executes user
 * modules. Throws `ProjectionError` carrying every hard diagnostic found
 * across the whole graph when any candidate's exact keys cannot be proven or
 * any `define()` association is invalid.
 */
export function compileProjection(ctx: AdapterContext): ProjectionManifest {
  const profile = process.env["WEBUI_PROJECTION_PROFILE"] === "1";
  const started = profile ? performance.now() : 0;
  const diagnostics: ProjectionDiagnostic[] = [];
  validateTypeScriptVersion(diagnostics);
  validateAdapterContext(ctx, diagnostics);
  if (diagnostics.length > 0) throw new ProjectionError(diagnostics);

  const analyses = buildAnalyses(ctx, diagnostics);
  const parsed = profile ? performance.now() : 0;
  const candidates = compileDefinedComponents(
    ctx,
    analyses,
    diagnostics
  );
  const analyzed = profile ? performance.now() : 0;

  if (diagnostics.length > 0) {
    throw new ProjectionError(diagnostics);
  }

  const manifest = buildManifest(ctx, candidates);
  if (profile) {
    const finished = performance.now();
    console.error(
      `[webui-projection-compiler] parse=${(parsed - started).toFixed(1)}ms semantics=${(analyzed - parsed).toFixed(1)}ms manifest=${(finished - analyzed).toFixed(1)}ms graphModules=${ctx.graph.modules.size} parsedModules=${analyses.size} components=${Object.keys(manifest.components).length}`
    );
  }
  return manifest;
}

function validateTypeScriptVersion(
  diagnostics: ProjectionDiagnostic[]
): void {
  const parts = parseVersion(ts.version);
  const supported =
    parts !== undefined &&
    parts.major === 6 &&
    (parts.minor > 0 ||
      (parts.minor === 0 && parts.patch >= 3));
  if (!supported) {
    diagnostics.push(
      createDiagnostic("PROJ-P001", {
        help: `Install a supported TypeScript peer (^6.0.3); found ${ts.version}.`,
      })
    );
  }
}

function parseVersion(
  value: string
): { major: number; minor: number; patch: number } | undefined {
  const parts = value.split(".");
  if (parts.length < 3) return undefined;
  const major = Number(parts[0]);
  const minor = Number(parts[1]);
  let patchEnd = 0;
  const patchText = parts[2]!;
  while (
    patchEnd < patchText.length &&
    patchText.charCodeAt(patchEnd) >= 48 &&
    patchText.charCodeAt(patchEnd) <= 57
  ) {
    patchEnd++;
  }
  if (patchEnd === 0) return undefined;
  const patch = Number(patchText.slice(0, patchEnd));
  return Number.isInteger(major) &&
    Number.isInteger(minor) &&
    Number.isInteger(patch)
    ? { major, minor, patch }
    : undefined;
}

function validateAdapterContext(
  ctx: AdapterContext,
  diagnostics: ProjectionDiagnostic[]
): void {
  if (!path.isAbsolute(ctx.rootDir) || !path.isAbsolute(ctx.manifestPath)) {
    diagnostics.push(
      createDiagnostic("PROJ-C013", {
        help: "Adapters must provide absolute rootDir and manifestPath values.",
      })
    );
  }
  for (const entry of ctx.graph.entries) {
    if (!ctx.graph.modules.has(entry)) {
      diagnostics.push(
        createDiagnostic("PROJ-C013", {
          location: entry,
          help: "Every graph entry must identify a module present in graph.modules.",
        })
      );
    }
  }
  for (const [moduleId, node] of ctx.graph.modules) {
    if (moduleId !== node.id) {
      diagnostics.push(
        createDiagnostic("PROJ-C013", {
          location: moduleId,
          help: "Each graph map key must exactly equal ModuleNode.id.",
        })
      );
    }
    if (node.kind === "file" && node.source === undefined) {
      diagnostics.push(
        createDiagnostic("PROJ-C013", {
          location: moduleId,
          help: "Adapters must provide raw UTF-8 source for every physical file module.",
        })
      );
    }
    for (const edge of node.imports) {
      if (
        !edge.external &&
        edge.resolvedId !== undefined &&
        !ctx.graph.modules.has(edge.resolvedId)
      ) {
        diagnostics.push(
          createDiagnostic("PROJ-C013", {
            location: moduleId,
            help: `The resolved target for "${edge.specifier}" must be present in graph.modules.`,
          })
        );
      }
      if (!edge.external && edge.resolvedId === undefined) {
        diagnostics.push(
          createDiagnostic("PROJ-C013", {
            location: moduleId,
            help: `The non-external edge for "${edge.specifier}" is missing resolvedId.`,
          })
        );
      }
    }
  }
  for (const [outputId, members] of ctx.membership.outputs) {
    for (const moduleId of members) {
      if (!ctx.graph.modules.has(moduleId)) {
        diagnostics.push(
          createDiagnostic("PROJ-C013", {
            location: outputId,
            help: `Output membership references unknown module "${moduleId}".`,
          })
        );
      }
    }
    if (!isVirtualId(outputId) && !ctx.outputContents.has(outputId)) {
      diagnostics.push(
        createDiagnostic("PROJ-C014", {
          location: outputId,
          help: "Provide exact emitted bytes so disk manifests cannot bypass stale-output validation.",
        })
      );
    }
  }
}

// ---------------------------------------------------------------------------
// Pass 1 — parse modules, build per-file scope/export tables, find define()s
// ---------------------------------------------------------------------------

function buildAnalyses(
  ctx: AdapterContext,
  diagnostics: ProjectionDiagnostic[]
): AnalysisRegistry {
  const analyses = new AnalysisRegistry(ctx, diagnostics);
  for (const [moduleId, node] of ctx.graph.modules) {
    if (
      node.source !== undefined &&
      SUPPORTED_EXTENSIONS.has(getExtension(moduleId)) &&
      containsPotentialDefineCall(node)
    ) {
      analyses.get(moduleId);
    }
  }
  return analyses;
}

class AnalysisRegistry {
  private readonly analyses = new Map<string, ModuleAnalysis>();
  private readonly attempted = new Set<string>();

  constructor(
    private readonly ctx: AdapterContext,
    private readonly diagnostics: ProjectionDiagnostic[]
  ) {}

  get size(): number {
    return this.analyses.size;
  }

  values(): IterableIterator<ModuleAnalysis> {
    return this.analyses.values();
  }

  get(moduleId: string): ModuleAnalysis | undefined {
    const existing = this.analyses.get(moduleId);
    if (existing || this.attempted.has(moduleId)) return existing;
    this.attempted.add(moduleId);

    const node = this.ctx.graph.modules.get(moduleId);
    if (
      node?.source === undefined ||
      !SUPPORTED_EXTENSIONS.has(getExtension(moduleId))
    ) {
      return undefined;
    }
    const sourceFile = parseModule(moduleId, node);
    if (getParseDiagnostics(sourceFile).length > 0) {
      this.diagnostics.push(
        createDiagnostic("PROJ-C001", {
          location: moduleId,
          help: "Fix the TypeScript/JavaScript syntax error reported by the parser.",
        })
      );
      return undefined;
    }
    const analysis = analyzeModule(moduleId, sourceFile);
    this.analyses.set(moduleId, analysis);
    if (process.env["WEBUI_PROJECTION_PROFILE_DETAIL"] === "1") {
      console.error(`[webui-projection-module] ${moduleId}`);
    }
    return analysis;
  }
}

function containsPotentialDefineCall(node: ModuleNode): boolean {
  const source = node.source ?? "";
  const hasFrameworkEdge = node.imports.some(
    (edge) => edge.packageName === FRAMEWORK_SPECIFIER
  );
  let offset = 0;
  while (offset < source.length) {
    const found = source.indexOf("define", offset);
    if (found < 0) return false;
    const before = found === 0 ? -1 : source.charCodeAt(found - 1);
    const afterIndex = found + "define".length;
    const after =
      afterIndex === source.length
        ? -1
        : source.charCodeAt(afterIndex);
    if (!isIdentifierCode(before) && !isIdentifierCode(after)) {
      let dot = found;
      while (dot > 0 && isWhitespaceCode(source.charCodeAt(dot - 1))) dot--;
      if (dot > 0 && source.charCodeAt(dot - 1) === 46) {
        let open = afterIndex;
        while (
          open < source.length &&
          isWhitespaceCode(source.charCodeAt(open))
        ) {
          open++;
        }
        if (source.charCodeAt(open) === 40) {
          let argument = open + 1;
          while (
            argument < source.length &&
            isWhitespaceCode(source.charCodeAt(argument))
          ) {
            argument++;
          }
          const argumentCode = source.charCodeAt(argument);
          if (
            argumentCode === 34 ||
            argumentCode === 39 ||
            hasFrameworkEdge ||
            receiverIdentifier(source, dot - 1) === "customElements"
          ) {
            return true;
          }
        }
      }
    }
    offset = found + "define".length;
  }
  return false;
}

function receiverIdentifier(
  source: string,
  dotIndex: number
): string {
  let end = dotIndex;
  while (end > 0 && isWhitespaceCode(source.charCodeAt(end - 1))) end--;
  let start = end;
  while (start > 0 && isIdentifierCode(source.charCodeAt(start - 1))) start--;
  return source.slice(start, end);
}

function isWhitespaceCode(code: number): boolean {
  return (
    code === 9 ||
    code === 10 ||
    code === 13 ||
    code === 32
  );
}

function isIdentifierCode(code: number): boolean {
  return (
    (code >= 48 && code <= 57) ||
    (code >= 65 && code <= 90) ||
    (code >= 97 && code <= 122) ||
    code === 36 ||
    code === 95
  );
}

function parseModule(moduleId: string, node: ModuleNode): ts.SourceFile {
  return ts.createSourceFile(
    moduleId,
    node.source ?? "",
    ts.ScriptTarget.Latest,
    /* setParentNodes */ false,
    scriptKindForExtension(getExtension(moduleId))
  );
}

function getParseDiagnostics(sourceFile: ts.SourceFile): readonly ts.Diagnostic[] {
  // The TS parser is error-tolerant; syntax errors are recorded on the
  // source file rather than thrown. `parseDiagnostics` is an established
  // (if internal) property of the parser result used by tooling that only
  // needs a syntax check without building a full `ts.Program`.
  const withDiagnostics = sourceFile as unknown as { parseDiagnostics?: ts.Diagnostic[] };
  return withDiagnostics.parseDiagnostics ?? [];
}

function getExtension(moduleId: string): string {
  const base = moduleId.slice(moduleId.lastIndexOf("/") + 1);
  const dot = base.lastIndexOf(".");
  return dot === -1 ? "" : base.slice(dot);
}

function scriptKindForExtension(ext: string): ts.ScriptKind {
  switch (ext) {
    case ".tsx":
      return ts.ScriptKind.TSX;
    case ".jsx":
      return ts.ScriptKind.JSX;
    case ".js":
    case ".mjs":
    case ".cjs":
      return ts.ScriptKind.JS;
    default:
      return ts.ScriptKind.TS;
  }
}

/** Builds the scope/export tables and collects `define()` call sites for one module. */
function analyzeModule(moduleId: string, sourceFile: ts.SourceFile): ModuleAnalysis {
  const scope = new Map<string, ScopeBinding>();
  const exports = new Map<string, ExportBinding>();
  const starReexports: StarReexport[] = [];

  for (const statement of sourceFile.statements) {
    if (ts.isImportDeclaration(statement)) {
      analyzeImportDeclaration(statement, scope);
    } else if (ts.isExportDeclaration(statement)) {
      analyzeExportDeclaration(statement, scope, exports, starReexports);
    } else if (ts.isClassDeclaration(statement)) {
      analyzeClassDeclaration(statement, scope, exports);
    } else if (ts.isFunctionDeclaration(statement)) {
      analyzeNamedOtherDeclaration(statement.name?.text, isExported(statement), isDefaultExport(statement), scope, exports);
    } else if (ts.isVariableStatement(statement)) {
      analyzeVariableStatement(statement, scope, exports);
    } else if (ts.isExportAssignment(statement)) {
      analyzeExportAssignment(statement, exports);
    }
  }

  const defineCalls = findDefineCalls(moduleId, sourceFile);
  return { moduleId, scope, exports, starReexports, defineCalls };
}

function isExported(node: ts.Node): boolean {
  const modifiers = ts.canHaveModifiers(node) ? ts.getModifiers(node) : undefined;
  return (modifiers ?? []).some((m) => m.kind === ts.SyntaxKind.ExportKeyword);
}

function isDefaultExport(node: ts.Node): boolean {
  const modifiers = ts.canHaveModifiers(node) ? ts.getModifiers(node) : undefined;
  return (modifiers ?? []).some((m) => m.kind === ts.SyntaxKind.DefaultKeyword);
}

function analyzeImportDeclaration(decl: ts.ImportDeclaration, scope: Map<string, ScopeBinding>): void {
  if (!ts.isStringLiteral(decl.moduleSpecifier)) return;
  const specifier = decl.moduleSpecifier.text;
  const clause = decl.importClause;
  if (!clause) return;

  if (clause.name) {
    // Default import: `import Foo from '...'`.
    scope.set(clause.name.text, {
      kind: "import",
      specifier,
      importedName: "default",
    });
  }

  const bindings = clause.namedBindings;
  if (!bindings) return;

  if (ts.isNamespaceImport(bindings)) {
    scope.set(bindings.name.text, { kind: "namespaceImport", specifier });
    return;
  }

  if (ts.isNamedImports(bindings)) {
    for (const spec of bindings.elements) {
      const importedName = spec.propertyName?.text ?? spec.name.text;
      const localName = spec.name.text;
      scope.set(localName, { kind: "import", specifier, importedName });
    }
  }
}

function analyzeExportDeclaration(
  decl: ts.ExportDeclaration,
  scope: Map<string, ScopeBinding>,
  exports: Map<string, ExportBinding>,
  starReexports: StarReexport[]
): void {
  const specifierNode = decl.moduleSpecifier;
  const hasFrom = specifierNode !== undefined && ts.isStringLiteral(specifierNode);
  const specifier = hasFrom ? (specifierNode as ts.StringLiteral).text : undefined;
  if (!decl.exportClause) {
    // `export * from '...'` (only meaningful with a module specifier).
    if (specifier === undefined) return;
    starReexports.push({ specifier });
    return;
  }

  if (ts.isNamespaceExport(decl.exportClause)) {
    const ns = decl.exportClause.name.text;
    if (specifier === undefined) return; // `export * as ns;` without `from` is not valid syntax.
    exports.set(ns, { kind: "namespace", specifier });
    // `export * as ns from '...'` also introduces a local binding usable in this file.
    scope.set(ns, { kind: "namespaceImport", specifier });
    return;
  }

  // NamedExports: `export { a, b as c } [from '...']`.
  for (const spec of decl.exportClause.elements) {
    const exportedName = spec.name.text;
    const sourceName = spec.propertyName?.text ?? spec.name.text;
    if (specifier === undefined) {
      exports.set(exportedName, { kind: "localRef", localName: sourceName });
    } else {
      exports.set(exportedName, { kind: "reexport", specifier, importedName: sourceName });
    }
  }
}

function analyzeClassDeclaration(
  decl: ts.ClassDeclaration,
  scope: Map<string, ScopeBinding>,
  exports: Map<string, ExportBinding>
): void {
  if (!decl.name) {
    if (isDefaultExport(decl)) {
      exports.set("default", { kind: "class", node: decl });
    }
    return;
  }
  const name = decl.name.text;
  scope.set(name, { kind: "localClass", node: decl });
  if (isDefaultExport(decl)) {
    exports.set("default", { kind: "localRef", localName: name });
  } else if (isExported(decl)) {
    exports.set(name, { kind: "localRef", localName: name });
  }
}

function analyzeNamedOtherDeclaration(
  name: string | undefined,
  exported: boolean,
  isDefault: boolean,
  scope: Map<string, ScopeBinding>,
  exports: Map<string, ExportBinding>
): void {
  if (!name) return;
  scope.set(name, { kind: "localOther" });
  if (isDefault) exports.set("default", { kind: "localRef", localName: name });
  else if (exported) exports.set(name, { kind: "localRef", localName: name });
}

function analyzeVariableStatement(
  stmt: ts.VariableStatement,
  scope: Map<string, ScopeBinding>,
  exports: Map<string, ExportBinding>
): void {
  const exported = isExported(stmt);
  for (const decl of stmt.declarationList.declarations) {
    if (!ts.isIdentifier(decl.name)) continue;
    if (decl.initializer && ts.isClassExpression(decl.initializer)) {
      scope.set(decl.name.text, {
        kind: "localClass",
        node: decl.initializer,
      });
      if (exported) {
        exports.set(decl.name.text, {
          kind: "localRef",
          localName: decl.name.text,
        });
      }
    } else {
      analyzeNamedOtherDeclaration(
        decl.name.text,
        exported,
        false,
        scope,
        exports
      );
    }
  }
}

function analyzeExportAssignment(
  statement: ts.ExportAssignment,
  exports: Map<string, ExportBinding>
): void {
  if (statement.isExportEquals) return;
  if (ts.isIdentifier(statement.expression)) {
    exports.set("default", {
      kind: "localRef",
      localName: statement.expression.text,
    });
  } else if (ts.isClassExpression(statement.expression)) {
    exports.set("default", {
      kind: "class",
      node: statement.expression,
    });
  }
}

// ---------------------------------------------------------------------------
// define() call site discovery (full-tree, iterative, explicit stack)
// ---------------------------------------------------------------------------

function findDefineCalls(moduleId: string, sourceFile: ts.SourceFile): DefineCallSite[] {
  const sites: DefineCallSite[] = [];
  const stack: ts.Node[] = [sourceFile];
  while (stack.length > 0) {
    const node = stack.pop()!;
    const site = tryParseDefineCall(moduleId, node);
    if (site) sites.push(site);

    const children: ts.Node[] = [];
    ts.forEachChild(node, (child) => {
      children.push(child);
    });
    for (let i = children.length - 1; i >= 0; i--) stack.push(children[i]!);
  }
  return sites;
}

function tryParseDefineCall(moduleId: string, node: ts.Node): DefineCallSite | undefined {
  if (!ts.isCallExpression(node)) return undefined;
  const callee = node.expression;
  if (!ts.isPropertyAccessExpression(callee) || callee.name.text !== "define") return undefined;

  const isCustomElements = ts.isIdentifier(callee.expression) && callee.expression.text === "customElements";
  const args = node.arguments;

  if (isCustomElements) {
    return {
      moduleId,
      form: "customElements",
      node,
      argCountOk: args.length === 2,
      tagArg: args[0],
      classArg: args[1] ?? callee.expression,
    };
  }

  return {
    moduleId,
    form: "class",
    node,
    argCountOk: args.length === 1,
    tagArg: args[0],
    classArg: callee.expression,
  };
}

// ---------------------------------------------------------------------------
// Pass 2 — associate define() calls with tags and classes
// ---------------------------------------------------------------------------

function compileDefinedComponents(
  ctx: AdapterContext,
  analyses: AnalysisRegistry,
  diagnostics: ProjectionDiagnostic[]
): Map<string, { moduleId: string; stateKeys: ExactStateKeys }> {
  const result = new Map<
    string,
    { moduleId: string; stateKeys: ExactStateKeys }
  >();
  const seenTags = new Set<string>();

  const allSites: DefineCallSite[] = [];
  for (const analysis of analyses.values()) allSites.push(...analysis.defineCalls);

  for (const site of allSites) {
    const analysis = analyses.get(site.moduleId);
    if (
      site.form === "customElements" &&
      analysis?.scope.has("customElements")
    ) {
      continue;
    }

    const classResolution = resolveClassReferenceExpression(
      ctx,
      analyses,
      site.moduleId,
      site.classArg
    );
    if (classResolution.kind !== "class") {
      // `.define()` is a common API outside Web Components. Unknown receivers
      // are ignored here; strict WebUI build coverage catches any real
      // scripted component omitted from the manifest without penalizing
      // unrelated libraries.
      continue;
    }
    if (
      ctx.graph.modules.get(classResolution.moduleId)?.kind ===
      "virtual"
    ) {
      diagnostics.push(
        createDiagnostic("PROJ-C006", {
          location: classResolution.moduleId,
          help: "Disk projection manifests require a physical source module for every shipped WebUI component.",
        })
      );
      continue;
    }
    if (
      !isPotentialWebUIClass(
        ctx,
        analyses,
        classResolution.moduleId,
        classResolution.node
      )
    ) {
      continue;
    }

    if (!site.argCountOk) {
      diagnostics.push(
        createDiagnostic("PROJ-C011", {
          location: site.moduleId,
          help:
            site.form === "customElements"
              ? "customElements.define(tag, Class) requires exactly two arguments."
              : "Class.define(tag) requires exactly one argument.",
        })
      );
      continue;
    }

    if (site.tagArg === undefined || !ts.isStringLiteral(site.tagArg)) {
      diagnostics.push(
        createDiagnostic("PROJ-C008", {
          location: site.moduleId,
          help: "Use a string literal tag name, e.g. MyElement.define(\"my-element\").",
        })
      );
      continue;
    }
    const tag = site.tagArg.text;

    if (seenTags.has(tag)) {
      diagnostics.push(
        createDiagnostic("PROJ-C010", {
          location: site.moduleId,
          help: `Tag "${tag}" is already defined elsewhere. Each custom element tag must be defined exactly once.`,
        })
      );
      continue;
    }

    const stateKeys = computeExactKeys(
      ctx,
      analyses,
      classResolution.moduleId,
      classResolution.node,
      diagnostics
    );
    if (stateKeys === undefined) continue;

    seenTags.add(tag);
    result.set(tag, {
      moduleId: classResolution.moduleId,
      stateKeys,
    });
  }

  return result;
}

function isPotentialWebUIClass(
  ctx: AdapterContext,
  analyses: AnalysisRegistry,
  startModuleId: string,
  startNode: ts.ClassLikeDeclaration
): boolean {
  const stack: Array<{
    readonly moduleId: string;
    readonly node: ts.ClassLikeDeclaration;
  }> = [{ moduleId: startModuleId, node: startNode }];
  const visited = new Set<string>();
  let hasFrameworkDecorator = false;

  while (stack.length > 0) {
    const current = stack.pop()!;
    const key = `${current.moduleId}:${current.node.pos}:${current.node.end}`;
    if (visited.has(key)) continue;
    visited.add(key);

    for (const member of current.node.members) {
      if (!ts.isPropertyDeclaration(member)) continue;
      const decorators = ts.canHaveDecorators(member)
        ? ts.getDecorators(member) ?? []
        : [];
      for (const decorator of decorators) {
        const parsed = parseDecoratorExpression(decorator.expression);
        if (parsed.kind === "unsupported") continue;
        const identity = resolveDecoratorTarget(
          ctx,
          analyses,
          current.moduleId,
          parsed.calleeExpr
        );
        if (
          identity.kind === "frameworkObservable" ||
          identity.kind === "frameworkAttr"
        ) {
          hasFrameworkDecorator = true;
        }
      }
    }

    const base = getExtendsExpression(current.node);
    if (!base) continue;
    const resolved = resolveDecoratorTarget(
      ctx,
      analyses,
      current.moduleId,
      base
    );
    if (resolved.kind === "frameworkElement") return true;
    if (resolved.kind === "class") {
      stack.push({ moduleId: resolved.moduleId, node: resolved.node });
    }
  }

  return hasFrameworkDecorator;
}

function resolveClassReferenceExpression(
  ctx: AdapterContext,
  analyses: AnalysisRegistry,
  moduleId: string,
  expr: ts.Expression
): Resolved {
  if (ts.isClassExpression(expr)) {
    return { kind: "class", moduleId, node: expr };
  }
  if (ts.isIdentifier(expr)) {
    return resolveIdentity(ctx, analyses, moduleId, expr.text, "local");
  }
  if (ts.isPropertyAccessExpression(expr) && ts.isIdentifier(expr.expression)) {
    const ns = resolveIdentity(ctx, analyses, moduleId, expr.expression.text, "local");
    if (ns.kind !== "namespace") return { kind: "unresolved", reason: "no-export" };
    return resolveIdentity(ctx, analyses, ns.moduleId, expr.name.text, "export");
  }
  return { kind: "unresolved", reason: "no-export" };
}

// ---------------------------------------------------------------------------
// Iterative symbol resolution (no recursion; explicit stack)
// ---------------------------------------------------------------------------

function resolveSpecifierToModuleId(
  ctx: AdapterContext,
  currentModuleId: string,
  specifier: string
): string | undefined {
  const node = ctx.graph.modules.get(currentModuleId);
  if (!node) return undefined;

  let match:
    | {
        readonly resolvedId: string | undefined;
        readonly external: boolean;
        readonly packageName?: string;
      }
    | undefined;
  for (const edge of node.imports) {
    if (edge.kind !== "static" || edge.specifier !== specifier) continue;
    if (
      match !== undefined &&
      (match.resolvedId !== edge.resolvedId ||
        match.external !== edge.external ||
        match.packageName !== edge.packageName)
    ) {
      return undefined;
    }
    match = edge;
  }
  if (!match) return undefined;
  if (match.packageName === FRAMEWORK_SPECIFIER) {
    return FRAMEWORK_SENTINEL;
  }
  return match.external ? undefined : match.resolvedId;
}

type ResolutionMode = "local" | "export";
interface ResolutionState {
  readonly mode: ResolutionMode;
  readonly moduleId: string;
  readonly name: string;
}

/**
 * Resolves a name in a module's local scope (`mode: "local"`) or export table
 * (`mode: "export"`) to its ultimate origin, following import/re-export/
 * star-forward chains with an explicit stack instead of recursive calls.
 * Cycles are detected via a visited-state set and reported as `"circular"`.
 */
function resolveIdentity(
  ctx: AdapterContext,
  analyses: AnalysisRegistry,
  startModuleId: string,
  startName: string,
  startMode: ResolutionMode
): Resolved {
  const stack: ResolutionState[] = [{ mode: startMode, moduleId: startModuleId, name: startName }];
  const visited = new Set<string>();
  let hadCircular = false;
  let hadNoSource = false;
  let hadAnyModule = false;

  while (stack.length > 0) {
    const cur = stack.pop()!;
    const key = `${cur.mode[0]}:${cur.moduleId}:${cur.name}`;
    if (visited.has(key)) {
      hadCircular = true;
      continue;
    }
    visited.add(key);

    if (cur.moduleId === FRAMEWORK_SENTINEL) {
      const resolved = resolveFrameworkName(cur.name);
      if (resolved) return resolved;
      continue;
    }

    if (!ctx.graph.modules.has(cur.moduleId)) continue;
    hadAnyModule = true;

    const analysis = analyses.get(cur.moduleId);
    if (!analysis) {
      hadNoSource = true;
      continue;
    }

    const next =
      cur.mode === "local"
        ? stepLocal(ctx, analysis, cur, stack)
        : stepExport(ctx, analysis, cur, stack);
    if (next) return next;
  }

  if (hadCircular) return { kind: "unresolved", reason: "circular" };
  if (hadNoSource) return { kind: "unresolved", reason: "no-source" };
  if (hadAnyModule) return { kind: "unresolved", reason: "no-export" };
  return { kind: "unresolved", reason: "no-module" };
}

function resolveFrameworkName(name: string): Resolved | undefined {
  if (name === "observable") return { kind: "frameworkObservable" };
  if (name === "attr") return { kind: "frameworkAttr" };
  if (name === "WebUIElement") return { kind: "frameworkElement" };
  return undefined;
}

function stepLocal(
  ctx: AdapterContext,
  analysis: ModuleAnalysis,
  cur: ResolutionState,
  stack: ResolutionState[]
): Resolved | undefined {
  const binding = analysis.scope.get(cur.name);
  if (!binding) return undefined;

  switch (binding.kind) {
    case "localClass":
      return { kind: "class", moduleId: cur.moduleId, node: binding.node };
    case "localOther":
      return { kind: "other" };
    case "import": {
      const target = resolveSpecifierToModuleId(
        ctx,
        cur.moduleId,
        binding.specifier
      );
      if (target !== undefined) stack.push({ mode: "export", moduleId: target, name: binding.importedName });
      return undefined;
    }
    case "namespaceImport": {
      const target = resolveSpecifierToModuleId(
        ctx,
        cur.moduleId,
        binding.specifier
      );
      if (target !== undefined) return { kind: "namespace", moduleId: target };
      return undefined;
    }
  }
}

function stepExport(
  ctx: AdapterContext,
  analysis: ModuleAnalysis,
  cur: ResolutionState,
  stack: ResolutionState[]
): Resolved | undefined {
  const exp = analysis.exports.get(cur.name);
  if (exp) {
    switch (exp.kind) {
      case "class":
        return { kind: "class", moduleId: cur.moduleId, node: exp.node };
      case "localRef":
        stack.push({ mode: "local", moduleId: cur.moduleId, name: exp.localName });
        return undefined;
      case "reexport": {
        const target = resolveSpecifierToModuleId(
          ctx,
          cur.moduleId,
          exp.specifier
        );
        if (target !== undefined) stack.push({ mode: "export", moduleId: target, name: exp.importedName });
        return undefined;
      }
      case "namespace": {
        const target = resolveSpecifierToModuleId(
          ctx,
          cur.moduleId,
          exp.specifier
        );
        if (target !== undefined) return { kind: "namespace", moduleId: target };
        return undefined;
      }
    }
  }

  for (const star of analysis.starReexports) {
    const target = resolveSpecifierToModuleId(
      ctx,
      cur.moduleId,
      star.specifier
    );
    if (target !== undefined) {
      stack.push({ mode: "export", moduleId: target, name: cur.name });
    }
  }
  return undefined;
}

// ---------------------------------------------------------------------------
// Pass 3 — exact key derivation (decorators + iterative inheritance walk)
// ---------------------------------------------------------------------------

/** Returns exact initial/navigation surfaces, or `undefined` after diagnostics. */
function computeExactKeys(
  ctx: AdapterContext,
  analyses: AnalysisRegistry,
  startModuleId: string,
  startNode: ts.ClassLikeDeclaration,
  diagnostics: ProjectionDiagnostic[]
): ExactStateKeys | undefined {
  const hydrationKeys = new Set<string>();
  const navigationKeys = new Set<string>();
  const stack: Array<{ moduleId: string; node: ts.ClassLikeDeclaration }> = [
    { moduleId: startModuleId, node: startNode },
  ];
  const visitedClasses = new Set<string>();
  let ok = true;

  while (stack.length > 0) {
    const { moduleId, node } = stack.pop()!;
    const classKey = `${moduleId}:${node.pos}:${node.end}`;
    if (visitedClasses.has(classKey)) continue;
    visitedClasses.add(classKey);

    ok =
      collectOwnKeys(
        ctx,
        analyses,
        moduleId,
        node,
        hydrationKeys,
        navigationKeys,
        diagnostics
      ) && ok;
    ok = climbBaseClass(ctx, analyses, moduleId, node, stack, diagnostics) && ok;
  }

  return ok
    ? {
        hydrationKeys: [...hydrationKeys].sort(compareUtf8),
        navigationKeys: [...navigationKeys].sort(compareUtf8),
      }
    : undefined;
}

function collectOwnKeys(
  ctx: AdapterContext,
  analyses: AnalysisRegistry,
  moduleId: string,
  node: ts.ClassLikeDeclaration,
  hydrationKeys: Set<string>,
  navigationKeys: Set<string>,
  diagnostics: ProjectionDiagnostic[]
): boolean {
  let ok = true;
  for (const member of node.members) {
    if (!ts.isPropertyDeclaration(member)) continue;
    const decorators = ts.canHaveDecorators(member) ? (ts.getDecorators(member) ?? []) : [];
    for (const decorator of decorators) {
      ok =
        applyDecorator(
          ctx,
          analyses,
          moduleId,
          member,
          decorator,
          hydrationKeys,
          navigationKeys,
          diagnostics
        ) && ok;
    }
  }
  return ok;
}

function applyDecorator(
  ctx: AdapterContext,
  analyses: AnalysisRegistry,
  moduleId: string,
  member: ts.PropertyDeclaration,
  decorator: ts.Decorator,
  hydrationKeys: Set<string>,
  navigationKeys: Set<string>,
  diagnostics: ProjectionDiagnostic[]
): boolean {
  if (!ts.isIdentifier(member.name)) {
    diagnostics.push(
      createDiagnostic("PROJ-C007", {
        location: moduleId,
        help: "Decorated reactive properties must use a plain identifier property name.",
      })
    );
    return false;
  }
  const propertyName = member.name.text;

  const parsed = parseDecoratorExpression(decorator.expression);
  if (parsed.kind === "unsupported") {
    diagnostics.push(
      createDiagnostic("PROJ-C007", {
        location: moduleId,
        help: "Only a bare @observable/@attr identifier, a namespaced member, or @attr({ attribute }) is supported.",
      })
    );
    return false;
  }

  const identity = resolveDecoratorTarget(ctx, analyses, moduleId, parsed.calleeExpr);
  if (identity.kind === "frameworkObservable") {
    if (parsed.kind === "call") {
      diagnostics.push(
        createDiagnostic("PROJ-C007", {
          location: moduleId,
          help: "@observable does not accept factory-call arguments.",
        })
      );
      return false;
    }
    hydrationKeys.add(propertyName);
    navigationKeys.add(propertyName);
    return true;
  }
  if (identity.kind === "frameworkAttr") {
    // SSR host attributes win when present. Bootstrap state remains necessary
    // for authored @attr values that are not materialized on the host.
    hydrationKeys.add(propertyName);
    navigationKeys.add(propertyName);
    return true;
  }
  if (identity.kind === "other") return true;

  diagnostics.push(
    createDiagnostic(
      identity.kind === "unresolved" && identity.reason === "circular"
        ? "PROJ-C012"
        : identity.kind === "unresolved" && identity.reason === "no-source"
          ? "PROJ-C006"
          : "PROJ-C004",
      {
      location: moduleId,
      help: "Decorators must resolve to observable/attr exported by @microsoft/webui-framework.",
      }
    )
  );
  return false;
}

type ParsedDecorator =
  | { readonly kind: "bare"; readonly calleeExpr: ts.Expression }
  | { readonly kind: "call"; readonly calleeExpr: ts.Expression; readonly call: ts.CallExpression }
  | { readonly kind: "unsupported" };

function parseDecoratorExpression(expr: ts.Expression): ParsedDecorator {
  if (ts.isIdentifier(expr) || ts.isPropertyAccessExpression(expr)) {
    return { kind: "bare", calleeExpr: expr };
  }
  if (ts.isCallExpression(expr)) {
    const callee = expr.expression;
    if (ts.isIdentifier(callee) || ts.isPropertyAccessExpression(callee)) {
      return { kind: "call", calleeExpr: callee, call: expr };
    }
    return { kind: "unsupported" };
  }
  return { kind: "unsupported" };
}

function resolveDecoratorTarget(
  ctx: AdapterContext,
  analyses: AnalysisRegistry,
  moduleId: string,
  expr: ts.Expression
): Resolved {
  if (ts.isIdentifier(expr)) {
    return resolveIdentity(ctx, analyses, moduleId, expr.text, "local");
  }
  if (ts.isPropertyAccessExpression(expr) && ts.isIdentifier(expr.expression)) {
    const ns = resolveIdentity(ctx, analyses, moduleId, expr.expression.text, "local");
    if (ns.kind !== "namespace") return { kind: "unresolved", reason: "no-export" };
    return resolveIdentity(ctx, analyses, ns.moduleId, expr.name.text, "export");
  }
  return { kind: "unresolved", reason: "no-export" };
}

function climbBaseClass(
  ctx: AdapterContext,
  analyses: AnalysisRegistry,
  moduleId: string,
  node: ts.ClassLikeDeclaration,
  stack: Array<{ moduleId: string; node: ts.ClassLikeDeclaration }>,
  diagnostics: ProjectionDiagnostic[]
): boolean {
  const heritage = getExtendsExpression(node);
  if (!heritage) return true;

  const resolved = resolveDecoratorTarget(ctx, analyses, moduleId, heritage);
  if (resolved.kind === "frameworkElement") return true;
  if (resolved.kind === "class") {
    stack.push({ moduleId: resolved.moduleId, node: resolved.node });
    return true;
  }

  const isSourceUnavailable =
    resolved.kind === "unresolved" && resolved.reason === "no-source";
  const isCircular =
    resolved.kind === "unresolved" && resolved.reason === "circular";
  diagnostics.push(
    createDiagnostic(
      isCircular
        ? "PROJ-C012"
        : isSourceUnavailable
          ? "PROJ-C006"
          : "PROJ-C005",
      {
      location: moduleId,
      help: isSourceUnavailable
        ? "The base class's module has no available source (external/virtual); its keys cannot be proven."
        : "Ensure the base class is a statically resolvable class declaration reachable from the adapter graph.",
      }
    )
  );
  return false;
}

function getExtendsExpression(node: ts.ClassLikeDeclaration): ts.Expression | undefined {
  const heritageClauses = node.heritageClauses ?? [];
  for (const clause of heritageClauses) {
    if (clause.token === ts.SyntaxKind.ExtendsKeyword) {
      return clause.types[0]?.expression;
    }
  }
  return undefined;
}

// ---------------------------------------------------------------------------
// Pass 4/5 — output membership filter, path normalization, manifest assembly
// ---------------------------------------------------------------------------

function buildManifest(
  ctx: AdapterContext,
  candidates: Map<
    string,
    { moduleId: string; stateKeys: ExactStateKeys }
  >
): ProjectionManifest {
  const root = manifestRoot(ctx);
  const components: Record<string, ComponentEntry> = {};

  for (const tag of [...candidates.keys()].sort(compareUtf8)) {
    const { moduleId, stateKeys } = candidates.get(tag)!;
    const outputs = findOutputsContaining(ctx, moduleId)
      .map((outputId) => canonicalOutputId(ctx, outputId))
      .sort(compareUtf8);
    if (outputs.length === 0) continue; // Tree-shaken: silently excluded, no diagnostic.

    components[tag] = {
      module: canonicalModuleId(ctx, moduleId),
      outputs,
      hydrationKeys: stateKeys.hydrationKeys,
      navigationKeys: stateKeys.navigationKeys,
    };
  }

  const inputs = buildInputsMap(ctx);
  const outputs = buildOutputsMap(ctx);
  const analysisHash = computeAnalysisHash(ctx);

  const producerVersion = readProducerVersion();
  const buildId = computeBuildId({
    producerName: "@microsoft/webui/projection.js",
    producerVersion,
    adapterName: ctx.bundlerName,
    adapterBundler: `${ctx.bundlerName}@${ctx.bundlerVersion}`,
    root,
    analysisHash,
    sortedInputs: Object.entries(inputs),
    sortedOutputs: Object.entries(outputs),
    sortedComponents: Object.keys(components)
      .sort(compareUtf8)
      .map(
        (tag) =>
          [
            tag,
            components[tag]!.module,
            components[tag]!.outputs,
            components[tag]!.hydrationKeys,
            components[tag]!.navigationKeys,
          ] as const
      ),
  });

  return {
    schema: MANIFEST_SCHEMA,
    producer: { name: "@microsoft/webui/projection.js", version: producerVersion },
    adapter: { name: ctx.bundlerName, bundler: `${ctx.bundlerName}@${ctx.bundlerVersion}` },
    root,
    analysisHash,
    buildId,
    outputs,
    inputs,
    components,
  };
}

function findOutputsContaining(ctx: AdapterContext, moduleId: string): string[] {
  const found: string[] = [];
  for (const [outputPath, moduleIds] of ctx.membership.outputs) {
    if (moduleIds.has(moduleId)) found.push(outputPath);
  }
  return found;
}

function buildInputsMap(ctx: AdapterContext): Record<string, string> {
  const entries: Array<[string, string]> = [];
  for (const [moduleId, node] of ctx.graph.modules) {
    const hash =
      node.kind === "virtual"
        ? VIRTUAL_HASH
        : hashContent(node.source ?? "");
    entries.push([canonicalModuleId(ctx, moduleId), hash]);
  }
  entries.sort((left, right) => compareUtf8(left[0], right[0]));
  return Object.fromEntries(entries);
}

function buildOutputsMap(ctx: AdapterContext): Record<string, string> {
  const entries: Array<[string, string]> = [];
  for (const outputId of ctx.membership.outputs.keys()) {
    const hash = isVirtualId(outputId)
      ? VIRTUAL_HASH
      : hashContent(ctx.outputContents.get(outputId)!);
    entries.push([canonicalOutputId(ctx, outputId), hash]);
  }
  entries.sort((left, right) => compareUtf8(left[0], right[0]));
  return Object.fromEntries(entries);
}

function manifestRoot(ctx: AdapterContext): string {
  const rootDir = path.resolve(ctx.rootDir);
  const manifestPath = path.resolve(ctx.manifestPath);
  ensureWithinRoot(rootDir, manifestPath);
  const relative = toPortableSeparators(
    path.relative(path.dirname(manifestPath), rootDir)
  );
  return relative.length === 0 ? "." : relative;
}

function canonicalModuleId(ctx: AdapterContext, moduleId: string): string {
  return isVirtualId(moduleId)
    ? encodeVirtualId(moduleId)
    : canonicalPhysicalPath(ctx.rootDir, moduleId);
}

function canonicalOutputId(ctx: AdapterContext, outputId: string): string {
  return isVirtualId(outputId)
    ? encodeVirtualId(outputId)
    : canonicalPhysicalPath(ctx.rootDir, outputId);
}

function canonicalPhysicalPath(rootDir: string, targetPath: string): string {
  const root = path.resolve(rootDir);
  const target = path.resolve(targetPath);
  ensureWithinRoot(root, target);
  const relative = toPortableSeparators(path.relative(root, target));
  if (relative.length === 0) {
    throw new ProjectionError([
      createDiagnostic("PROJ-S003", {
        location: targetPath,
        help: "Manifest file entries must identify a file below the build root.",
      }),
    ]);
  }
  return relative;
}

function ensureWithinRoot(root: string, target: string): void {
  const relative = path.relative(root, target);
  if (
    relative.length === 0 ||
    (relative !== ".." &&
      !relative.startsWith(`..${path.sep}`) &&
      !path.isAbsolute(relative))
  ) {
    return;
  }
  throw new ProjectionError([
    createDiagnostic("PROJ-S003", {
      location: target,
      help: `Choose a build root that contains the manifest and every physical input/output (${root}).`,
    }),
  ]);
}

function toPortableSeparators(value: string): string {
  return value.split(path.sep).join("/");
}

function isVirtualId(value: string): boolean {
  return value.charCodeAt(0) === 0;
}

function encodeVirtualId(value: string): string {
  const bytes = Buffer.from(value.slice(1), "utf8");
  let encoded = "virtual:";
  for (const byte of bytes) encoded += byte.toString(16).padStart(2, "0");
  return encoded;
}

function computeAnalysisHash(ctx: AdapterContext): string {
  const records: string[] = [];
  appendAnalysisRecord(records, "entries", [
    String(ctx.graph.entries.length),
  ]);
  const entries = [...ctx.graph.entries]
    .map((entry) => canonicalModuleId(ctx, entry))
    .sort(compareUtf8);
  for (const entry of entries) {
    appendAnalysisRecord(records, "entry", [entry]);
  }

  const modules = [...ctx.graph.modules.values()].sort((left, right) =>
    compareUtf8(canonicalModuleId(ctx, left.id), canonicalModuleId(ctx, right.id))
  );
  appendAnalysisRecord(records, "modules", [String(modules.length)]);
  for (const node of modules) {
    const imports = [...node.imports].sort(compareImportEdges);
    const fields = [
      canonicalModuleId(ctx, node.id),
      node.kind,
      node.packageName ?? "",
      node.source === undefined ? VIRTUAL_HASH : hashContent(node.source),
      String(imports.length),
    ];
    for (const edge of imports) {
      fields.push(
        edge.specifier,
        edge.resolvedId === undefined
          ? ""
          : canonicalModuleId(ctx, edge.resolvedId),
        edge.external ? "external" : "internal",
        edge.kind,
        edge.packageName ?? ""
      );
    }
    appendAnalysisRecord(records, "module", fields);
  }

  const outputs = [...ctx.membership.outputs.entries()].sort((left, right) =>
    compareUtf8(
      canonicalOutputId(ctx, left[0]),
      canonicalOutputId(ctx, right[0])
    )
  );
  appendAnalysisRecord(records, "memberships", [String(outputs.length)]);
  for (const [outputId, memberIds] of outputs) {
    const members = [...memberIds]
      .map((memberId) => canonicalModuleId(ctx, memberId))
      .sort(compareUtf8);
    appendAnalysisRecord(records, "membership", [
      canonicalOutputId(ctx, outputId),
      String(members.length),
      ...members,
    ]);
  }
  return hashContent(records.join(""));
}

function compareImportEdges(
  left: ModuleNode["imports"][number],
  right: ModuleNode["imports"][number]
): number {
  const leftKey = `${left.specifier}\0${left.resolvedId ?? ""}\0${left.external ? "1" : "0"}\0${left.kind}\0${left.packageName ?? ""}`;
  const rightKey = `${right.specifier}\0${right.resolvedId ?? ""}\0${right.external ? "1" : "0"}\0${right.kind}\0${right.packageName ?? ""}`;
  return compareUtf8(leftKey, rightKey);
}

function appendAnalysisRecord(
  destination: string[],
  label: string,
  fields: ReadonlyArray<string>
): void {
  let record = label;
  for (const field of fields) {
    record += `${Buffer.byteLength(field, "utf8")}:${field}`;
  }
  destination.push(`${record}\n`);
}

let cachedProducerVersion: string | undefined;

/**
 * Reads this package's own `package.json` `version` field once, for the
 * manifest `producer.version` field. This never resolves user modules or
 * bundler-owned specifiers; it only reads this package's own metadata via a
 * synchronous CommonJS-style `require` created for this ESM module.
 */
function readProducerVersion(): string {
  if (cachedProducerVersion !== undefined) return cachedProducerVersion;
  try {
    const require = createRequire(import.meta.url);
    const pkg = require("../../package.json") as { version?: string };
    if (typeof pkg.version !== "string" || pkg.version.length === 0) {
      throw new Error("package version is missing");
    }
    cachedProducerVersion = pkg.version;
  } catch (error: unknown) {
    throw new ProjectionError([
      createDiagnostic("PROJ-C013", {
        help: `Unable to read @microsoft/webui package version: ${error instanceof Error ? error.message : String(error)}`,
      }),
    ]);
  }
  return cachedProducerVersion;
}

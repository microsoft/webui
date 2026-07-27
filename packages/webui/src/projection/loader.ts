// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import type { AdapterContext } from "./graph.js";
import type { ProjectionManifest } from "./manifest.js";
import { ProjectionError, createDiagnostic } from "./diagnostics.js";

type CompilerModule = typeof import("./compiler.js");
let compilerModule: Promise<CompilerModule> | undefined;

/**
 * Lazily load the TypeScript-backed compiler so a missing optional peer is
 * surfaced as a stable projection diagnostic.
 */
export async function compileProjection(
  context: AdapterContext
): Promise<ProjectionManifest> {
  const compiler = await loadCompiler();
  return compiler.compileProjection(context);
}

/** Start compiler/TypeScript loading without waiting for semantic analysis. */
export async function preloadProjectionCompiler(): Promise<void> {
  await loadCompiler();
}

function loadCompiler(): Promise<CompilerModule> {
  compilerModule ??= importCompiler();
  return compilerModule;
}

async function importCompiler(): Promise<CompilerModule> {
  try {
    return await import("./compiler.js");
  } catch (error: unknown) {
    if (isMissingTypeScriptPeer(error)) {
      throw new ProjectionError([
        createDiagnostic("PROJ-P001", {
          help: "Install a compatible application-owned TypeScript peer (^6.0.3) before using @microsoft/webui/projection.js.",
        }),
      ]);
    }
    throw error;
  }
}

function isMissingTypeScriptPeer(error: unknown): boolean {
  if (!(error instanceof Error)) return false;
  const code = (error as Error & { code?: unknown }).code;
  return (
    code === "ERR_MODULE_NOT_FOUND" &&
    (error.message.includes("'typescript'") ||
      error.message.includes('"typescript"') ||
      error.message.includes("typescript/package.json"))
  );
}

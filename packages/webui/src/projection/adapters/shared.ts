// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

/**
 * Bundler-neutral helpers shared by the official WebUI projection adapters.
 *
 * These utilities carry no bundler-specific semantics. They read authored disk
 * source, prove package identity from the nearest `package.json`, derive a
 * common build root, and parse semantic versions. Keeping one implementation
 * lets the esbuild and Rspack adapters behave identically where the underlying
 * facts are the same.
 */

import { readFile } from "node:fs/promises";
import * as path from "node:path";
import { ProjectionError, createDiagnostic } from "../diagnostics.js";

/** A parsed `major.minor.patch` triple; trailing pre-release text is ignored. */
export interface SemanticVersion {
  readonly major: number;
  readonly minor: number;
  readonly patch: number;
}

/**
 * Reads authored UTF-8 source for a physical file, returning `undefined` when
 * the path does not exist. Missing files signal a virtual/non-physical module.
 */
export async function readPhysicalSource(
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

/**
 * Resolves the nearest `package.json` `name` above a physical file.
 *
 * The walk is memoized per directory so repeated lookups across a large module
 * graph stay cheap. This is how an alias that resolves to
 * `@microsoft/webui-framework` keeps that semantic identity regardless of the
 * on-disk path it was aliased to.
 */
export async function nearestPackageName(
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

/**
 * Derives a canonical package name from a bare specifier
 * (`"@scope/name/sub"` → `"@scope/name"`, `"pkg/sub"` → `"pkg"`).
 *
 * Returns `undefined` for relative/absolute specifiers, which carry no package
 * identity.
 */
export function packageNameFromSpecifier(
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

/**
 * Computes the deepest directory that contains every supplied path.
 *
 * All physical inputs, outputs, and the manifest must share this root so the
 * compiler can express them as root-relative manifest keys.
 */
export function commonAncestor(paths: ReadonlyArray<string>): string {
  if (paths.length === 0) {
    throw adapterError(
      "the bundler produced no physical projection artifacts",
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

/** Whether `candidate` is `root` itself or a descendant of it. */
export function isWithin(root: string, candidate: string): boolean {
  const relative = path.relative(root, candidate);
  return (
    relative.length === 0 ||
    (relative !== ".." &&
      !relative.startsWith(`..${path.sep}`) &&
      !path.isAbsolute(relative))
  );
}

/** Cross-platform equality for two absolute paths. */
export function samePath(left: string, right: string): boolean {
  return pathKey(left) === pathKey(right);
}

/** Case-folded (on Windows) resolved key for path comparison and lookup. */
export function pathKey(value: string): string {
  const resolved = path.resolve(value);
  return process.platform === "win32" ? resolved.toLowerCase() : resolved;
}

/** Cross-language lexical ordering over raw UTF-8 bytes, ascending. */
export function comparePaths(left: string, right: string): number {
  return Buffer.compare(
    Buffer.from(left, "utf8"),
    Buffer.from(right, "utf8")
  );
}

/** Parses a `major.minor.patch` prefix, ignoring any pre-release suffix. */
export function parseVersion(value: string): SemanticVersion | undefined {
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

/** Builds a hard adapter-contract error (`PROJ-C013`) with actionable help. */
export function adapterError(title: string, help: string): ProjectionError {
  return new ProjectionError([
    createDiagnostic("PROJ-C013", { help: `${title}. ${help}` }),
  ]);
}

/** Whether an error is a benign "file not found" from the filesystem. */
export function isMissingFile(error: unknown): boolean {
  if (!(error instanceof Error)) return false;
  const code = (error as Error & { code?: unknown }).code;
  return code === "ENOENT" || code === "ENOTDIR";
}

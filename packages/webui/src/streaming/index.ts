// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import * as path from "node:path";
import type { BuildOptions, BuildResult, Plugin } from "esbuild";
import { outputImportClosure } from "../projection/adapters/esbuild-graph.js";

/** Esbuild output identities, not served URLs or a deployment manifest. */
export interface StreamingAsset {
  /** Coordinator key in result.metafile.outputs. */
  readonly entry: string;
  /** Other static output keys in largest-first preload order. */
  readonly imports: readonly string[];
}

/** Unsupported configuration, missing coordinator, or invalid dependency isolation. */
export type StreamingBuildDiagnosticCode = "STREAM-B001" | "STREAM-B002" | "STREAM-B003";

/** Recoverable error from streaming build setup or output inspection. */
export class StreamingBuildError extends Error {
  /** Describe a failed integration and its corrective action without terminal styling. */
  constructor(readonly code: StreamingBuildDiagnosticCode, message: string, help: string) {
    super(`${code}: ${message}\nhelp: ${help}`);
    this.name = "StreamingBuildError";
  }
}

const NAME = "webui-streaming-assets";
const ENTRY = "webui-streaming:coordinator";
const FRAMEWORK = "@microsoft/webui-framework";
const RUNTIME = `${FRAMEWORK}/streaming.js`;

/**
 * Optional esbuild adapter for the bundler-independent streaming entry.
 *
 * Adds one coordinator entry in the application's bundled browser ESM graph.
 * After a successful build/rebuild, call getStreamingAsset(result) before
 * publishing assets. No files, URLs, or manifest formats are added by the adapter.
 */
export function esbuildStreaming(): Plugin {
  return {
    name: NAME,
    setup(build) {
      const options = build.initialOptions;
      validateOptions(options);
      const working = path.resolve(options.absWorkingDir ?? process.cwd());
      const entries = options.entryPoints ?? [];
      const normalized = Array.isArray(entries)
        ? entries.filter(entry => (typeof entry === "string" ? entry : entry.in) !== ENTRY)
        : Object.entries(entries).map(([out, input]) => ({ in: input, out }));
      options.entryPoints = [...normalized, { in: ENTRY, out: "webui-streaming" }];
      options.metafile = true;
      build.onResolve({ filter: /^webui-streaming:coordinator$/ }, () => ({
        path: "coordinator", namespace: "webui-streaming",
      }));
      build.onLoad({ filter: /^coordinator$/, namespace: "webui-streaming" }, () => ({
        contents: `import ${JSON.stringify(RUNTIME)};`, loader: "js", resolveDir: working,
      }));
      build.onResolve(
        { filter: /^@microsoft\/webui-framework\/streaming\.js$/, namespace: "webui-streaming" },
        async args => {
          const resolved = await build.resolve(args.path, {
            kind: "import-statement", resolveDir: working,
          });
          if (resolved.errors.length) return resolved;
          if (resolved.external || resolved.namespace !== "file" || resolved.suffix) {
            throw new StreamingBuildError("STREAM-B002", "coordinator must be bundled with its module identity intact",
              `Resolve ${RUNTIME} to a physical framework module without externalization or URL suffixes.`);
          }
          // Initialization is required, even if a resolver marks modules side-effect-free.
          return { ...resolved, sideEffects: true };
        }
      );
    },
  };
}

/**
 * Inspect a successful esbuild result and identify its isolated coordinator.
 *
 * Returns exact metafile output keys, including hashed names. The host maps
 * these keys to its deployed URLs and may store them in its existing asset
 * manifest. Works with disk, in-memory and context.rebuild() results.
 */
export function getStreamingAsset(result: BuildResult): StreamingAsset {
  const meta = result.metafile;
  const entry = meta && Object.keys(meta.outputs).find(id => meta.outputs[id]!.entryPoint === ENTRY);
  const runtime = meta?.inputs[ENTRY]?.imports.find(edge => (edge.original ?? edge.path) === RUNTIME);
  if (result.errors.length || !meta || !entry || !runtime || runtime.external) {
    throw new StreamingBuildError("STREAM-B002", "no successful coordinator output",
      "Use esbuildStreaming() and inspect the result only after all build plugins have succeeded.");
  }
  const imports = outputImportClosure(meta, entry);
  const staticOutputs = [entry, ...imports];
  if (!staticOutputs.some(id => (meta.outputs[id]?.inputs[runtime.path]?.bytesInOutput ?? 0) > 0)) {
    throw new StreamingBuildError("STREAM-B002", "coordinator initialization was eliminated",
      "Preserve the streaming entry's initialization when applying loaders or tree shaking.");
  }
  const implementation = path.posix.dirname(runtime.path);
  for (const id of [entry, ...outputImportClosure(meta, entry, true)]) {
    const output = meta.outputs[id];
    if (!output || output.cssBundle || output.imports.some(edge => edge.external)) {
      throw new StreamingBuildError("STREAM-B003", "coordinator has an unbundled dependency",
        "Keep the coordinator and its deferred framework support in the same browser JavaScript build.");
    }
    for (const [input, contribution] of Object.entries(output.inputs)) {
      if (input !== ENTRY && contribution.bytesInOutput && !within(implementation, input)) {
        throw new StreamingBuildError("STREAM-B003", "application code entered the coordinator graph",
          "Keep application imports out of the coordinator and its deferred framework dependencies.");
      }
    }
  }
  for (const input of Object.values(meta.inputs)) {
    for (const edge of input.imports) {
      const specifier = edge.original ?? edge.path;
      if (specifier !== FRAMEWORK && !specifier.startsWith(`${FRAMEWORK}/`)) continue;
      if (edge.external || !within(implementation, edge.path)) {
        throw new StreamingBuildError("STREAM-B003", "framework imports have different module identities",
          "Resolve application and coordinator imports to one bundled framework source tree.");
      }
    }
  }
  return { entry, imports };
}

function validateOptions(options: BuildOptions): void {
  if (
    !options.bundle || options.format !== "esm" || !options.splitting ||
    !options.outdir || options.outfile || (options.platform && options.platform !== "browser") ||
    options.inject?.length || options.preserveSymlinks
  ) {
    throw new StreamingBuildError("STREAM-B001", "unsupported streaming build configuration",
      'Use bundle:true, format:"esm", splitting:true and outdir, without global inject or preserveSymlinks.');
  }
  if (options.plugins?.filter(plugin => plugin.name === NAME).length !== 1) {
    throw new StreamingBuildError("STREAM-B001", "streaming plugin must appear once",
      "Add one esbuildStreaming() instance to this build.");
  }
}

function within(root: string, file: string): boolean {
  const relative = path.posix.relative(root, file);
  return relative !== ".." && !relative.startsWith("../") && !path.posix.isAbsolute(relative);
}

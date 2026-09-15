// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { createHash } from "node:crypto";
import { readFile, realpath } from "node:fs/promises";
import * as path from "node:path";
import type { BuildOptions, BuildResult, Metafile, Plugin } from "esbuild";
import { writeAtomic } from "../projection/atomic-write.js";

/** Host-facing asset descriptor version, independent of the streaming wire format. */
export const STREAMING_ASSETS_SCHEMA = "webui.streaming-assets/v1";

/** Coordinator module and its ordered static framework dependencies. */
export interface StreamingCoordinatorAsset {
  /** Served module URL, relative to the output mount without publicPath. */
  readonly src: string;
  readonly type: "module";
  readonly async: true;
  /** Static framework dependency URLs in largest-first preload order. */
  readonly imports: readonly string[];
}

/** Emitted as <outdir>/webui-streaming.json after a successful build. */
export interface StreamingAssetsManifest {
  readonly schema: typeof STREAMING_ASSETS_SCHEMA;
  readonly coordinator: StreamingCoordinatorAsset;
}

/** Stable build diagnostic categories: configuration, dependency, isolation, collision, URL, I/O. */
export type StreamingBuildDiagnosticCode =
  | "STREAM-B001" | "STREAM-B002" | "STREAM-B003"
  | "STREAM-B004" | "STREAM-B005" | "STREAM-B006";

/** Recoverable, actionable error exposed through esbuild's errors[].detail/id. */
export class StreamingBuildError extends Error {
  /** Describe a failed build and the corrective action without terminal styling. */
  constructor(
    readonly code: StreamingBuildDiagnosticCode,
    message: string,
    help: string
  ) {
    super(`${code}: ${message}\nhelp: ${help}`);
    this.name = "StreamingBuildError";
  }
}

const NAME = "webui-streaming-assets";
const ENTRY = "webui-streaming:coordinator";
const FRAMEWORK = "@microsoft/webui-framework";
const RUNTIME = `${FRAMEWORK}/streaming.js`;
const MANIFEST = "webui-streaming.json";

/**
 * Emit an independent coordinator in the application's one code-splitting build.
 *
 * Place last, after esbuildProjection() if used. Requires bundled browser ESM
 * with outdir; global injection, JS banners/footers and preserved symlinks are
 * unsupported because they defeat isolation or shared module identity.
 * All delivery/naming options come from esbuild; there is no second build.
 */
export function esbuildStreaming(): Plugin {
  return {
    name: NAME,
    setup(build) {
      const options = build.initialOptions;
      validateOptions(options);
      const working = path.resolve(options.absWorkingDir ?? process.cwd());
      const outdir = path.resolve(working, options.outdir!);
      const publicPath = options.publicPath ?? "";
      validatePublicPath(publicPath);
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
      build.onEnd(async result => {
        if (result.errors.length) return;
        try {
          await publish(result, { working, outdir, publicPath, write: options.write !== false });
        } catch (error: unknown) {
          const failure = error instanceof StreamingBuildError ? error : new StreamingBuildError(
            "STREAM-B006",
            error instanceof Error ? error.message : String(error),
            "Check the framework package and output directory are readable/writable."
          );
          return { errors: [{ id: failure.code, text: failure.message, detail: failure }] };
        }
      });
    },
  };
}

function validateOptions(options: BuildOptions): void {
  if (
    !options.bundle || options.format !== "esm" || !options.splitting ||
    !options.outdir || options.outfile || (options.platform && options.platform !== "browser") ||
    options.inject?.length || options.banner?.js || options.footer?.js || options.preserveSymlinks
  ) {
    throw new StreamingBuildError("STREAM-B001", "unsupported streaming build configuration",
      'Use bundle:true, format:"esm", splitting:true and outdir. Import application code explicitly, without global inject/banner/footer or preserveSymlinks.');
  }
  const plugins = options.plugins ?? [];
  if (plugins.at(-1)?.name !== NAME || plugins.filter(plugin => plugin.name === NAME).length !== 1) {
    throw new StreamingBuildError("STREAM-B001", "streaming plugin must appear once, last",
      "Place esbuildStreaming() after other plugins so failed validation never publishes assets.");
  }
}

interface OutputContext {
  working: string;
  outdir: string;
  publicPath: string;
  write: boolean;
}

async function publish(result: BuildResult, context: OutputContext): Promise<void> {
  const meta = result.metafile;
  const coordinator = meta && Object.keys(meta.outputs).find(id => meta.outputs[id]!.entryPoint === ENTRY);
  const runtime = meta?.inputs[ENTRY]?.imports.find(edge => (edge.original ?? edge.path) === RUNTIME);
  if (!meta || !coordinator || !runtime || runtime.external) {
    throw new StreamingBuildError("STREAM-B002", "missing bundled coordinator",
      `Bundle ${RUNTIME}; do not externalize or replace its virtual entry.`);
  }
  const implementation = path.dirname(path.resolve(context.working, runtime.path));
  await requireFrameworkPackage(implementation);
  const reached = closure(meta, coordinator, true);
  for (const output of reached) {
    for (const [input, contribution] of Object.entries(meta.outputs[output]!.inputs)) {
      if (input !== ENTRY && contribution.bytesInOutput && !within(implementation, path.resolve(context.working, input))) {
        throw new StreamingBuildError("STREAM-B003", "application code entered the coordinator graph",
          "Keep application imports out of the coordinator and its deferred framework dependencies.");
      }
    }
  }
  for (const input of Object.values(meta.inputs)) {
    for (const edge of input.imports) {
      const specifier = edge.original ?? edge.path;
      if (specifier !== FRAMEWORK && !specifier.startsWith(`${FRAMEWORK}/`)) continue;
      if (edge.external || !within(implementation, path.resolve(context.working, edge.path))) {
        throw new StreamingBuildError("STREAM-B003", "framework imports have different module identities",
          "Resolve application and coordinator imports to one bundled framework source tree.");
      }
    }
  }
  const dependencies = [...closure(meta, coordinator, false)].filter(id => id !== coordinator);
  dependencies.sort((a, b) => meta.outputs[b]!.bytes - meta.outputs[a]!.bytes ||
    Buffer.compare(Buffer.from(a), Buffer.from(b)));
  const manifest: StreamingAssetsManifest = {
    schema: STREAMING_ASSETS_SCHEMA,
    coordinator: {
      src: servedUrl(coordinator, context), type: "module", async: true,
      imports: dependencies.map(id => servedUrl(id, context)),
    },
  };
  const filename = path.join(context.outdir, MANIFEST);
  let physicalOutdir = context.outdir;
  try { physicalOutdir = await realpath(context.outdir); }
  catch (error: unknown) {
    if ((error as NodeJS.ErrnoException).code !== "ENOENT") throw error;
  }
  const physicalManifest = key(path.join(physicalOutdir, MANIFEST));
  for (const id of [...Object.keys(meta.inputs), ...Object.keys(meta.outputs)]) {
    const candidate = key(path.resolve(context.working, id));
    if (candidate === key(filename) || candidate === physicalManifest) {
      throw new StreamingBuildError("STREAM-B004", "streaming manifest collides with a build input/output",
        `Reserve ${MANIFEST} for the streaming descriptor.`);
    }
  }
  const text = JSON.stringify(manifest);
  if (context.write) await writeAtomic(filename, text);
  else {
    if (!result.outputFiles) throw new Error("esbuild omitted outputFiles for write:false");
    const contents = Buffer.from(text);
    result.outputFiles.push({
      path: filename, contents, hash: createHash("sha256").update(contents).digest("hex"),
      get text() {
        return Buffer.from(this.contents.buffer, this.contents.byteOffset, this.contents.byteLength).toString("utf8");
      },
    });
  }
}

function closure(meta: Metafile, entry: string, dynamic: boolean): Set<string> {
  const outputs = new Set([entry]);
  for (const id of outputs) {
    const output = meta.outputs[id];
    if (!output || output.cssBundle) throw new StreamingBuildError("STREAM-B003", "invalid coordinator output graph",
      "Keep coordinator dependencies in browser JavaScript modules.");
    for (const edge of output.imports) {
      if (!dynamic && edge.kind !== "import-statement") continue;
      if (edge.external) throw new StreamingBuildError("STREAM-B002", "external coordinator dependency",
        "Bundle the coordinator and its deferred framework dependencies in the same build.");
      outputs.add(edge.path);
    }
  }
  return outputs;
}

async function requireFrameworkPackage(directory: string): Promise<void> {
  let current = directory;
  while (true) {
    try {
      const owner: { name?: unknown } = JSON.parse(await readFile(path.join(current, "package.json"), "utf8"));
      if (owner.name === FRAMEWORK) return;
      break;
    } catch (error: unknown) {
      if ((error as NodeJS.ErrnoException).code !== "ENOENT") throw error;
      const parent = path.dirname(current);
      if (parent === current) break;
      current = parent;
    }
  }
  throw new StreamingBuildError("STREAM-B003", "streaming entry is not owned by the framework",
    `Resolve ${RUNTIME} to the installed framework or its actual source.`);
}

function within(root: string, file: string): boolean {
  const relative = path.relative(root, file);
  return relative !== ".." && !relative.startsWith(`..${path.sep}`) && !path.isAbsolute(relative);
}

function key(file: string): string {
  return process.platform === "win32" ? file.toLowerCase() : file;
}

function validatePublicPath(value: string): void {
  if (!value) return;
  let url: URL;
  try {
    url = new URL(value, "https://webui.invalid");
  } catch {
    throw new StreamingBuildError("STREAM-B005", "invalid publicPath",
      "Use a root-relative or HTTP(S) asset base.");
  }
  if (
    (!value.startsWith("/") && !value.startsWith("https://") && !value.startsWith("http://")) ||
    value.includes("?") || value.includes("#") || value.includes("\\") ||
    url.username || url.password || (url.protocol !== "http:" && url.protocol !== "https:")
  ) throw new StreamingBuildError("STREAM-B005", "invalid publicPath",
    "Use a root-relative or HTTP(S) asset base without credentials, query or fragment.");
}

function servedUrl(id: string, context: OutputContext): string {
  const file = path.resolve(context.working, id);
  const relative = path.relative(context.outdir, file);
  const segments = relative.split(path.sep);
  if (!within(context.outdir, file) || !relative || segments.some(segment =>
    segment.includes("%") || segment.includes("?") || segment.includes("#") || segment.includes("\\")
  )) {
    throw new StreamingBuildError("STREAM-B005", "invalid emitted asset path",
      "Keep outputs within outdir without URL delimiters in filenames.");
  }
  const suffix = segments.map(encodeURIComponent).join("/");
  return context.publicPath
    ? `${context.publicPath}${context.publicPath.endsWith("/") ? "" : "/"}${suffix}`
    : suffix;
}

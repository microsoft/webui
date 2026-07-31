// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { createRequire } from "node:module";
import { execFileSync } from "node:child_process";
import fs from "node:fs";
import nodePath from "node:path";
import { resolve, platformKey } from "./platform.js";

const require = createRequire(import.meta.url);

// ── Types ────────────────────────────────────────────────────────────

/** Options for building a WebUI application. */
export interface BuildOptions {
  /** Path to the application folder containing templates. */
  appDir: string;
  /** Entry HTML file name (default: "index.html"). */
  entry?: string;
  /** CSS delivery strategy: "link" (default), "style", or "module". */
  css?: "link" | "style" | "module";
  /** Parser plugin name. */
  plugin?: string;
  /** Additional component sources (npm packages or local paths). */
  components?: string[];
  /** Root component tags emitted as static `.webui.js` ESM assets. */
  componentAssetRoots?: string[];
  /** Emitted asset filename template for Link-mode CSS and component assets. Tokens: [name], [hash], [ext]. */
  cssFileNameTemplate?: string;
  /** Optional base URL/path prefix for Link-mode CSS hrefs. */
  cssPublicBase?: string;
  /** Output directory (used by CLI fallback for build-to-disk). */
  outDir?: string;
  /** Design token theme: a JSON file path or npm package name. */
  theme?: string;
  /** Projection manifest file paths, merged in order. */
  projectionManifests?: string[];
  /** Inline manifests with logical paths anchoring root/stale validation. */
  projectionManifestObjects?: Array<{
    path: string;
    manifest: unknown;
  }>;
}

/** Build statistics. */
export interface BuildStats {
  /** Build duration in milliseconds. */
  durationMs: number;
  /** Total number of protocol fragments. */
  fragmentCount: number;
  /** Number of registered components. */
  componentCount: number;
  /** Number of CSS files produced. */
  cssFileCount: number;
  /** Size of the serialized protocol in bytes. */
  protocolSizeBytes: number;
  /** Number of unique CSS tokens discovered. */
  tokenCount: number;
}

/** Result of a successful build operation. */
export interface BuildResult {
  /** Serialized protocol (protobuf binary). */
  protocol: Buffer;
  /** CSS files as alternating [filename, content, ...]. */
  cssFiles: string[];
  /** Static component asset files as alternating [filename, content, ...]. */
  componentAssetFiles: string[];
  /** Non-fatal build advisories as plain diagnostic strings. */
  warnings: string[];
  /** Build statistics. */
  stats: BuildStats;
}

/** Options for rendering a protocol. */
export interface RenderOptions {
  /** Fragment ID to start rendering from (default: "index.html"). */
  entry?: string;
  /** URL path to match routes against (default: "/"). */
  requestPath?: string;
}

/** Options fixed for the lifetime of a loaded protocol. */
export interface ProtocolOptions {
  /** Handler plugin name. */
  plugin?: string;
}

/**
 * Whether a committed boundary may receive later state updates.
 *
 * `final` releases every boundary-local reference once the island hydrates.
 * `updatable` retains the roots and projection so `update()` can patch them,
 * so use it only for boundaries you actually intend to patch.
 */
export type BoundaryMode = "final" | "updatable";

/** Per-response settings for a host-driven streaming session. */
export interface StreamOptions {
  /** Fragment ID to start rendering from (default: "index.html"). */
  entry?: string;
  /** URL path to match routes against (default: "/"). */
  requestPath?: string;
  /** CSP nonce applied to generated inline `<script>` tags. */
  nonce?: string;
  /** HTML injected at the structural `head_end` boundary. */
  headInject?: string;
  /** HTML injected at the structural `body_end` boundary. */
  bodyInject?: string;
}

/** Response from `renderComponentTemplates()` for on-demand component loading. */
export interface ComponentTemplatesResponse {
  /** Module CSS `<style>` strings for the requested components. */
  templateStyles: string[];
  /** JSON-safe component template metadata keyed by tag name. */
  templates: Record<string, unknown>;
  /** JavaScript condition closure arrays keyed by tag name. */
  templateFunctions: Record<string, string>;
  /** Updated hex bitmask of loaded component templates. */
  inventory: string;
}

/** Complete JSON partial response from the server for client-side navigation. */
export interface PartialResponse {
  /** Application state for the matched route. */
  state: Record<string, unknown>;
  /** JSON-safe component template metadata keyed by tag name. */
  templates: Record<string, unknown>;
  /** JavaScript condition closure arrays keyed by tag name. */
  templateFunctions?: Record<string, string>;
  /** Updated hex bitmask of loaded component templates. */
  inventory: string;
  /** The request path. */
  path: string;
  /** Matched route chain — one entry per nesting level. */
  chain: Array<{
    component: string;
    path: string;
    params?: Record<string, string>;
    exact?: boolean;
  }>;
}

// ── Internal: native addon loading ───────────────────────────────────

interface NativeAddon {
  Protocol?: new (protocol: Buffer, plugin?: string) => NativeProtocol;
  build(options: {
    appDir: string;
    entry?: string;
    css?: string;
    plugin?: string;
    components?: string[];
    componentAssetRoots?: string[];
    cssFileNameTemplate?: string;
    cssPublicBase?: string;
    projectionManifests?: string[];
    projectionManifestObjects?: Array<{
      path: string;
      json: string;
    }>;
  }): BuildResult;
  inspect(protocolData: Buffer): string;
}

interface NativeProtocol {
  render(stateJson: string, entry: string, requestPath: string): Buffer;
  renderStream(
    stateJson: string,
    entry: string,
    requestPath: string,
    onChunk: (html: string) => void,
  ): void;
  streamResponse(
    entry: string,
    requestPath: string,
    options?: {
      nonce?: string;
      headInject?: string;
      bodyInject?: string;
    },
  ): NativeStreamingSession;
  renderPartial(stateJson: string, entryId: string, requestPath: string, inventoryHex: string): string;
  renderComponentTemplates(componentTags: string[], inventoryHex: string): string;
  tokens(): string[];
}

interface NativeStreamingSession {
  readonly boundaryCount: number;
  readonly finished: boolean;
  boundary(name: string): number;
  writeShell(stateJson: string): Buffer;
  writeBoundary(boundary: number, stateJson: string, mode?: BoundaryMode): Buffer;
  update(boundary: number, stateJson: string): Buffer;
  finish(stateJson: string): Buffer;
}

let addon: NativeAddon | null = null;
let fallbackWarned = false;

function loadAddon(): NativeAddon | null {
  if (addon) return addon;

  const addonPath = resolve("addon");
  if (addonPath) {
    try {
      // .node files load via require(), native libs (.dylib/.so/.dll) via dlopen
      if (addonPath.endsWith(".node")) {
        addon = require(addonPath) as NativeAddon;
      } else {
        const m: { exports: NativeAddon } = { exports: {} as NativeAddon };
        process.dlopen(m, addonPath);
        addon = m.exports;
      }
      return addon;
    } catch {
      // Fall through to WASM.
    }
  }
  return null;
}

function warnFallback(): void {
  if (fallbackWarned) return;
  fallbackWarned = true;
  console.warn(
    `[webui] Native addon not available for ${platformKey()}. ` +
      `Using WASM fallback — performance may be degraded.\n` +
      `Install the platform-specific package for optimal performance.`,
  );
}

// ── Build API ────────────────────────────────────────────────────────

/** Build a WebUI application from an app directory. */
export function build(options: BuildOptions): BuildResult {
  const native = loadAddon();
  if (native?.build) {
    const { projectionManifestObjects, ...nativeOptions } = options;
    return native.build({
      ...nativeOptions,
      projectionManifestObjects: projectionManifestObjects?.map(
        ({ path, manifest }) => ({
          path,
          json: JSON.stringify(manifest),
        })
      ),
    });
  }

  // Fallback: shell out to CLI binary.
  const binPath = resolve("bin");
  if (!binPath) {
    throw new Error(
      "[webui] Cannot build: no native addon or CLI binary available.",
    );
  }

  const args = ["build", options.appDir ?? "."];
  if (options.entry) args.push("--entry", options.entry);
  if (options.css) args.push("--css", options.css);
  if (options.plugin) args.push("--plugin", options.plugin);
  if (options.components) {
    for (const c of options.components) {
      args.push("--components", c);
    }
  }
  if (options.projectionManifests) {
    for (const manifest of options.projectionManifests) {
      args.push("--projection-manifest", manifest);
    }
  }
  if (
    options.projectionManifestObjects &&
    options.projectionManifestObjects.length > 0
  ) {
    throw new Error(
      "[webui] Inline projection manifest objects require the native addon; write the manifest and pass projectionManifests when using the CLI fallback."
    );
  }
  if (options.componentAssetRoots && options.componentAssetRoots.length > 0) {
    args.push("--emit-component-assets", options.componentAssetRoots.join(","));
  }
  if (options.cssFileNameTemplate) {
    args.push("--css-file-name-template", options.cssFileNameTemplate);
  }
  if (options.cssPublicBase) {
    args.push("--css-public-base", options.cssPublicBase);
  }
  if (options.theme) args.push("--theme", options.theme);
  if (options.outDir) args.push("--out", options.outDir);

  execFileSync(binPath, args, { stdio: "inherit" });

  // CLI fallback does not return in-memory protocol.
  if (options.outDir) {
    const protocol = fs.readFileSync(nodePath.join(options.outDir, "protocol.bin"));
    return {
      protocol,
      cssFiles: [],
      componentAssetFiles: readComponentAssetFiles(options.outDir),
      warnings: [],
      stats: emptyStats(),
    };
  }

  return {
    protocol: Buffer.alloc(0),
    cssFiles: [],
    componentAssetFiles: [],
    warnings: [],
    stats: emptyStats(),
  };
}

function readComponentAssetFiles(outDir: string): string[] {
  const files: string[] = [];
  const entries = fs.readdirSync(outDir, { withFileTypes: true });
  for (let i = 0; i < entries.length; i++) {
    const entry = entries[i];
    if (!entry.isFile()) continue;
    const name = entry.name;
    const path = nodePath.join(outDir, name);
    const content = fs.readFileSync(path, "utf8");
    if (!content.startsWith("const asset=") || !content.includes("webui-component-asset")) continue;
    files.push(name, content);
  }
  return files;
}

// ── Runtime protocol API ─────────────────────────────────────────────

/**
 * A decoded protocol with reusable indices for all runtime operations.
 *
 * Create one instance when the server loads `protocol.bin` and share it
 * across requests. Construction decodes and indexes the protocol once.
 */
export class Protocol {
  readonly #native: NativeProtocol;

  constructor(protocolData: Buffer, options?: ProtocolOptions) {
    const native = loadAddon();
    const NativeProtocol = native?.Protocol;
    if (!NativeProtocol) {
      warnFallback();
      throw new Error(
        "[webui] Native addon is incompatible: Protocol is required.",
      );
    }
    this.#native = new NativeProtocol(protocolData, options?.plugin);
  }

  /** Render a complete HTML response as a UTF-8 Node.js buffer. */
  render(state: object | string, options?: RenderOptions): Buffer {
    const stateJson = typeof state === "string" ? state : JSON.stringify(state);
    return this.#native.render(
      stateJson,
      options?.entry ?? "index.html",
      options?.requestPath ?? "/",
    );
  }

  /** Stream a complete HTML response in chunks around 16 KiB. */
  renderStream(
    state: object | string,
    onChunk: (html: string) => void,
    options?: RenderOptions,
  ): void {
    const stateJson = typeof state === "string" ? state : JSON.stringify(state);
    this.#native.renderStream(
      stateJson,
      options?.entry ?? "index.html",
      options?.requestPath ?? "/",
      onChunk,
    );
  }

  /** Produce a complete JSON partial-navigation response. */
  renderPartial(
    state: object | string,
    entryId: string,
    requestPath: string,
    inventoryHex: string,
  ): string {
    const stateJson = typeof state === "string" ? state : JSON.stringify(state);
    return this.#native.renderPartial(stateJson, entryId, requestPath, inventoryHex);
  }

  /** Render component templates and styles for on-demand loading. */
  renderComponentTemplates(
    componentTags: string[],
    inventoryHex: string,
  ): string {
    return this.#native.renderComponentTemplates(componentTags, inventoryHex);
  }

  /** Return CSS token names in build order. */
  tokens(): string[] {
    return this.#native.tokens();
  }

  /**
   * Open a host-driven progressive response.
   *
   * Unlike {@link renderStream}, which pushes every chunk during one
   * synchronous call, the returned session hands each chunk back so this
   * server owns the socket, the write order, and backpressure.
   */
  streamResponse(options?: StreamOptions): StreamingSession {
    return new StreamingSession(
      this.#native.streamResponse(
        options?.entry ?? "index.html",
        options?.requestPath ?? "/",
        {
          nonce: options?.nonce,
          headInject: options?.headInject,
          bodyInject: options?.bodyInject,
        },
      ),
    );
  }
}

/**
 * A progressive HTML response written one chunk at a time.
 *
 * Every method returns the bytes it produced instead of writing them, so the
 * caller decides when they reach the socket and can await `drain` between
 * chunks. The session holds no transport and never blocks on one.
 *
 * Ordering is enforced: the shell first, then each boundary exactly once in
 * declaration order, `update()` only after its boundary commits as
 * `updatable`, and `finish()` last. A violation throws before any byte is
 * produced.
 *
 * ```js
 * const session = protocol.streamResponse({ requestPath: req.url });
 * const weather = session.boundary("weather-shell");
 *
 * res.write(session.writeShell(shellState));
 * res.write(session.writeBoundary(weather, weatherShell, "updatable"));
 *
 * const forecast = await forecastReady;
 * if (!res.write(session.update(weather, forecast))) {
 *   await once(res, "drain");
 * }
 *
 * res.end(session.finish({}));
 * ```
 */
export class StreamingSession {
  readonly #native: NativeStreamingSession;

  /** @internal Created by {@link Protocol.streamResponse}. */
  constructor(native: NativeStreamingSession) {
    this.#native = native;
  }

  /** Number of compile-time boundaries declared by this entry. */
  get boundaryCount(): number {
    return this.#native.boundaryCount;
  }

  /** Whether the terminal record has been written. */
  get finished(): boolean {
    return this.#native.finished;
  }

  /**
   * Resolve an authored boundary name to a stable integer handle.
   *
   * Resolve once outside the write loop; reusing the handle costs nothing.
   * An unknown name throws with the valid names and a suggestion.
   */
  boundary(name: string): number {
    return this.#native.boundary(name);
  }

  /** Render everything before the first boundary. */
  writeShell(state: object | string): Buffer {
    return this.#native.writeShell(toStateJson(state));
  }

  /** Render and commit the next boundary in declaration order. */
  writeBoundary(
    boundary: number,
    state: object | string,
    mode: BoundaryMode = "final",
  ): Buffer {
    return this.#native.writeBoundary(boundary, toStateJson(state), mode);
  }

  /** Push a projected state patch to a committed `updatable` boundary. */
  update(boundary: number, state: object | string): Buffer {
    return this.#native.update(boundary, toStateJson(state));
  }

  /** Render the document tail and emit the terminal record. */
  finish(state: object | string = {}): Buffer {
    return this.#native.finish(toStateJson(state));
  }
}

function toStateJson(state: object | string): string {
  return typeof state === "string" ? state : JSON.stringify(state);
}

/** Inspect protocol bytes and return JSON representation. */
export function inspect(protocolData: Buffer): string {
  const native = loadAddon();
  if (native?.inspect) {
    return native.inspect(protocolData);
  }
  throw new Error("[webui] inspect() requires the native addon.");
}

// ── Helpers ──────────────────────────────────────────────────────────

function emptyStats(): BuildStats {
  return {
    durationMs: 0,
    fragmentCount: 0,
    componentCount: 0,
    cssFileCount: 0,
    protocolSizeBytes: 0,
    tokenCount: 0,
  };
}

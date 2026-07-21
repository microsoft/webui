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
  render(stateJson: string, entry: string, requestPath: string): string;
  renderBuffer?(stateJson: string, entry: string, requestPath: string): Buffer;
  renderStream(
    stateJson: string,
    entry: string,
    requestPath: string,
    onChunk: (html: string) => void,
  ): void;
  renderPartial(stateJson: string, entryId: string, requestPath: string, inventoryHex: string): string;
  renderComponentTemplates(componentTags: string[], inventoryHex: string): string;
  tokens(): string[];
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

  /** Render a complete HTML response. */
  render(state: object | string, options?: RenderOptions): string {
    const stateJson = serializeState(state);
    return this.#native.render(
      stateJson,
      options?.entry ?? "index.html",
      options?.requestPath ?? "/",
    );
  }

  /** Render a complete HTML response as a UTF-8 Node.js buffer. */
  renderBuffer(state: object | string, options?: RenderOptions): Buffer {
    if (!this.#native.renderBuffer) {
      throw new Error(
        "[webui] Native addon is incompatible: Protocol.renderBuffer is required.",
      );
    }
    const stateJson = serializeState(state);
    return this.#native.renderBuffer(
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
    const stateJson = serializeState(state);
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
    const stateJson = serializeState(state);
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

function serializeState(state: object | string): string {
  return typeof state === "string" ? state : JSON.stringify(state);
}

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

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
  /** Parser plugin (e.g., "fast" for FAST-HTML hydration). */
  plugin?: string;
  /** Additional component sources (npm packages or local paths). */
  components?: string[];
  /** Output directory (used by CLI fallback for build-to-disk). */
  outDir?: string;
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
  /** Build statistics. */
  stats: BuildStats;
}

/** Options for rendering a protocol. */
export interface RenderOptions {
  /** Fragment ID to start rendering from (default: "index.html"). */
  entry?: string;
  /** URL path to match routes against (default: "/"). */
  requestPath?: string;
  /** Parser plugin name (e.g., "fast" for FAST-HTML hydration). */
  plugin?: string;
}

/** Complete JSON partial response from the server for client-side navigation. */
export interface PartialResponse {
  /** Application state for the matched route. */
  state: Record<string, unknown>;
  /** f-template HTML strings the client doesn't already have. */
  templates: string[];
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
  render(
    protocol: Buffer,
    stateJson: string,
    entry: string,
    requestPath: string,
    onChunk: (html: string) => void,
    plugin?: string,
  ): void;
  build(options: {
    appDir: string;
    entry?: string;
    css?: string;
    plugin?: string;
    components?: string[];
  }): BuildResult;
  inspect(protocolData: Buffer): string;
  renderPartial(protocolData: Buffer, stateJson: string, entryId: string, requestPath: string, inventoryHex: string): string;
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
    return native.build(options);
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
  if (options.outDir) args.push("--out", options.outDir);

  execFileSync(binPath, args, { stdio: "inherit" });

  // CLI fallback does not return in-memory protocol.
  if (options.outDir) {
    const protocol = fs.readFileSync(nodePath.join(options.outDir, "protocol.bin"));
    return { protocol, cssFiles: [], stats: emptyStats() };
  }

  return { protocol: Buffer.alloc(0), cssFiles: [], stats: emptyStats() };
}

// ── Render API ───────────────────────────────────────────────────────

/**
 * Render a pre-compiled protocol with state data.
 * Uses native addon when available, WASM fallback otherwise.
 */
export function render(
  protocol: Buffer,
  state: object | string,
  options?: RenderOptions,
): string {
  const native = loadAddon();
  if (native) {
    let result = "";
    const stateStr = typeof state === "string" ? state : JSON.stringify(state);
    const entry = options?.entry ?? "index.html";
    const requestPath = options?.requestPath ?? "/";
    native.render(protocol, stateStr, entry, requestPath, (chunk) => {
      result += chunk;
    }, options?.plugin);
    return result;
  }

  warnFallback();
  throw new Error(
    "[webui] render() requires the native addon. WASM render fallback not yet wired.",
  );
}

/**
 * Render a protocol with streaming output.
 * Each HTML fragment is passed to the onChunk callback as it is produced.
 */
export function renderStream(
  protocol: Buffer,
  state: object | string,
  onChunk: (html: string) => void,
  options?: RenderOptions,
): void {
  const native = loadAddon();
  if (native) {
    const stateStr = typeof state === "string" ? state : JSON.stringify(state);
    const entry = options?.entry ?? "index.html";
    const requestPath = options?.requestPath ?? "/";
    native.render(protocol, stateStr, entry, requestPath, onChunk, options?.plugin);
    return;
  }

  warnFallback();
  throw new Error(
    "[webui] renderStream() requires the native addon. WASM render fallback not yet wired.",
  );
}

// ── Convenience ──────────────────────────────────────────────────────

/** Build and render in a single call. */
export function buildAndRender(
  options: BuildOptions,
  state: object | string,
  renderOpts?: RenderOptions,
): string {
  const result = build(options);
  if (!result.protocol || result.protocol.length === 0) {
    throw new Error("[webui] Build did not return protocol data.");
  }
  return render(result.protocol, state, renderOpts);
}

/** Inspect protocol bytes and return JSON representation. */
export function inspect(protocolData: Buffer): string {
  const native = loadAddon();
  if (native?.inspect) {
    return native.inspect(protocolData);
  }
  throw new Error("[webui] inspect() requires the native addon.");
}

/**
 * Produce a complete JSON partial response for client-side navigation.
 *
 * Returns a JSON string with `state`, `templates`, `inventory`, `path`, and `chain`.
 * Pipe directly to the HTTP response — no post-processing needed.
 *
 * If you need to inspect the response, parse it with the exported `PartialResponse` type:
 * ```ts
 * const partial: PartialResponse = JSON.parse(renderPartial(...));
 * ```
 */
export function renderPartial(
  protocolData: Buffer,
  stateJson: string,
  entryId: string,
  requestPath: string,
  inventoryHex: string,
): string {
  const native = loadAddon();
  if (native?.renderPartial) {
    return native.renderPartial(protocolData, stateJson, entryId, requestPath, inventoryHex);
  }
  throw new Error("[webui] renderPartial() requires the native addon.");
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

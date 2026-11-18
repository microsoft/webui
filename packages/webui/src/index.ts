// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { addonInstallHelp, loadBinding } from "./binding.js";

// ── Types ────────────────────────────────────────────────────────────

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

export type ComponentStyleResource = (
  | { kind: "link"; href: string }
  | { kind: "style"; css: string }
  | { kind: "module"; specifier: string; css: string }
) & {
  /** Component resource IDs whose rules this bundled resource covers. */
  members?: string[];
};

export interface ComponentStyles {
  version: 1;
  strategy: "link" | "style" | "module";
  resources: Record<string, ComponentStyleResource>;
  closures: Record<string, string[]>;
}

/** A runtime-discovered boundary occurrence waiting to be resumed. */
export interface BoundaryDescriptor {
  /** Gapless response-local occurrence ID passed to `resume()` and `update()`. */
  instanceId: number;
  /** Stable build-local ID for the authored boundary declaration. */
  declarationId: number;
  /** Entry or component template that owns the declaration. */
  owner: string;
  /** Free-form authored boundary name. */
  name: string;
  /** Evaluated boundary key, preserving its authored JSON type. */
  key?: string | number;
}

/** Bytes and continuation state produced by a streaming session step. */
export interface StreamStep {
  /** Complete bytes produced by this semantic step. */
  bytes: Buffer;
  /** Whether the document tail and terminal record have been emitted. */
  done: boolean;
  /** Runtime occurrence waiting for `resume()`, present only at a boundary. */
  boundary?: BoundaryDescriptor;
}

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
  buildId?: string;
  styleInventory?: string;
  /** Versioned component style definitions and ordered closures. */
  componentStyles: ComponentStyles;
  /** JSON-safe component template metadata keyed by tag name. */
  templates: Record<string, unknown>;
  /** JavaScript condition closure arrays keyed by tag name. */
  templateFunctions: Record<string, string>;
  /** Updated hex bitmask of loaded component templates. */
  inventory: string;
}

/** Complete JSON partial response from the server for client-side navigation. */
export interface PartialResponse {
  buildId?: string;
  styleInventory?: string;
  /** Application state for the matched route. */
  state: Record<string, unknown>;
  /** JSON-safe component template metadata keyed by tag name. */
  templates: Record<string, unknown>;
  /** JavaScript condition closure arrays keyed by tag name. */
  templateFunctions?: Record<string, string>;
  /** Versioned component style definitions and ordered closures. */
  componentStyles: ComponentStyles;
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
  inspect(protocolData: Buffer): string;
}

interface NativeProtocol {
  render(stateJson: string, entry: string, requestPath: string): Buffer;
  prepareState(stateJson: string): PreparedState;
  renderPrepared(state: PreparedState, entry: string, requestPath: string): Buffer;
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

declare const preparedStateBrand: unique symbol;

/**
 * An immutable, process-local native state snapshot.
 *
 * The snapshot retains its parsed native state until this handle is garbage
 * collected. Create snapshots only for state that will be rendered repeatedly.
 */
export interface PreparedState {
  readonly [preparedStateBrand]: never;
}

interface NativeStreamingSession {
  start(stateJson: string): NativeStreamStep;
  resume(instanceId: number, stateJson: string, mode?: BoundaryMode): NativeStreamStep;
  advance(): NativeStreamStep;
  update(instanceId: number, patchJson: string): Buffer;
}

interface NativeBoundaryDescriptor {
  instanceId: number;
  declarationId: number;
  owner: string;
  name: string;
  key?: string | number | null;
}

interface NativeStreamStep {
  bytes: Buffer;
  done: boolean;
  boundary?: NativeBoundaryDescriptor | null;
}

function loadAddon(): NativeAddon {
  return loadBinding<NativeAddon>();
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
    const NativeProtocol = loadAddon().Protocol;
    if (!NativeProtocol) {
      throw new Error(
        `[webui] Native addon is incompatible: Protocol is required. ${addonInstallHelp()}`,
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

  /**
   * Parse state once for repeated rendering.
   *
   * The returned snapshot is immutable and does not observe later mutations to
   * the source object. Prepare a new snapshot when request state changes.
   */
  prepareState(state: object | string): PreparedState {
    return this.#native.prepareState(
      typeof state === "string" ? state : JSON.stringify(state),
    );
  }

  /** Render a prepared state snapshot without serializing or parsing it again. */
  renderPrepared(state: PreparedState, options?: RenderOptions): Buffer {
    return this.#native.renderPrepared(
      state,
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
 * `start()` discovers the first runtime occurrence. Each `resume()` commits
 * only the pending occurrence. An optional `update()` may follow for an
 * `updatable` occurrence, then `advance()` discovers the next occurrence or
 * returns the document tail and terminal record.
 *
 * ```js
 * const session = protocol.streamResponse({ requestPath: req.url });
 * let step = session.start(shellState);
 * res.write(step.bytes);
 *
 * while (!step.done) {
 *   const boundary = step.boundary;
 *   const state = await loadBoundary(boundary.owner, boundary.name, boundary.key);
 *   step = session.resume(boundary.instanceId, state);
 *   res.write(step.bytes);
 *   step = session.advance();
 *   res.write(step.bytes);
 * }
 * res.end();
 * ```
 */
export class StreamingSession {
  readonly #native: NativeStreamingSession;

  /** @internal Created by {@link Protocol.streamResponse}. */
  constructor(native: NativeStreamingSession) {
    this.#native = native;
  }

  /** Render until the first runtime boundary occurrence or terminal. */
  start(state: object | string): StreamStep {
    return toStreamStep(this.#native.start(toStateJson(state)));
  }

  /** Commit the pending occurrence through its checkpoint, then stop. */
  resume(
    instanceId: number,
    state: object | string,
    mode: BoundaryMode = "final",
  ): StreamStep {
    return toStreamStep(this.#native.resume(instanceId, toStateJson(state), mode));
  }

  /** Write following parent bytes and discover the next boundary or terminal. */
  advance(): StreamStep {
    return toStreamStep(this.#native.advance());
  }

  /** Push a projected state patch to a committed `updatable` occurrence. */
  update(instanceId: number, patch: object | string): Buffer {
    return this.#native.update(instanceId, toStateJson(patch));
  }
}

function toStreamStep(step: NativeStreamStep): StreamStep {
  const boundary = step.boundary;
  if (!boundary) {
    return { bytes: step.bytes, done: step.done };
  }
  return {
    bytes: step.bytes,
    done: step.done,
    boundary: toBoundaryDescriptor(boundary),
  };
}

function toBoundaryDescriptor(
  boundary: NativeBoundaryDescriptor,
): BoundaryDescriptor {
  const descriptor: BoundaryDescriptor = {
    instanceId: boundary.instanceId,
    declarationId: boundary.declarationId,
    owner: boundary.owner,
    name: boundary.name,
  };
  if (boundary.key !== undefined && boundary.key !== null) {
    descriptor.key = boundary.key;
  }
  return descriptor;
}

function toStateJson(state: object | string): string {
  return typeof state === "string" ? state : JSON.stringify(state);
}

/** Inspect protocol bytes and return JSON representation. */
export function inspect(protocolData: Buffer): string {
  const native = loadAddon();
  if (typeof native.inspect !== "function") {
    throw new Error(
      `[webui] Native addon is incompatible: inspect() is required. ${addonInstallHelp()}`,
    );
  }
  return native.inspect(protocolData);
}

// ── Helpers ──────────────────────────────────────────────────────────

/* tslint:disable */
/* eslint-disable */

/**
 * A decoded protocol with reusable indices for repeated WASM renders.
 */
export class Protocol {
    free(): void;
    [Symbol.dispose](): void;
    /**
     * Decode protobuf bytes once for repeated rendering.
     */
    constructor(protocol_bytes: Uint8Array, plugin?: string | null);
    /**
     * Render from an existing JSON string.
     */
    render(state_json: string, options?: object | null): string;
    /**
     * Return component template payloads for requested component tags.
     */
    renderComponentTemplates(component_tags: any, inventory_hex: string): string;
    /**
     * Produce a complete partial-navigation response.
     */
    renderPartial(state_json: string, entry_id: string, request_path: string, inventory_hex: string): string;
    /**
     * Stream from an existing JSON string in bounded chunks.
     */
    renderStream(state_json: string, on_chunk: Function, options?: object | null): void;
    /**
     * Open a host-driven progressive response for a streaming entry.
     *
     * Unlike `renderStream`, which pushes every chunk through one callback
     * during a single synchronous call, the returned session hands each chunk
     * back so the host owns the socket, the write order, and backpressure.
     */
    streamResponse(entry: string, request_path: string, options?: object | null): StreamingSession;
    /**
     * Return CSS token names in build order.
     */
    tokens(): any;
}

/**
 * A progressive HTML response driven one semantic step at a time from JavaScript.
 *
 * `start()`, `resume()`, and `advance()` return
 * `{ bytes, done, boundary? }`, where `bytes` is a `Uint8Array` and a boundary is
 * `{ instanceId, declarationId, owner, name, key }`. Boundary keys retain
 * their authored JSON type: strings are JavaScript strings and finite numbers
 * are JavaScript numbers.
 *
 * ```js
 * const session = protocol.streamResponse('index.html', '/');
 * let step = session.start(JSON.stringify(shellState));
 * controller.enqueue(step.bytes);
 * while (!step.done) {
 *   const { instanceId, name, key } = step.boundary;
 *   const state = await loadBoundary(name, key);
 *   step = session.resume(instanceId, JSON.stringify(state), 'updatable');
 *   controller.enqueue(step.bytes);
 *   step = session.advance();
 *   controller.enqueue(step.bytes);
 * }
 * ```
 */
export class StreamingSession {
    private constructor();
    free(): void;
    [Symbol.dispose](): void;
    /**
     * Write the parent bytes after the committed occurrence.
     *
     * Valid only after `resume()`. Returns the next boundary occurrence or
     * completes the document tail.
     */
    advance(): object;
    /**
     * Commit the pending occurrence through its checkpoint, then stop.
     *
     * `mode` is `"final"` (default) or `"updatable"`. Only updatable
     * boundaries accept later `update()` calls.
     */
    resume(instance_id: number, state_json: string, mode?: string | null): object;
    /**
     * Render until the first runtime boundary occurrence or terminal.
     */
    start(state_json: string): object;
    /**
     * Push a projected state patch to a committed updatable boundary.
     */
    update(instance_id: number, patch_json: string): Uint8Array;
}

/**
 * Build protocol bytes, lazy-region template packs, and `lazy.webui.js`.
 *
 * `options` is an optional object with `projectionManifests`, `dom`
 * (`"shadow"` or `"light"`), `assetBase` (a public URL or path ending in `/`
 * that maps to the output directory; requests override it with
 * `$webui.assetBase`), `componentModules` (module specifiers keyed by custom
 * element tag), and `modulesOut` (the virtual directory that will receive
 * `lazy.webui.js`; relative specifiers start there, at the file map root by
 * default).
 *
 * Returns `{ protocol, lazyPackFiles, lazyScriptsModule?, warnings }`.
 * `protocol` is a `Uint8Array`. Each `lazyPackFiles` item has `fileName`,
 * `content`, `path`, `region`, `owner`, `trigger`, `hash`, `bytes`,
 * `scripts`, and `stylesheets`; publish its content at `path` in the output
 * directory. WASM builds inline component CSS, so `stylesheets` is always
 * empty, and they do not measure brotli sizes. `lazyScriptsModule` is
 * `{ fileName, content, declaration: { fileName, content } }`, the generated
 * `lazy.webui.js` and its `lazy.webui.d.ts`, when the build has lazy regions.
 */
export function buildApp(files: any, entry: string, options?: any | null): any;

/**
 * Build protocol protobuf bytes from virtual files without rendering.
 *
 * `dom` accepts `"shadow"` (default) or `"light"` for unwrapped components;
 * authored open Shadow roots remain Shadow in either mode.
 *
 * Returns the serialized `WebUIProtocol` as protobuf bytes. Templates with
 * lazy regions also need their template packs; use `buildApp` from the
 * combined WASM build, which returns both.
 */
export function build_protocol(files: any, entry: string, projection_manifests?: any | null, dom?: string | null): Uint8Array;

export type InitInput = RequestInfo | URL | Response | BufferSource | WebAssembly.Module;

export interface InitOutput {
    readonly memory: WebAssembly.Memory;
    readonly __wbg_protocol_free: (a: number, b: number) => void;
    readonly __wbg_streamingsession_free: (a: number, b: number) => void;
    readonly buildApp: (a: number, b: number, c: number, d: number, e: number) => void;
    readonly build_protocol: (a: number, b: number, c: number, d: number, e: number, f: number, g: number) => void;
    readonly protocol_new: (a: number, b: number, c: number, d: number, e: number) => void;
    readonly protocol_render: (a: number, b: number, c: number, d: number, e: number) => void;
    readonly protocol_renderComponentTemplates: (a: number, b: number, c: number, d: number, e: number) => void;
    readonly protocol_renderPartial: (a: number, b: number, c: number, d: number, e: number, f: number, g: number, h: number, i: number, j: number) => void;
    readonly protocol_renderStream: (a: number, b: number, c: number, d: number, e: number, f: number) => void;
    readonly protocol_streamResponse: (a: number, b: number, c: number, d: number, e: number, f: number, g: number) => void;
    readonly protocol_tokens: (a: number, b: number) => void;
    readonly streamingsession_advance: (a: number, b: number) => void;
    readonly streamingsession_resume: (a: number, b: number, c: number, d: number, e: number, f: number, g: number) => void;
    readonly streamingsession_start: (a: number, b: number, c: number, d: number) => void;
    readonly streamingsession_update: (a: number, b: number, c: number, d: number, e: number) => void;
    readonly __wbindgen_export: (a: number, b: number) => number;
    readonly __wbindgen_export2: (a: number, b: number, c: number, d: number) => number;
    readonly __wbindgen_export3: (a: number) => void;
    readonly __wbindgen_add_to_stack_pointer: (a: number) => number;
    readonly __wbindgen_export4: (a: number, b: number, c: number) => void;
}

export type SyncInitInput = BufferSource | WebAssembly.Module;

/**
 * Instantiates the given `module`, which can either be bytes or
 * a precompiled `WebAssembly.Module`.
 *
 * @param {{ module: SyncInitInput }} module - Passing `SyncInitInput` directly is deprecated.
 *
 * @returns {InitOutput}
 */
export function initSync(module: { module: SyncInitInput } | SyncInitInput): InitOutput;

/**
 * If `module_or_path` is {RequestInfo} or {URL}, makes a request and
 * for everything else, calls `WebAssembly.instantiate` directly.
 *
 * @param {{ module_or_path: InitInput | Promise<InitInput> }} module_or_path - Passing `InitInput` directly is deprecated.
 *
 * @returns {Promise<InitOutput>}
 */
export default function __wbg_init (module_or_path?: { module_or_path: InitInput | Promise<InitInput> } | InitInput | Promise<InitInput>): Promise<InitOutput>;

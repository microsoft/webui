/* tslint:disable */
/* eslint-disable */
/**
 * Extract the CSS token name list from a protocol JSON string.
 *
 * Returns a JavaScript array of token name strings, preserving the original
 * order from the build step.
 */
export function protocol_tokens(protocol_json: string): any;
/**
 * Produce a complete JSON partial response for client-side navigation.
 *
 * Combines application state, route templates, inventory, request path, and
 * matched route chain into a single JSON string:
 * `{"state":{...},"templates":[...],"inventory":"...","path":"...","chain":[...]}`.
 *
 * Host servers return this directly — no assembly required.
 */
export function render_partial(protocol_json: string, state_json: string, entry_id: string, request_path: string, inventory_hex: string): string;
/**
 * Build the protocol JSON from virtual files without rendering.
 *
 * Returns the serialized `WebUIProtocol` as a JSON string.
 */
export function build_protocol(files: any, entry: string): string;
/**
 * Build and render a WebUI application from virtual files.
 *
 * Uses a lightweight pure-Rust parser suitable for the playground.
 * Handles signals, for-loops, if-conditions, components, and dynamic attributes.
 *
 * # Arguments
 *
 * * `files` — A JS object mapping filenames to their string content.
 *   Example: `{ "index.html": "<h1>{{title}}</h1>", "my-card.html": "<p><slot></slot></p>" }`
 * * `state_json` — A JSON string of the state data to render with.
 * * `entry` — The entry HTML filename (e.g. `"index.html"`).
 *
 * # Returns
 *
 * The rendered HTML string, or throws a JS error on failure.
 */
export function build_and_render(files: any, state_json: string, entry: string, request_path: string): string;
export function render_component_templates(protocol_json: string, component_tags_json: string, inventory_hex: string): string;
/**
 * Render a pre-built WebUI protocol with state data.
 *
 * # Arguments
 *
 * * `protocol_json` — JSON string of the serialized `WebUIProtocol`.
 * * `state_json` — JSON string of the state data.
 * * `plugin` — Optional plugin identifier (see crate documentation for available identifiers).
 *
 * # Returns
 *
 * The rendered HTML string, or throws a JS error on failure.
 */
export function render(protocol_json: string, state_json: string, entry: string, request_path: string, plugin?: string | null): string;

export type InitInput = RequestInfo | URL | Response | BufferSource | WebAssembly.Module;

export interface InitOutput {
  readonly memory: WebAssembly.Memory;
  readonly build_and_render: (a: number, b: number, c: number, d: number, e: number, f: number, g: number, h: number) => void;
  readonly build_protocol: (a: number, b: number, c: number, d: number) => void;
  readonly protocol_tokens: (a: number, b: number, c: number) => void;
  readonly render: (a: number, b: number, c: number, d: number, e: number, f: number, g: number, h: number, i: number, j: number, k: number) => void;
  readonly render_component_templates: (a: number, b: number, c: number, d: number, e: number, f: number, g: number) => void;
  readonly render_partial: (a: number, b: number, c: number, d: number, e: number, f: number, g: number, h: number, i: number, j: number, k: number) => void;
  readonly __wbindgen_export_0: (a: number, b: number) => number;
  readonly __wbindgen_export_1: (a: number, b: number, c: number, d: number) => number;
  readonly __wbindgen_export_2: (a: number) => void;
  readonly __wbindgen_add_to_stack_pointer: (a: number) => number;
  readonly __wbindgen_export_3: (a: number, b: number, c: number) => void;
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

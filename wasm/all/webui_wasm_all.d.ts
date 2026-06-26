/* tslint:disable */
/* eslint-disable */
/**
 * Build protocol protobuf bytes from virtual files without rendering.
 *
 * Returns the serialized `WebUIProtocol` as protobuf bytes.
 */
export function build_protocol(files: any, entry: string): Uint8Array;
/**
 * Return component template payloads for requested component tags.
 */
export function render_component_templates(protocol_bytes: Uint8Array, component_tags_json: string, inventory_hex: string): string;
/**
 * Produce a complete JSON partial response for client-side navigation.
 *
 * Combines application state, route templates, inventory, request path, and
 * matched route chain into a single JSON string:
 * `{"state":{...},"templates":[...],"inventory":"...","path":"...","chain":[...]}`.
 *
 * Host servers return this directly - no assembly required.
 */
export function render_partial(protocol_bytes: Uint8Array, state_json: string, entry_id: string, request_path: string, inventory_hex: string): string;
/**
 * Render a pre-built WebUI protocol with state data, streaming chunks to a callback.
 *
 * # Arguments
 *
 * * `protocol_bytes` - Protobuf bytes of the serialized `WebUIProtocol`.
 * * `state_json` - JSON string of the state data.
 * * `on_chunk` - Callback invoked with each rendered HTML fragment.
 * * `options` - Optional object with `entry`, `requestPath`, and `plugin` fields.
 *
 * # Returns
 *
 * Nothing on success, or throws a JS error on failure.
 */
export function render(protocol_bytes: Uint8Array, state_json: string, on_chunk: Function, options?: object | null): void;
/**
 * Extract the CSS token name list from protocol protobuf bytes.
 *
 * Returns a JavaScript array of token name strings, preserving the original
 * order from the build step.
 */
export function protocol_tokens(protocol_bytes: Uint8Array): any;

export type InitInput = RequestInfo | URL | Response | BufferSource | WebAssembly.Module;

export interface InitOutput {
  readonly memory: WebAssembly.Memory;
  readonly build_protocol: (a: number, b: number, c: number, d: number) => void;
  readonly protocol_tokens: (a: number, b: number, c: number) => void;
  readonly render: (a: number, b: number, c: number, d: number, e: number, f: number, g: number) => void;
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

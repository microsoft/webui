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
     * Return CSS token names in build order.
     */
    tokens(): any;
}

export type InitInput = RequestInfo | URL | Response | BufferSource | WebAssembly.Module;

export interface InitOutput {
    readonly memory: WebAssembly.Memory;
    readonly __wbg_protocol_free: (a: number, b: number) => void;
    readonly protocol_new: (a: number, b: number, c: number, d: number, e: number) => void;
    readonly protocol_render: (a: number, b: number, c: number, d: number, e: number) => void;
    readonly protocol_renderComponentTemplates: (a: number, b: number, c: number, d: number, e: number) => void;
    readonly protocol_renderPartial: (a: number, b: number, c: number, d: number, e: number, f: number, g: number, h: number, i: number, j: number) => void;
    readonly protocol_renderStream: (a: number, b: number, c: number, d: number, e: number, f: number) => void;
    readonly protocol_tokens: (a: number, b: number) => void;
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

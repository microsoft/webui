// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

/**
 * Template type definitions — shared across packages without pulling in
 * the global `Window` augmentation from `template.ts`.
 *
 * These tuple shapes mirror the compact payload emitted by the Rust parser.
 * Keep them allocation-light and stable: the browser runtime indexes directly
 * into these arrays on hot hydration/update paths.
 */

export type CompiledAttrPart = string | [path: string];

/**
 * Pre-order index of an element within one compiled section's static HTML.
 *
 * `0` is the section root; elements are numbered `1..N` in the order a
 * depth-first walk of `h` meets them.  Each section — the root template and
 * every `<if>` / `<for>` block — numbers independently, and hydration rebuilds
 * the same numbering from the server-rendered DOM, so a binding resolves by
 * array index instead of by walking a path of child offsets.
 */
export type TemplateNodeIndex = number;
export type TemplateSlot = [
  parentIndex: TemplateNodeIndex,
  beforeIndex: number,
  order?: number,
];
export type CompiledTextRunMeta = [slot: TemplateSlot, parts: CompiledAttrPart[], raw?: 1];
export type CompiledAttrGroupMeta = [
  target: TemplateNodeIndex,
  start: number,
  count: number,
];

/**
 * Compiled condition — JSON metadata carries a function index plus the paths it
 * references. The Rust compiler emits the actual function bodies in a separate
 * closure array, and the runtime normalizes indexes to functions once.
 *
 * - `[0]` — evaluator function or component-local function index
 * - `[1]` — referenced paths for the reactive path index
 */
export type CompiledConditionFn = (v: (path: string, s?: unknown) => unknown, s?: unknown) => boolean;
export type CompiledCondition = [fn: CompiledConditionFn, paths: string[]];
export type SerializedCompiledCondition = [fnIndex: number, paths: string[]];
export type TemplateCondition = CompiledCondition | SerializedCompiledCondition;
export type CompiledConditionalMeta = [condition: TemplateCondition, blockIndex: number, slot: TemplateSlot];

export type CompiledAttrMeta =
  | [name: string, kind: 0, value: string]
  | [name: string, kind: 1, value: string]
  | [name: string, kind: 2, condition: TemplateCondition]
  | [name: string, kind: 3, parts: CompiledAttrPart[]];

export type CompiledRepeatMeta = [
  collection: string,
  itemVar: string,
  blockIndex: number,
  slot: TemplateSlot,
  keyPath?: string,
];
export type CompiledEventArg =
  | ['e']
  | ['p', string]
  | ['s', string]
  | ['n', number]
  | ['b', number]
  | ['z'];
export type CompiledEventArgs = CompiledEventArg[];
export type CompiledEventBindingMeta = [
  handler: string,
  args: CompiledEventArgs,
  target: TemplateNodeIndex,
  usesEvent?: 1,
];
export type CompiledEventGroupMeta = [name: string, bindings: CompiledEventBindingMeta[]];

export interface TemplateBlockMeta {
  h: string;
  tx?: CompiledTextRunMeta[];
  a?: CompiledAttrMeta[];
  ag?: CompiledAttrGroupMeta[];
  c?: CompiledConditionalMeta[];
  r?: CompiledRepeatMeta[];
  eg?: CompiledEventGroupMeta[];
}

export interface TemplateMeta extends TemplateBlockMeta {
  b?: TemplateBlockMeta[];
  sa?: string;
  re?: [string, string, CompiledEventArgs][];
  /** Component-level state roots referenced by template bindings. */
  tr?: string[];
  /** Observed host attributes index-aligned with `tr`. */
  ta?: string[];
  /** Compact shadow DOM flag - when present, client-created components use a shadow root. */
  sd?: 1;
  /** Internal compiler-owned dormant TemplateElement host flag. */
  th?: boolean | 1;
  /**
   * Compiler-owned work policy: `1` defers SSR hydration by viewport;
   * `2` couples hydration to browser-managed lazy rendering relevance.
   */
  wp?: 1 | 2;
}

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
export type TemplateNodePath = number[];
export type TemplateSlotPath = [
  parentPath: TemplateNodePath,
  beforeIndex: number,
  order?: number,
];
export type CompiledTextRunMeta = [slot: TemplateSlotPath, parts: CompiledAttrPart[], raw?: 1];
export type CompiledAttrGroupMeta = [
  target: TemplateNodePath,
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
export type CompiledConditionalMeta = [condition: TemplateCondition, blockIndex: number, slot: TemplateSlotPath];

export type CompiledAttrMeta =
  | [name: string, kind: 0, value: string]
  | [name: string, kind: 1, value: string]
  | [name: string, kind: 2, condition: TemplateCondition]
  | [name: string, kind: 3, parts: CompiledAttrPart[]];

export type CompiledRepeatMeta = [collection: string, itemVar: string, blockIndex: number, slot: TemplateSlotPath];
export type CompiledEventArg =
  | ['e']
  | ['p', string]
  | ['s', string]
  | ['n', number]
  | ['b', number]
  | ['z'];
export type CompiledEventArgs = CompiledEventArg[];
export type CompiledEventMeta = [name: string, handler: string, args: CompiledEventArgs, target: TemplateNodePath];

export interface TemplateBlockMeta {
  h: string;
  tx?: CompiledTextRunMeta[];
  a?: CompiledAttrMeta[];
  ag?: CompiledAttrGroupMeta[];
  c?: CompiledConditionalMeta[];
  r?: CompiledRepeatMeta[];
  e?: CompiledEventMeta[];
}

export interface TemplateMeta extends TemplateBlockMeta {
  b?: TemplateBlockMeta[];
  sa?: string;
  re?: [string, string, CompiledEventArgs][];
  /** Component-level state roots referenced by template bindings. */
  tr?: string[];
  /** Flat observed-attribute map: `[attrName, stateRoot, ...]`. */
  ta?: string[];
  /** Template feature bitmask emitted by the compiler. */
  tf?: number;
  /** Shadow DOM flag — when true, client-created components use shadow root. */
  sd?: boolean;
  /** Internal auto-element flag — true when the parser found no authored script. */
  ae?: boolean | 1;
}

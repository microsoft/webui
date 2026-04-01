// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

/**
 * Template type definitions — shared across packages without pulling in
 * the global `Window` augmentation from `template.ts`.
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
 * Compiled condition — a pre-compiled JS function plus the paths it references.
 * The Rust compiler emits the function body at build time so the runtime
 * doesn't need a condition AST interpreter.
 *
 * - `[0]` — evaluator function: `(resolve, scope) => boolean`
 * - `[1]` — referenced paths for the reactive path index
 */
export type CompiledCondition = [
  fn: (v: (path: string, s?: unknown) => unknown, s?: unknown) => boolean,
  paths: string[],
];
export type CompiledConditionalMeta = [condition: CompiledCondition, blockIndex: number];

export type CompiledAttrMeta =
  | [name: string, kind: 0, value: string]
  | [name: string, kind: 1, value: string]
  | [name: string, kind: 2, condition: CompiledCondition]
  | [name: string, kind: 3, parts: CompiledAttrPart[]];

export interface TemplateBlockMeta {
  h: string;
  tx?: CompiledTextRunMeta[];
  a?: CompiledAttrMeta[];
  ag?: CompiledAttrGroupMeta[];
  c?: CompiledConditionalMeta[];
  cl?: TemplateSlotPath[];
  r?: [string, string, number][];
  rl?: TemplateSlotPath[];
  e?: [string, string, number][];
  el?: TemplateNodePath[];
}

export interface TemplateMeta extends TemplateBlockMeta {
  b?: TemplateBlockMeta[];
  sa?: string;
  re?: [string, string, number][];
  /** Shadow DOM flag — when true, client-created components use shadow root. */
  sd?: boolean;
}

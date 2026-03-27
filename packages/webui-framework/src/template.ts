// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

/**
 * Template registry — stores compiled metadata objects from the Rust compiler.
 *
 * Each entry is a metadata object with:
 * - `h`  — marker-free static HTML for the client-created path, including any
 *          baked-in `<link>` / `<style>` nodes from the active CSS strategy
 * - `t`  — text binding paths used by existing SSR hydration metadata
 * - `tx` — client text runs `[slot, parts]`
 * - `a`  — attribute binding metadata
 * - `ag` — client attribute target groups
 * - `c`  — conditional blocks (array of [conditionAst, blockIndex])
 * - `cl` — conditional anchor slots
 * - `r`  — repeat/for blocks (array of [collection, itemVar, blockIndex])
 * - `rl` — repeat anchor slots
 * - `e`  — body events (array of [eventName, handlerName, needsEvent])
 * - `el` — event target element paths
 * - `b`  — nested compiled block metadata
 * - `sa` — optional adopted stylesheet specifier for module-mode component CSS
 * - `re` — root events (array of [eventName, handlerName, needsEvent])
 */

export type CompiledAttrPart = string | [path: string];
export type TemplateNodePath = number[];
export type TemplateSlotPath = [
  parentPath: TemplateNodePath,
  beforeIndex: number,
  order?: number,
];
export type CompiledTextRunMeta = [slot: TemplateSlotPath, parts: CompiledAttrPart[]];
export type CompiledAttrGroupMeta = [
  target: TemplateNodePath,
  start: number,
  count: number,
];

export type CompiledComparisonOperator = 1 | 2 | 3 | 4 | 5 | 6;
export type CompiledLogicalOperator = 1 | 2;
export type CompiledConditionExpr =
  | [kind: 0, value: string]
  | [kind: 1, left: string, operator: CompiledComparisonOperator, right: string]
  | [kind: 2, condition: CompiledConditionExpr]
  | [
    kind: 3,
    left: CompiledConditionExpr,
    operator: CompiledLogicalOperator,
    right: CompiledConditionExpr,
  ];
export type CompiledConditionalMeta = [condition: CompiledConditionExpr, blockIndex: number];

export type CompiledAttrMeta =
  | [name: string, kind: 0, value: string]
  | [name: string, kind: 1, value: string]
  | [name: string, kind: 2, condition: CompiledConditionExpr]
  | [name: string, kind: 3, parts: CompiledAttrPart[]];

export interface TemplateBlockMeta {
  h: string;
  t?: string[];
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
}

declare global {
  interface Window {
    __webui_templates?: Record<string, TemplateMeta>;
  }
}

export function getTemplate(name: string): TemplateMeta | undefined {
  return window.__webui_templates?.[name];
}

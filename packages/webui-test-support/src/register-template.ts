// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import type {
  CompiledAttrGroupMeta,
  CompiledAttrMeta,
  CompiledAttrPart,
  CompiledComparisonOperator,
  CompiledConditionExpr,
  CompiledConditionalMeta,
  CompiledLogicalOperator,
  CompiledTextRunMeta,
  TemplateBlockMeta,
  TemplateMeta,
  TemplateNodePath,
  TemplateSlotPath,
} from '../../webui-framework/src/template.js';

export type FixtureNodePath = TemplateNodePath;

/** Zero-based insertion slot inside a parent node path. */
export interface FixtureSlot {
  parent?: FixtureNodePath;
  before: number;
  order?: number;
}

export interface FixtureTextRun {
  slot: FixtureSlot;
  parts: CompiledAttrPart[];
}

export interface FixtureAttrGroup {
  target: FixtureNodePath;
  startIndex: number;
  bindingCount: number;
}

export interface FixtureCondition {
  when: CompiledConditionExpr;
  blockIndex: number;
}

export interface FixtureRepeat {
  each: string;
  as: string;
  blockIndex: number;
}

export interface FixtureEvent {
  type: string;
  handler: string;
  needsEvent?: boolean;
}

export interface FixtureCompiledBlockMeta {
  h: string;
  text?: FixtureTextRun[];
  attrs?: CompiledAttrMeta[];
  attrGroups?: FixtureAttrGroup[];
  conditionals?: FixtureCondition[];
  conditionSlots?: FixtureSlot[];
  repeats?: FixtureRepeat[];
  repeatSlots?: FixtureSlot[];
  events?: FixtureEvent[];
  eventTargets?: FixtureNodePath[];
}

export interface FixtureCompiledTemplateMeta extends FixtureCompiledBlockMeta {
  blocks?: FixtureCompiledBlockMeta[];
  adoptedStylesheet?: string;
  rootEvents?: FixtureEvent[];
}

/** Builds a zero-based child path from the current block root. */
export function nodePath(...segments: number[]): FixtureNodePath {
  return segments;
}

/** Declares a zero-based insertion slot inside `parent` before `before`. */
export function slot(options: FixtureSlot): FixtureSlot {
  return options;
}

/** Marks a dynamic value lookup inside text or template attribute parts. */
export function dynamic(path: string): [path: string] {
  return [path];
}

export function identifier(path: string): CompiledConditionExpr {
  return [0, path];
}

export function predicate(
  left: string,
  operator: CompiledComparisonOperator,
  right: string,
): CompiledConditionExpr {
  return [1, left, operator, right];
}

export function not(condition: CompiledConditionExpr): CompiledConditionExpr {
  return [2, condition];
}

export function compound(
  left: CompiledConditionExpr,
  operator: CompiledLogicalOperator,
  right: CompiledConditionExpr,
): CompiledConditionExpr {
  return [3, left, operator, right];
}

export function stringLiteral(value: string): string {
  return JSON.stringify(value);
}

export function numberLiteral(value: number): string {
  return String(value);
}

export function booleanLiteral(value: boolean): string {
  return value ? 'true' : 'false';
}

export function eq(left: string, right: string): CompiledConditionExpr {
  return predicate(left, 3, right);
}

export function neq(left: string, right: string): CompiledConditionExpr {
  return predicate(left, 4, right);
}

export function gt(left: string, right: string): CompiledConditionExpr {
  return predicate(left, 1, right);
}

export function lt(left: string, right: string): CompiledConditionExpr {
  return predicate(left, 2, right);
}

export function gte(left: string, right: string): CompiledConditionExpr {
  return predicate(left, 5, right);
}

export function lte(left: string, right: string): CompiledConditionExpr {
  return predicate(left, 6, right);
}

export function and(
  left: CompiledConditionExpr,
  right: CompiledConditionExpr,
): CompiledConditionExpr {
  return compound(left, 1, right);
}

export function or(
  left: CompiledConditionExpr,
  right: CompiledConditionExpr,
): CompiledConditionExpr {
  return compound(left, 2, right);
}

/** Inserts one runtime text node at `target` using the provided static/dynamic parts. */
export function bindText(
  target: FixtureSlot,
  ...parts: CompiledAttrPart[]
): FixtureTextRun {
  return { slot: target, parts };
}

/** Binds an HTML attribute directly to a state path. */
export function bindAttr(name: string, path: string): CompiledAttrMeta {
  return [name, 0, path];
}

/** Binds a property (`:${name}` in compiled metadata) to a state path. */
export function bindProp(name: string, path: string): CompiledAttrMeta {
  return [`:${name}`, 1, path];
}

/** Binds a boolean attribute that toggles from a condition expression. */
export function bindBoolAttr(name: string, condition: CompiledConditionExpr): CompiledAttrMeta {
  return [name, 2, condition];
}

/** Binds an attribute assembled from static strings and dynamic parts. */
export function bindTemplateAttr(
  name: string,
  ...parts: CompiledAttrPart[]
): CompiledAttrMeta {
  return [name, 3, parts];
}

/** Group contiguous `attrs` entries that should be attached to one target element. */
export function attrTarget(
  target: FixtureNodePath,
  options: {
    startIndex: number;
    bindingCount: number;
  },
): FixtureAttrGroup {
  return { target, ...options };
}

/** Render `blocks[blockIndex]` when `condition` is truthy. */
export function when(
  condition: CompiledConditionExpr,
  options: {
    blockIndex: number;
  },
): FixtureCondition {
  return { when: condition, ...options };
}

/** Repeat `blocks[blockIndex]` for each item in `each`, exposed as `as`. */
export function repeat(
  each: string,
  as: string,
  options: {
    blockIndex: number;
  },
): FixtureRepeat {
  return { each, as, ...options };
}

/** Attach an event listener to the node at the matching `eventTargets` path. */
export function bindEvent(
  type: string,
  handler: string,
  needsEvent = false,
): FixtureEvent {
  return { type, handler, needsEvent };
}

function normalizeSlot(entry: FixtureSlot): TemplateSlotPath {
  const parent = entry.parent ?? [];
  if (entry.order == null) {
    return [parent, entry.before];
  }

  return [parent, entry.before, entry.order];
}

function normalizeTextRun(entry: FixtureTextRun): CompiledTextRunMeta {
  return [normalizeSlot(entry.slot), entry.parts];
}

function normalizeAttrGroup(entry: FixtureAttrGroup): CompiledAttrGroupMeta {
  return [entry.target, entry.startIndex, entry.bindingCount];
}

function normalizeCondition(entry: FixtureCondition): CompiledConditionalMeta {
  return [entry.when, entry.blockIndex];
}

function normalizeRepeat(entry: FixtureRepeat): [string, string, number] {
  return [entry.each, entry.as, entry.blockIndex];
}

function normalizeEvent(entry: FixtureEvent): [string, string, number] {
  return [entry.type, entry.handler, entry.needsEvent ? 1 : 0];
}

function normalizeBlock(meta: FixtureCompiledBlockMeta): TemplateBlockMeta {
  const block: TemplateBlockMeta = { h: meta.h };

  if (meta.text?.length) {
    block.tx = meta.text.map(normalizeTextRun);
  }

  if (meta.attrs?.length) {
    block.a = meta.attrs;
  }

  if (meta.attrGroups?.length) {
    block.ag = meta.attrGroups.map(normalizeAttrGroup);
  }

  if (meta.conditionals?.length) {
    block.c = meta.conditionals.map(normalizeCondition);
  }

  if (meta.conditionSlots?.length) {
    block.cl = meta.conditionSlots.map(normalizeSlot);
  }

  if (meta.repeats?.length) {
    block.r = meta.repeats.map(normalizeRepeat);
  }

  if (meta.repeatSlots?.length) {
    block.rl = meta.repeatSlots.map(normalizeSlot);
  }

  if (meta.events?.length) {
    block.e = meta.events.map(normalizeEvent);
  }

  if (meta.eventTargets?.length) {
    block.el = meta.eventTargets;
  }

  return block;
}

export function buildTemplate(meta: FixtureCompiledTemplateMeta): TemplateMeta {
  const template: TemplateMeta = { ...normalizeBlock(meta) };

  if (meta.blocks?.length) {
    template.b = meta.blocks.map(normalizeBlock);
  }

  if (meta.adoptedStylesheet) {
    template.sa = meta.adoptedStylesheet;
  }

  if (meta.rootEvents?.length) {
    template.re = meta.rootEvents.map(normalizeEvent);
  }

  return template;
}

export const compileTemplate = buildTemplate;

function isFixtureCompiledTemplateMeta(
  meta: FixtureCompiledTemplateMeta | TemplateMeta,
): meta is FixtureCompiledTemplateMeta {
  return (
    'text' in meta ||
    'attrs' in meta ||
    'attrGroups' in meta ||
    'conditionals' in meta ||
    'conditionSlots' in meta ||
    'repeats' in meta ||
    'repeatSlots' in meta ||
    'events' in meta ||
    'eventTargets' in meta ||
    'blocks' in meta ||
    'adoptedStylesheet' in meta ||
    'rootEvents' in meta
  );
}

function normalizeTemplateMeta(
  meta: FixtureCompiledTemplateMeta | TemplateMeta,
): TemplateMeta {
  return isFixtureCompiledTemplateMeta(meta) ? buildTemplate(meta) : meta;
}

export function registerCompiledTemplate(
  name: string,
  meta: FixtureCompiledTemplateMeta | TemplateMeta,
): void {
  const templates = window.__webui_templates ?? (window.__webui_templates = {});
  templates[name] = normalizeTemplateMeta(meta);
}

export function renderTemplateScript(name: string, meta: TemplateMeta): string {
  return `<script>(function(){var w=window.__webui_templates||(window.__webui_templates={});w[${JSON.stringify(name)}]=${JSON.stringify(meta)};})();</script>`;
}

// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import type {
  CompiledAttrGroupMeta,
  CompiledAttrMeta,
  CompiledAttrPart,
  CompiledCondition,
  CompiledConditionalMeta,
  CompiledTextRunMeta,
  TemplateBlockMeta,
  TemplateMeta,
  TemplateNodePath,
  TemplateSlotPath,
} from '../../webui-framework/src/template-types.js';

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
  when: CompiledCondition;
  blockIndex: number;
  slot: FixtureSlot;
}

export interface FixtureRepeat {
  each: string;
  as: string;
  blockIndex: number;
  slot: FixtureSlot;
}

export interface FixtureEvent {
  type: string;
  handler: string;
  needsEvent?: boolean;
  target: FixtureNodePath;
}

export interface FixtureCompiledBlockMeta {
  h: string;
  text?: FixtureTextRun[];
  attrs?: CompiledAttrMeta[];
  attrGroups?: FixtureAttrGroup[];
  conditionals?: FixtureCondition[];
  repeats?: FixtureRepeat[];
  events?: FixtureEvent[];
}

export interface FixtureCompiledTemplateMeta extends FixtureCompiledBlockMeta {
  blocks?: FixtureCompiledBlockMeta[];
  adoptedStylesheet?: string;
  rootEvents?: FixtureEvent[];
  /** Shadow DOM flag — when true, client-created components use shadow root. */
  shadowDom?: boolean;
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

function parseLiteral(value: string): unknown {
  if (
    (value.startsWith('"') && value.endsWith('"')) ||
    (value.startsWith("'") && value.endsWith("'"))
  ) {
    return value.slice(1, -1);
  }
  if (value === 'true') return true;
  if (value === 'false') return false;
  const num = Number(value);
  if (!Number.isNaN(num)) return num;
  return undefined; // not a literal — it's an identifier
}

function makeComparator(op: number): (a: unknown, b: unknown) => boolean {
  switch (op) {
    case 1:
      return (a, b) => (a as number) > (b as number);
    case 2:
      return (a, b) => (a as number) < (b as number);
    case 3:
      return (a, b) => Object.is(a, b);
    case 4:
      return (a, b) => !Object.is(a, b);
    case 5:
      return (a, b) => (a as number) >= (b as number);
    case 6:
      return (a, b) => (a as number) <= (b as number);
    default:
      return () => false;
  }
}

export function identifier(path: string): CompiledCondition {
  return [(v, s) => !!v(path, s), [path]];
}

export function predicate(
  left: string,
  operator: number,
  right: string,
): CompiledCondition {
  const literal = parseLiteral(right);
  const paths = [left];
  if (literal === undefined) paths.push(right); // right is also an identifier
  const cmp = makeComparator(operator);
  return [
    (v, s) => cmp(v(left, s), literal !== undefined ? literal : v(right, s)),
    paths,
  ];
}

export function not(condition: CompiledCondition): CompiledCondition {
  return [(v, s) => !condition[0](v, s), condition[1]];
}

export function compound(
  left: CompiledCondition,
  operator: number,
  right: CompiledCondition,
): CompiledCondition {
  if (operator === 1) {
    // AND
    return [
      (v, s) => left[0](v, s) && right[0](v, s),
      [...left[1], ...right[1]],
    ];
  }
  // OR
  return [
    (v, s) => left[0](v, s) || right[0](v, s),
    [...left[1], ...right[1]],
  ];
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

export function eq(left: string, right: string): CompiledCondition {
  return predicate(left, 3, right);
}

export function neq(left: string, right: string): CompiledCondition {
  return predicate(left, 4, right);
}

export function gt(left: string, right: string): CompiledCondition {
  return predicate(left, 1, right);
}

export function lt(left: string, right: string): CompiledCondition {
  return predicate(left, 2, right);
}

export function gte(left: string, right: string): CompiledCondition {
  return predicate(left, 5, right);
}

export function lte(left: string, right: string): CompiledCondition {
  return predicate(left, 6, right);
}

export function and(
  left: CompiledCondition,
  right: CompiledCondition,
): CompiledCondition {
  return compound(left, 1, right);
}

export function or(
  left: CompiledCondition,
  right: CompiledCondition,
): CompiledCondition {
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

/** Binds a property to a state path (complex binding, kind=1). */
export function bindProp(name: string, path: string): CompiledAttrMeta {
  return [name, 1, path];
}

/** Binds a boolean attribute that toggles from a condition expression. */
export function bindBoolAttr(name: string, condition: CompiledCondition): CompiledAttrMeta {
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
  condition: CompiledCondition,
  options: {
    blockIndex: number;
    slot?: FixtureSlot;
  },
): FixtureCondition {
  return { when: condition, blockIndex: options.blockIndex, slot: options.slot ?? { before: 0 } };
}

/** Repeat `blocks[blockIndex]` for each item in `each`, exposed as `as`. */
export function repeat(
  each: string,
  as: string,
  options: {
    blockIndex: number;
    slot?: FixtureSlot;
  },
): FixtureRepeat {
  return { each, as, blockIndex: options.blockIndex, slot: options.slot ?? { before: 0 } };
}

/** Attach an event listener to the node at `target`. */
export function bindEvent(
  type: string,
  handler: string,
  needsEvent: boolean | number = false,
  target: FixtureNodePath = [],
): FixtureEvent {
  return { type, handler, needsEvent: !!needsEvent, target };
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
  return [entry.when, entry.blockIndex, normalizeSlot(entry.slot)];
}

function normalizeRepeat(entry: FixtureRepeat): [string, string, number, TemplateSlotPath] {
  return [entry.each, entry.as, entry.blockIndex, normalizeSlot(entry.slot)];
}

function normalizeEvent(entry: FixtureEvent): [string, string, number, TemplateNodePath] {
  return [entry.type, entry.handler, entry.needsEvent ? 1 : 0, entry.target];
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

  if (meta.repeats?.length) {
    block.r = meta.repeats.map(normalizeRepeat);
  }

  if (meta.events?.length) {
    block.e = meta.events.map(normalizeEvent);
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
    template.re = meta.rootEvents.map(e => [e.type, e.handler, e.needsEvent ? 1 : 0] as [string, string, number]);
  }

  if (meta.shadowDom) {
    template.sd = true;
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
    'repeats' in meta ||
    'events' in meta ||
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
  // Use a local cast so the assignment works regardless of which framework
  // package provides the global Window augmentation for __webui_templates.
  const w = window as unknown as { __webui_templates?: Record<string, TemplateMeta> };
  const templates = w.__webui_templates ?? (w.__webui_templates = {});
  templates[name] = normalizeTemplateMeta(meta);
}

export function renderTemplateScript(name: string, meta: TemplateMeta): string {
  return `<script>(function(){var w=window.__webui_templates||(window.__webui_templates={});w[${JSON.stringify(name)}]=${JSON.stringify(meta)};})();</script>`;
}

// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

/**
 * Template registry — stores compiled metadata objects from the Rust compiler.
 *
 * Each entry is a metadata object with:
 * - `h`  — static HTML for the component template
 * - `tx` — text runs `[slot, parts]` for text binding positions
 * - `a`  — attribute binding metadata
 * - `ag` — attribute target groups `[path, startIndex, count]`
 * - `c`  — conditional blocks `[conditionRef, blockIndex, slot]`
 * - `r`  — repeat/for blocks `[collection, itemVar, blockIndex, slot]`
 * - `e`  — element events `[eventName, handlerName, argSpecs, targetPath]`
 * - `b`  — nested compiled block metadata
 * - `sa` — adopted stylesheet specifier for CSS module strategy
 * - `sd` — shadow DOM flag for client-created components
 * - `re` — root events on the host element
 * - `tr` — state roots referenced by the compiled template
 * - `ta` — observed host attributes for those roots, flat `[attr, root]` pairs
 * - `tf` — compiler feature bitmask
 * - `ae` — internal auto-element eligibility flag for scriptless templates
 *
 * Template registration also notifies optional runtimes, such as the
 * auto-element runtime, so dynamically loaded route templates can be claimed
 * without coupling the router package to the framework package.
 */

export type {
  CompiledAttrGroupMeta,
  CompiledAttrMeta,
  CompiledAttrPart,
  CompiledCondition,
  CompiledConditionFn,
  CompiledConditionalMeta,
  CompiledEventArg,
  CompiledEventArgs,
  SerializedCompiledCondition,
  TemplateCondition,
  CompiledTextRunMeta,
  TemplateBlockMeta,
  TemplateMeta,
  TemplateNodePath,
  TemplateSlotPath,
} from './template-types.js';

import type {
  CompiledConditionFn,
  SerializedCompiledCondition,
  TemplateBlockMeta,
  TemplateCondition,
  TemplateMeta,
} from './template-types.js';
import { dispatchTemplatesRegistered } from './template-events.js';

const WEBUI_DATA_ID = 'webui-data';
const normalizedTemplates = new WeakSet<TemplateMeta>();
let webuiDataLoaded = false;

declare global {
  interface Window {
    /** Consolidated SSR metadata loaded from `#webui-data` or partial responses. */
    __webui?: {
      state?: Record<string, unknown>;
      templates?: Record<string, TemplateMeta>;
      templateFns?: Record<string, CompiledConditionFn[]>;
      [key: string]: unknown;
    };
  }
}

/**
 * Return the normalized template metadata for a component tag.
 *
 * The first lookup lazily loads the SSR data block so components can hydrate
 * without every app eagerly parsing route/template metadata at startup.
 */
export function getTemplate(name: string): TemplateMeta | undefined {
  let meta = window.__webui?.templates?.[name];
  if (!meta) {
    loadWebUIDataBlock();
    meta = window.__webui?.templates?.[name];
  }
  if (meta) normalizeTemplate(name, meta);
  return meta;
}

/** Return the complete template registry, loading SSR data if needed. */
export function getTemplateRegistry(): Record<string, TemplateMeta> | undefined {
  loadWebUIDataBlock();
  return window.__webui?.templates;
}

/**
 * Register template metadata and optional condition closures at runtime.
 *
 * Used by component assets and tests. After registration, a DOM event is
 * dispatched so auto-elements can claim newly available scriptless templates.
 */
export function registerTemplateData(
  templates: Record<string, TemplateMeta>,
  templateFns?: Record<string, CompiledConditionFn[]>,
): void {
  const w = window as Window;
  if (!w.__webui) w.__webui = {};
  if (!w.__webui.templates) w.__webui.templates = {};
  if (templateFns) {
    if (!w.__webui.templateFns) w.__webui.templateFns = {};
    const fnNames = Object.keys(templateFns);
    for (let i = 0; i < fnNames.length; i++) {
      const tag = fnNames[i];
      w.__webui.templateFns[tag] = templateFns[tag];
    }
  }
  const names = Object.keys(templates);
  let hasTemplates = false;
  for (let i = 0; i < names.length; i++) {
    const tag = names[i];
    const meta = templates[tag];
    w.__webui.templates[tag] = meta;
    normalizeTemplate(tag, meta);
    hasTemplates = true;
  }
  if (hasTemplates) dispatchTemplatesRegistered(templates);
}

function loadWebUIDataBlock(): void {
  if (webuiDataLoaded || window.__webui?.state !== undefined || typeof document === 'undefined') return;
  const el = document.getElementById(WEBUI_DATA_ID);
  if (!el) {
    webuiDataLoaded = true;
    return;
  }

  const text = el.textContent;
  if (text) {
    const templateFns = window.__webui?.templateFns;
    const parsed = JSON.parse(text) as NonNullable<Window['__webui']>;
    if (templateFns) parsed.templateFns = templateFns;
    window.__webui = parsed;
  }
  el.remove();
  webuiDataLoaded = true;
}

function normalizeTemplate(name: string, meta: TemplateMeta): void {
  if (normalizedTemplates.has(meta)) return;
  const fns = window.__webui?.templateFns?.[name] ?? [];
  const stack: TemplateBlockMeta[] = [meta];
  while (stack.length > 0) {
    const block = stack.pop();
    if (!block) continue;
    if (block.a) {
      for (let i = 0; i < block.a.length; i++) {
        const attr = block.a[i];
        if (attr[1] === 2) normalizeCondition(name, attr[2], fns);
      }
    }
    if (block.c) {
      for (let i = 0; i < block.c.length; i++) {
        normalizeCondition(name, block.c[i][0], fns);
      }
    }
    const children = (block as TemplateMeta).b;
    if (children) {
      for (let i = 0; i < children.length; i++) stack.push(children[i]);
    }
  }
  normalizedTemplates.add(meta);
}

function normalizeCondition(
  tagName: string,
  condition: TemplateCondition,
  fns: CompiledConditionFn[],
): void {
  const first = condition[0];
  if (typeof first === 'function') return;
  const fn = fns[first];
  if (typeof fn !== 'function') {
    throw new Error(`[WebUI] Missing condition closure ${first} for <${tagName}>.`);
  }
  (condition as SerializedCompiledCondition as unknown as [CompiledConditionFn, string[]])[0] = fn;
}

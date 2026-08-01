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
 * - `r`  — repeat/for blocks `[collection, itemVar, blockIndex, slot, keyPath?]`
 * - `eg` — grouped element events `[eventName, [[handlerName, argSpecs, targetPath, usesEvent?]]]`
 * - `b`  — nested compiled block metadata
 * - `sa` — adopted stylesheet specifier for CSS module strategy
 * - `sd` — shadow DOM flag for client-created components
 * - `re` — root events on the host element
 * - `tr` — state roots referenced by the compiled template
 * - `ta` — observed host attributes index-aligned with `tr`
 * - `th` — compiler-owned dormant TemplateElement host flag
 *
 * Template registration notifies optional runtimes so dynamically loaded route
 * templates can be claimed without coupling the router to the framework.
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
  CompiledEventBindingMeta,
  CompiledEventGroupMeta,
  SerializedCompiledCondition,
  TemplateCondition,
  CompiledTextRunMeta,
  TemplateBlockMeta,
  TemplateMeta,
  TemplateNodePath,
  TemplateSlotPath,
} from './template-types.js';
import {
  dispatchTemplatesRegistered,
  templateRegistrationDetail,
  TEMPLATES_REGISTERED_EVENT,
} from './template-events.js';

import type {
  CompiledConditionFn,
  TemplateMeta,
} from './template-types.js';
import { validateAndNormalizeTemplate } from './template-validation.js';

const WEBUI_DATA_ID = 'webui-data';
const normalizedTemplates = new WeakSet<TemplateMeta>();
const assetValidatedTemplates = new WeakSet<TemplateMeta>();
let webuiDataLoaded = false;

interface PendingTemplateDefinition {
  readonly ctor: Function;
  readonly define: () => void;
}

/** Authored definitions waiting for their streamed template metadata. */
const pendingTemplateDefinitions = new Map<string, PendingTemplateDefinition>();

function acceptTemplateData(
  templates: Record<string, TemplateMeta>,
  templateFns?: Record<string, CompiledConditionFn[]>,
): boolean {
  prepareTemplateData(templates, templateFns);
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
  for (let i = 0; i < names.length; i++) {
    const tag = names[i];
    w.__webui.templates[tag] = templates[tag];
  }
  for (let i = 0; i < names.length; i++) {
    const pending = pendingTemplateDefinitions.get(names[i]);
    if (!pending) continue;
    pendingTemplateDefinitions.delete(names[i]);
    pending.define();
  }
  return names.length > 0;
}

if (typeof window !== 'undefined' && typeof window.addEventListener === 'function') {
  window.addEventListener(TEMPLATES_REGISTERED_EVENT, (event: Event) => {
    const templates = templateRegistrationDetail(event)?.templates;
    if (templates) acceptTemplateData(templates);
  });
}

declare global {
  interface Window {
    /** Consolidated SSR metadata loaded from `#webui-data` or partial responses. */
    __webui?: {
      state?: Record<string, unknown>;
      templates?: Record<string, TemplateMeta>;
      templateFns?: Record<string, CompiledConditionFn[]>;
      templateHostExclusions?: Set<string>;
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
 * Used by component assets and tests. Registration also lets the dormant-host
 * runtime claim newly available scriptless templates.
 */
export function registerTemplateData(
  templates: Record<string, TemplateMeta>,
  templateFns?: Record<string, CompiledConditionFn[]>,
): void {
  if (acceptTemplateData(templates, templateFns)) {
    dispatchTemplatesRegistered(templates);
  }
}

/**
 * Validate and normalize a complete template batch before registry mutation.
 *
 * Dynamic registrations use this preflight so malformed metadata cannot leave
 * a partially registered global template set.
 */
export function prepareTemplateData(
  templates: Record<string, unknown>,
  templateFns?: Record<string, unknown>,
): asserts templates is Record<string, TemplateMeta> {
  const names = Object.keys(templates);
  for (let i = 0; i < names.length; i++) {
    const name = names[i];
    const meta = templates[name];
    const candidateFns = templateFns?.[name];
    if (candidateFns !== undefined && !isConditionFunctionArray(candidateFns)) {
      throw new Error(`[WebUI] Template condition closures for <${name}> must be an array of functions.`);
    }
    normalizeTemplate(name, meta, candidateFns);
  }
}

/**
 * Strictly validate component asset templates against asset-local closures.
 *
 * Unlike streamed and SSR template registration, asset validation never falls
 * back to closures already present in the page-wide registry.
 */
export function prepareAssetTemplateData(
  templates: Record<string, unknown>,
  templateFns?: Record<string, unknown>,
): asserts templates is Record<string, TemplateMeta> {
  const names = Object.keys(templates);
  for (let i = 0; i < names.length; i++) {
    const name = names[i];
    const meta = templates[name];
    const known = templateObject(meta);
    if (known && assetValidatedTemplates.has(known)) continue;

    const candidateFns = templateFns?.[name];
    if (candidateFns !== undefined && !isConditionFunctionArray(candidateFns)) {
      throw new Error(`[WebUI] Template condition closures for <${name}> must be an array of functions.`);
    }
    const normalized = validateAndNormalizeTemplate(
      name,
      meta,
      candidateFns ?? [],
      false,
    );
    normalizedTemplates.add(normalized);
    assetValidatedTemplates.add(normalized);
  }
}

/**
 * Hold an authored custom-element definition until streamed metadata arrives.
 *
 * Internal to the framework package; the public registration surface remains
 * `TemplateElement.define()`.
 */
export function deferTemplateDefinition(
  tagName: string,
  ctor: Function,
  define: () => void,
): void {
  const pending = pendingTemplateDefinitions.get(tagName);
  if (pending) {
    if (pending.ctor === ctor) {
      throw new Error(`[WebUI] <${tagName}> is already pending definition.`);
    }
    throw new Error(`[WebUI] <${tagName}> already has a pending authored definition.`);
  }
  pendingTemplateDefinitions.set(tagName, { ctor, define });
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

function normalizeTemplate(
  name: string,
  meta: unknown,
  suppliedFns?: CompiledConditionFn[],
): void {
  const known = templateObject(meta);
  if (known && normalizedTemplates.has(known)) return;
  const fns = suppliedFns ?? window.__webui?.templateFns?.[name] ?? [];
  normalizedTemplates.add(
    validateAndNormalizeTemplate(name, meta, fns, true),
  );
}

function templateObject(value: unknown): TemplateMeta | undefined {
  return typeof value === 'object'
    && value !== null
    && !Array.isArray(value)
    ? value as TemplateMeta
    : undefined;
}

function isConditionFunctionArray(value: unknown): value is CompiledConditionFn[] {
  return Array.isArray(value)
    && value.every(candidate => typeof candidate === 'function');
}

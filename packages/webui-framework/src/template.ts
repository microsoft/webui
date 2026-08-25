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
 * - `eg` — grouped element events `[eventName, [[handlerName, argSpecs, targetIndex, usesEvent?]]]`
 * - `b`  — nested compiled block metadata
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
  TemplateNodeIndex,
  TemplateSlot,
} from './template-types.js';
import {
  dispatchTemplatesRegistered,
  templateRegistrationDetail,
  TEMPLATES_REGISTERED_EVENT,
} from './template-events.js';
import {
  prepareComponentStyles,
  registerComponentStyles,
  registerPreparedComponentStyles,
  requireComponentStyles,
  validateComponentStylesRegistration,
  type ComponentStyles,
} from './element/styles.js';
import {
  prepareComponentStyleLinks,
  prepareRegisteredLinkStyles,
} from './element/link-styles.js';

import type {
  CompiledCondition,
  CompiledConditionFn,
  TemplateBlockMeta,
  TemplateCondition,
  TemplateMeta,
} from './template-types.js';

const WEBUI_DATA_ID = 'webui-data';
const HYDRATION_COMPLETE_EVENT = 'webui:hydration-complete';
const TEMPLATE_FN_COUNT = Symbol.for('microsoft.webui.templateFnCount');
const normalizedTemplates = new WeakSet<TemplateMeta>();
const assetNormalizedTemplates = new WeakSet<TemplateMeta>();
let webuiDataLoaded = false;

type RuntimeTemplateFns = Record<string, CompiledConditionFn[]>
  & Record<symbol, number | undefined>;

function runtimeTemplateFnCount(registry: RuntimeTemplateFns): number {
  const count = registry[TEMPLATE_FN_COUNT];
  if (typeof count === 'number') return count;
  const initialized = Object.keys(registry).length;
  registry[TEMPLATE_FN_COUNT] = initialized;
  return initialized;
}

interface PendingTemplateDefinition {
  readonly ctor: Function;
  readonly define: () => void;
}

/** Authored definitions waiting for their streamed template metadata. */
const pendingTemplateDefinitions = new Map<string, PendingTemplateDefinition>();

function acceptTemplateData(
  templates: Record<string, TemplateMeta>,
  templateFns?: Record<string, CompiledConditionFn[]>,
  componentStyles?: ComponentStyles,
): boolean {
  validateComponentStylesRegistration(componentStyles);
  prepareTemplateData(templates, templateFns);
  if (componentStyles) {
    registerPreparedComponentStyles(componentStyles);
  }
  const w = window as Window;
  if (!w.__webui) w.__webui = {};
  if (!w.__webui.templates) w.__webui.templates = {};
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
  window.__webuiRegisterComponentStyles = (value: unknown): Promise<void> | undefined => {
    const styles = requireComponentStyles(value);
    validateComponentStylesRegistration(styles);
    registerPreparedComponentStyles(styles);
    return prepareComponentStyleLinks(styles);
  };
  if (window.__webui?.componentStyles) {
    registerComponentStyles(window.__webui.componentStyles);
  }
  window.addEventListener(TEMPLATES_REGISTERED_EVENT, (event: Event) => {
    const detail = templateRegistrationDetail(event);
    if (!detail?.templates) return;
    const styles = detail.componentStyles === undefined
      ? undefined
      : prepareComponentStyles(detail.componentStyles);
    acceptTemplateData(detail.templates, undefined, styles);
    const ready = prepareRegisteredLinkStyles(detail.templates);
    if (ready) detail.waitUntil?.(ready);
  });
  window.addEventListener(
    HYDRATION_COMPLETE_EVENT,
    releaseNonRoutedSSRBootstrapState,
    { once: true },
  );
}

declare global {
  interface WebUIRuntimeGlobal {
    state?: Record<string, unknown>;
    templates?: Record<string, TemplateMeta>;
    templateFns?: Record<string, CompiledConditionFn[]>;
    componentAssetStyles?: Record<string, readonly string[]>;
    templateHostExclusions?: Set<string>;
    componentStyles?: ComponentStyles;
    [key: string]: unknown;
  }

  interface Window {
    /** Consolidated SSR metadata loaded from `#webui-data` or partial responses. */
    __webui?: WebUIRuntimeGlobal;
    __webuiRegisterComponentStyles?: (value: unknown) => Promise<void> | undefined;
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
  componentStylesValue?: ComponentStyles | unknown,
): void {
  const componentStyles = prepareComponentStyles(componentStylesValue);
  if (acceptTemplateData(templates, templateFns, componentStyles)) {
    // Styles were already accepted above; listeners only need the templates.
    dispatchTemplatesRegistered(templates);
  }
}

function prepareTemplateData(
  templates: Record<string, TemplateMeta>,
  templateFns?: Record<string, CompiledConditionFn[]>,
): void {
  const names = Object.keys(templates);
  for (let i = 0; i < names.length; i++) {
    const name = names[i];
    normalizeTemplate(name, templates[name], templateFns?.[name]);
  }
}

/**
 * Resolve compiler-emitted component asset conditions against asset-local closures.
 *
 * Unlike streamed and SSR registration, asset normalization never falls back
 * to closures already present in the page-wide registry.
 */
export function prepareAssetTemplateData(
  templates: Record<string, TemplateMeta>,
  templateFns?: Record<string, CompiledConditionFn[]>,
): void {
  const names = Object.keys(templates);
  for (let i = 0; i < names.length; i++) {
    const name = names[i];
    const meta = templates[name];
    if (assetNormalizedTemplates.has(meta)) continue;
    normalizeTemplateConditions(name, meta, templateFns?.[name] ?? []);
    normalizedTemplates.add(meta);
    assetNormalizedTemplates.add(meta);
  }
}

/**
 * Hold an authored custom-element definition until compiled metadata arrives.
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
    const componentAssetStyles = window.__webui?.componentAssetStyles;
    const parsed = JSON.parse(text) as NonNullable<Window['__webui']>;
    if (templateFns) parsed.templateFns = templateFns;
    if (componentAssetStyles) parsed.componentAssetStyles = componentAssetStyles;
    // Publish before registering styles. A malformed `componentStyles` throws,
    // and doing it the other way round loses the templates and state that
    // parsed fine — then re-parses the whole block on the next lookup, because
    // nothing recorded that the work was already done.
    window.__webui = parsed;
    el.remove();
    webuiDataLoaded = true;
    registerComponentStyles(parsed.componentStyles);
    return;
  }
  el.remove();
  webuiDataLoaded = true;
}

/**
 * Release the one-shot page state after every startup hydration consumer has
 * copied the roots it owns. Template metadata remains available for future
 * client-created blocks and route navigation.
 *
 * @internal
 */
export function releaseSSRBootstrapState(): void {
  const runtime = window.__webui;
  if (runtime?.state !== undefined) {
    delete runtime.state;
  }
}

/** Release only on non-routed pages; the router owns routed startup state. */
export function releaseNonRoutedSSRBootstrapState(): void {
  const runtime = window.__webui;
  if (
    runtime?.chain === undefined &&
    runtime?.templateHostExclusions === undefined
  ) {
    releaseSSRBootstrapState();
  }
}

function normalizeTemplate(
  name: string,
  meta: TemplateMeta,
  suppliedFns?: CompiledConditionFn[],
): void {
  if (normalizedTemplates.has(meta)) return;
  const runtimeFns = window.__webui?.templateFns as
    | RuntimeTemplateFns
    | undefined;
  const fns = suppliedFns ?? runtimeFns?.[name] ?? [];
  normalizeTemplateConditions(name, meta, fns);
  normalizedTemplates.add(meta);
  if (suppliedFns === undefined && runtimeFns?.[name] === fns) {
    let remaining = runtimeTemplateFnCount(runtimeFns);
    delete runtimeFns[name];
    remaining--;
    runtimeFns[TEMPLATE_FN_COUNT] = remaining;
    if (remaining === 0 && window.__webui) {
      const liveCount = Object.keys(runtimeFns).length;
      if (liveCount === 0) {
        delete window.__webui.templateFns;
      } else {
        runtimeFns[TEMPLATE_FN_COUNT] = liveCount;
      }
    }
  }
}

function normalizeTemplateConditions(
  name: string,
  meta: TemplateMeta,
  fns: CompiledConditionFn[],
): void {
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
  (condition as CompiledCondition)[0] = fn;
}

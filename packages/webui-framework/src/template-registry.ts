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
  TemplateBlockReference,
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
  validateComponentStylesRegistration,
  type ComponentStyles,
} from './element/style-catalog.js';

import { TEMPLATE_META_VERSION, type TemplateMeta } from './template-types.js';

import { registerTrustedTemplateBlock } from './trusted-types-policy.js';
import { noteTemplateRead, templateCacheHooks } from './template-cache-contract.js';

const WEBUI_DATA_ID = 'webui-data';
const HYDRATION_COMPLETE_EVENT = 'webui:hydration-complete';
const normalizedTemplates = new WeakSet<TemplateMeta>();
let webuiDataLoaded = false;

interface PendingTemplateDefinition {
  readonly ctor: Function;
  readonly define: (ctor: Function, tagName: string, meta: TemplateMeta) => void;
}

/** Authored definitions waiting for runtime startup or compiled metadata. */
const pendingTemplateDefinitions = new Map<string, PendingTemplateDefinition>();
export let templateRuntimeReady = false;

/** Enable authored definitions, returning whether rendering was newly started. */
export function activateTemplateDefinitions(): boolean {
  if (templateRuntimeReady) return false;
  templateRuntimeReady = true;
  for (const tag of pendingTemplateDefinitions.keys()) {
    completeTemplateDefinition(tag);
  }
  return true;
}

function completeTemplateDefinition(tag: string): void {
  const pending = pendingTemplateDefinitions.get(tag);
  if (!pending) return;
  const meta = getTemplate(tag);
  if (!meta) return;
  pendingTemplateDefinitions.delete(tag);
  pending.define(pending.ctor, tag, meta);
}
let prepareResources: (
  (templates: Record<string, TemplateMeta>) => Promise<void> | undefined
) | undefined;
type DefinitionPreparation = (
  templates: Record<string, TemplateMeta>,
  names: readonly string[],
) => Promise<void> | undefined;
let prepareDefinitions: DefinitionPreparation | null | undefined;

/** Attach client-mount resource preparation without a second registry listener. */
export function installTemplateResourcePreparation(
  prepare: NonNullable<typeof prepareResources>,
): void {
  prepareResources = prepare;
}

/** Null permanently hands definition readiness to the installed host runtime. */
export function installTemplateDefinitionPreparation(
  prepare: DefinitionPreparation | null,
): void {
  if (prepareDefinitions !== null) prepareDefinitions = prepare;
}

function acceptTemplateData(
  templates: Record<string, TemplateMeta>,
  componentStyles?: ComponentStyles,
): string[] {
  validateComponentStylesRegistration(componentStyles);
  const names = prepareTemplateData(templates);
  templateCacheHooks?.accept(templates);
  if (componentStyles) {
    registerPreparedComponentStyles(componentStyles);
  }
  const registry = (window.__webui ??= {}).templates ??= {};
  for (let i = 0; i < names.length; i++) {
    const tag = names[i];
    registry[tag] = templates[tag];
  }
  if (templateRuntimeReady) {
    for (let i = 0; i < names.length; i++) {
      completeTemplateDefinition(names[i]);
    }
  }
  return names;
}

let registryInstalled = false;

/** Install this realm's renderer resource bridge once. Pure API imports do not install it. */
export function installTemplateRegistry(): void {
  if (typeof window === 'undefined' || typeof window.addEventListener !== 'function' ||
      registryInstalled) return;
  registryInstalled = true;
  if (window.__webui?.componentStyles) {
    registerComponentStyles(window.__webui.componentStyles);
  }
  window.addEventListener(TEMPLATES_REGISTERED_EVENT, (event: Event) => {
    const detail = templateRegistrationDetail(event);
    if (!detail?.templates) return;
    try {
      const styles = detail.componentStyles === undefined
        ? undefined
        : prepareComponentStyles(detail.componentStyles);
      const templates = detail.templates;
      const names = acceptTemplateData(templates, styles);
      const definitions = detail.waitUntil
        ? prepareDefinitions?.(templates, names)
        : undefined;
      if (definitions) {
        detail.waitUntil?.(definitions.then(() => prepareResources?.(templates)));
      } else {
        const ready = prepareResources?.(templates);
        if (ready) detail.waitUntil?.(ready);
      }
    } catch (error) {
      if (!detail.waitUntil) throw error;
      detail.waitUntil(Promise.reject(error));
    }
  });
  window.__webuiPrepareTemplates = (values) => {
    for (const tag of Object.keys(values)) {
      const value = values[tag] as TemplateMeta | null | undefined;
      if (typeof value !== 'object' || typeof value?.h !== 'string') {
        throw new Error(`[WebUI] Invalid compiled template <${tag}>. Rebuild matching artifacts.`);
      }
    }
    const templates = values as Record<string, TemplateMeta>;
    prepareTemplateData(templates);
    templateCacheHooks?.accept(templates);
  };
  window.addEventListener(
    HYDRATION_COMPLETE_EVENT,
    releaseNonRoutedSSRBootstrapState,
    { once: true },
  );
}

declare global {
  interface WebUIRuntimeGlobal {
    /**
     * Base URL or path, ending in `/`, of compiler-emitted assets when they are
     * not served relative to the document. It may be relative to the document.
     */
    assetBase?: string;
    buildId?: string;
    /** Hex bitset of compiled templates this document holds. */
    inventory?: string;
    styleInventory?: string;
    inventoryEpoch?: number;
    /** CSP nonce for scripts and styles the runtime inserts. */
    nonce?: string;
    state?: Record<string, unknown>;
    templates?: Record<string, TemplateMeta>;
    componentAssetStyles?: Record<string, readonly string[]>;
    templateHostExclusions?: Set<string>;
    componentStyles?: ComponentStyles;
    [key: string]: unknown;
  }

  interface Window {
    /** Consolidated SSR metadata loaded from `#webui-data` or partial responses. */
    __webui?: WebUIRuntimeGlobal;
    __webuiRegisterComponentStyles?: (value: unknown) => Promise<void> | undefined;
    __webuiPrepareTemplates?: (templates: Record<string, unknown>) => void;
    /**
     * Installed by the style runtime: prepare the stylesheets of component style
     * roots. `priority: 'low'` prepares them speculatively, once the window
     * has loaded; until then the result has nothing to wait for.
     */
    __webuiPrepareResources?: (
      ids: readonly string[],
      priority?: RequestPriority,
    ) => Promise<void> | undefined;
    /**
     * Installed by the style runtime: preload base-relative stylesheet paths, shared
     * with later style registration. `priority: 'low'` preloads speculatively,
     * once the window has loaded; until then the result has nothing to wait for.
     */
    __webuiPreloadStyles?: (
      hrefs: readonly string[],
      priority?: RequestPriority,
    ) => Promise<void> | undefined;
    __webuiCollectTemplates?: (keep?: readonly string[]) => number;
    __webuiRetainTemplate?: (tag: string) => unknown;
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
  if (!meta) meta = templateCacheHooks?.lookup(name);
  if (meta) {
    normalizeTemplate(name, meta);
    noteTemplateRead();
    return templateCacheHooks?.retain(name, meta) ?? meta;
  }
  return meta;
}

/** Return the complete template registry, loading SSR data if needed. */
export function getTemplateRegistry(): Record<string, TemplateMeta> | undefined {
  loadWebUIDataBlock();
  return window.__webui?.templates;
}

/**
 * Register template metadata and optional component styles at runtime.
 *
 * Used by component assets and tests. Registration also lets the dormant-host
 * runtime claim newly available scriptless templates.
 * This accepts trusted compiler output, not user HTML or request state. Under
 * Trusted Types it grants the immutable static HTML permission to enter DOM sinks.
 */
export function registerTemplateData(
  templates: Record<string, TemplateMeta>,
  componentStylesValue?: ComponentStyles | unknown,
): void {
  const componentStyles = prepareComponentStyles(componentStylesValue);
  if (acceptTemplateData(templates, componentStyles).length > 0) {
    // Styles were already accepted above; listeners only need the templates.
    dispatchTemplatesRegistered(templates);
  }
}

function prepareTemplateData(
  templates: Record<string, TemplateMeta>,
): string[] {
  const names = Object.keys(templates);
  for (let i = 0; i < names.length; i++) {
    const name = names[i];
    normalizeTemplate(name, templates[name]);
  }
  return names;
}

/**
 * Prepare compiled component asset metadata before publishing the asset's resources.
 */
export function prepareAssetTemplateData(
  templates: Record<string, TemplateMeta>,
): void {
  prepareTemplateData(templates);
}

/**
 * Hold an authored definition until rendering starts and compiled metadata arrives.
 *
 * Internal to the framework package; the public registration surface remains
 * `TemplateElement.define()`.
 */
export function deferTemplateDefinition(
  tagName: string,
  ctor: Function,
  define: (ctor: Function, tagName: string, meta: TemplateMeta) => void,
): void {
  const pending = pendingTemplateDefinitions.get(tagName);
  if (pending) {
    throw new Error(`[WebUI] <${tagName}> ${pending.ctor === ctor
      ? 'is already pending definition.'
      : 'already has a pending authored definition.'}`);
  }
  pendingTemplateDefinitions.set(tagName, { ctor, define });
}

/**
 * Whether an authored definition for this tag is waiting for rendering or its
 * compiled metadata, so it is declared but not yet a custom element.
 */
export function hasPendingTemplateDefinition(tagName: string): boolean {
  return pendingTemplateDefinitions.has(tagName);
}

function loadWebUIDataBlock(): void {
  if (webuiDataLoaded || window.__webui?.state !== undefined || typeof document === 'undefined') return;
  const el = document.getElementById(WEBUI_DATA_ID);
  const text = el?.textContent;
  let componentStyles: ComponentStyles | undefined;
  if (text) {
    const componentAssetStyles = window.__webui?.componentAssetStyles;
    const parsed = JSON.parse(text) as NonNullable<Window['__webui']>;
    if (window.__webui?.buildId && parsed.buildId && window.__webui.buildId !== parsed.buildId) {
      throw new Error('[WebUI] Resources use different builds. Reload the document.');
    }
    if (componentAssetStyles) parsed.componentAssetStyles = componentAssetStyles;
    // Publish before registering styles. A malformed present `componentStyles` throws,
    // and doing it the other way round loses the templates and state that
    // parsed fine — then re-parses the whole block on the next lookup, because
    // nothing recorded that the work was already done.
    window.__webui = parsed;
    componentStyles = parsed.componentStyles;
  }
  el?.remove();
  webuiDataLoaded = true;
  if (componentStyles !== undefined) registerComponentStyles(componentStyles);
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
  if (!runtime) return;
  delete runtime.state;
  // The installed style bridge owns validated, detached definitions for later
  // mounts and navigation. The one-shot wire catalog is not a second registry.
  if (window.__webuiRegisterComponentStyles) {
    delete runtime.componentStyles;
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

function normalizeTemplate(name: string, meta: TemplateMeta): void {
  if (meta.f && (meta.f !== name || typeof meta.o !== 'string' || meta.sd || meta.th)) {
    throw new Error(`[WebUI] Invalid fragment "${name}". Rebuild matching artifacts.`);
  }
  if (normalizedTemplates.has(meta)) return;
  if (meta.v !== TEMPLATE_META_VERSION) {
    throw new Error(`[WebUI] Unsupported template metadata version for <${name}>. Rebuild matching artifacts.`);
  }
  registerTrustedTemplateBlock(meta);
  const blocks = meta.b;
  if (blocks) for (let i = 0; i < blocks.length; i++) registerTrustedTemplateBlock(blocks[i]);
  normalizedTemplates.add(meta);
}

// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

/**
 * Custom element scripts of lazy regions.
 *
 * `webui build` emits `lazy.webui.js`, which registers one loader per
 * scripted component a region introduces. Applications import it through
 * their own bundler, so each loader is an ordinary dynamic import that the
 * bundler splits, resolves against its public path, and fingerprints.
 */

import type { TemplateMeta } from './template-types.js';
import { hasPendingTemplateDefinition } from './template-registry.js';

// Development tracking is removed from production bundles.
declare const __WEBUI_DEV__: boolean;

/** Loads the module that defines one custom element. */
export type LazyScriptLoader = () => Promise<unknown>;

/** Compiler-owned region dependencies, independent of application loading policy. */
export type LazyRegions = Readonly<Record<string, readonly string[]>>;
/** Application-owned loaders keyed by public custom element tag, not region identity. */
export type LazyScriptLoaders = Readonly<Record<string, LazyScriptLoader>>;

type RegionScripts = readonly [regions: LazyRegions, scripts: LazyScriptLoaders];

const loaders = new Map<string, RegionScripts>();
/** Settled successes are removed; `false` keeps a failed loader retryable even if it declared its tag. */
const loads = new Map<string, Promise<void> | false>();
/** Development only: tags a region asked for, and tags already reported. */
let requested: Set<string> | undefined;
let warned: Set<string> | undefined;
let warnedUnscripted = false;

/** Register generated dependencies with application-owned tag loaders. */
export function addScripts(regions: LazyRegions, scripts: LazyScriptLoaders): void {
  const entry: RegionScripts = [regions, scripts];
  for (const region of Object.keys(regions)) {
    loaders.set(region, entry);
  }
}

/** Load a region's custom elements; `undefined` when every one is already available. */
export function loadScripts(region: string): Promise<unknown> | undefined {
  const entry = loaders.get(region);
  if (!entry) return undefined;
  let waits: Promise<unknown>[] | undefined;
  for (const tag of entry[0][region]) {
    if (typeof __WEBUI_DEV__ === 'undefined' || __WEBUI_DEV__) (requested ??= new Set()).add(tag);
    let loading = loads.get(tag);
    if (loading === undefined && declared(tag)) continue;
    if (!loading) {
      loads.set(tag, loading = startScript(tag, entry[1][tag]).then(
        () => { loads.delete(tag); },
        (error: unknown) => { loads.set(tag, false); throw error; },
      ));
    }
    (waits ??= []).push(loading);
  }
  return waits && Promise.all(waits);
}

async function startScript(tag: string, loader: LazyScriptLoader | undefined): Promise<void> {
  await loader?.();
  if (!declared(tag)) {
    throw new Error(`[WebUI] Await <${tag}>'s module in defineLazyScripts.`);
  }
}

function declared(tag: string): boolean {
  return !!customElements.get(tag) || hasPendingTemplateDefinition(tag);
}

/**
 * Development: report lazy-only tags that a static import declared before any
 * region needed them. A WebUI component whose template has not arrived is
 * declared but not yet defined, which `declared` reports.
 */
export function warnStartupDefinitions(declared: (tag: string) => boolean): void {
  for (const [region, entry] of loaders) {
    for (const tag of entry[0][region]) {
      if (requested?.has(tag) || warned?.has(tag) || !(customElements.get(tag) || declared(tag))) continue;
      (warned ??= new Set()).add(tag);
      console.warn(
        `[WebUI] <${tag}> is only used inside lazy regions but was defined at startup; remove its static import so it loads with its region.`,
      );
    }
  }
}

/**
 * Development: report once that a pack registered scripted components no
 * loader covers and nothing declared, which renders them as empty elements.
 * It happens when an application omitted its default or application loaders.
 */
export function warnUnscriptedComponents(
  region: string,
  templates: Readonly<Record<string, TemplateMeta>>,
  declared: (tag: string) => boolean,
): void {
  if (warnedUnscripted) return;
  let missing = '';
  for (const tag of Object.keys(templates)) {
    // Region bodies and shared blocks are hostless, and scriptless components
    // get compiler-owned hosts.
    if (templates[tag].f || templates[tag].th || customElements.get(tag) || declared(tag)) continue;
    let covered = false;
    for (const [region, entry] of loaders) covered ||= entry[0][region].includes(tag);
    if (!covered) missing += `${missing ? ', ' : ''}<${tag}>`;
  }
  if (!missing) return;
  warnedUnscripted = true;
  console.warn(
    `[WebUI] Lazy region "${region}" registered ${missing}, but no script defines them, so they render empty. Import lazy.webui.js or register application loaders with defineLazyScripts before starting the runtime.`,
  );
}

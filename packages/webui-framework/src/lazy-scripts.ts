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

// Module-local, as in `template-element.ts`. This module has no runtime
// imports, so bundlers inline `DEV` and drop the development-only tracking below.
declare const __WEBUI_DEV__: boolean;
const DEV: boolean = typeof __WEBUI_DEV__ === 'undefined' || __WEBUI_DEV__;

/** Loads the module that defines one custom element. */
export type LazyScriptLoader = () => Promise<unknown>;

/** Loaders of the scripted custom elements each lazy region introduces, by region identity and tag. */
export type LazyScripts = Readonly<Record<string, Readonly<Record<string, LazyScriptLoader>>>>;

const loaders = new Map<string, Record<string, LazyScriptLoader>>();
/** Loads by tag while in flight, then `true`; a failed load is forgotten so the next request retries. */
const loads = new Map<string, Promise<unknown> | true>();
/** Development only: tags a region asked for, and tags already reported. */
let requested: Set<string> | undefined;
let warned: Set<string> | undefined;
let warnedUnscripted = false;

/** Merge generated region loaders into the runtime map. */
export function addScripts(scripts: LazyScripts): void {
  for (const [region, tags] of Object.entries(scripts)) loaders.set(region, { ...loaders.get(region), ...tags });
}

/** Load a region's custom elements; `undefined` when every one is already available. */
export function loadScripts(region: string): Promise<unknown> | undefined {
  const tags = loaders.get(region);
  if (!tags) return undefined;
  let waits: Promise<unknown>[] | undefined;
  for (const [tag, loader] of Object.entries(tags)) {
    if (DEV) (requested ??= new Set()).add(tag);
    let loading = loads.get(tag);
    if (loading === true || customElements.get(tag)) continue;
    if (!loading) {
      loads.set(tag, loading = loader().then(() => { loads.set(tag, true); }, (error: unknown) => {
        loads.delete(tag);
        throw error;
      }));
    }
    (waits ??= []).push(loading);
  }
  return waits && Promise.all(waits);
}

/**
 * Development: report lazy-only tags that a static import declared before any
 * region needed them. A WebUI component whose template has not arrived is
 * declared but not yet defined, which `declared` reports.
 */
export function warnStartupDefinitions(declared: (tag: string) => boolean): void {
  for (const tags of loaders.values()) {
    for (const tag of Object.keys(tags)) {
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
 * It happens when an application imports `lazy.js` instead of `lazy.webui.js`.
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
    for (const tags of loaders.values()) covered ||= !!tags[tag];
    if (!covered) missing += `${missing ? ', ' : ''}<${tag}>`;
  }
  if (!missing) return;
  warnedUnscripted = true;
  console.warn(
    `[WebUI] Lazy region "${region}" registered ${missing}, but no script defines them, so they render empty. Import the generated lazy.webui.js instead of @microsoft/webui-framework/lazy.js.`,
  );
}

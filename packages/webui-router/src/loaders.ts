// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

/**
 * Component loading & route loaders — lazy-loading of component JS
 * modules and resolution of static `loader()` methods on route
 * component constructors.
 */

import type { RouteLoaderContext } from './types.js';
import type { RouteChainEntry } from './cache.js';

/** Sentinel value indicating a loader existed but failed — fall back to server state. */
export const LOADER_FAILED: unique symbol = Symbol('LOADER_FAILED');

/** Shared never-aborted signal for loaders called without an external signal. */
export const NOOP_SIGNAL: AbortSignal = new AbortController().signal;

/**
 * Ensure a component's JS module is loaded. If a lazy loader is
 * configured for this tag and the element isn't already registered,
 * invoke the loader. The promise is cached so each loader runs at
 * most once.
 *
 * When no loader exists and the tag is not yet registered, a passive
 * stub element is auto-defined. This implements the islands
 * architecture pattern: only interactive components need explicit
 * class definitions — passive route targets (pages with no client-side
 * logic) are handled automatically by the framework.
 */
export async function ensureComponentLoaded(
  tag: string,
  loaders: Record<string, () => Promise<unknown>>,
  loaderPromises: Map<string, Promise<void>>,
): Promise<void> {
  if (customElements.get(tag)) return;

  const loader = loaders[tag];
  if (!loader) {
    // No loader and not registered — auto-define a passive stub so
    // the router can create/query this element during SPA navigation.
    definePassiveStub(tag);
    return;
  }

  let promise = loaderPromises.get(tag);
  if (!promise) {
    promise = loader().then(() => {}).finally(() => { loaderPromises.delete(tag); });
    loaderPromises.set(tag, promise);
  }
  await promise;
}

/**
 * Auto-define a passive stub custom element for tags that have no
 * registered class and no lazy loader.  The stub extends HTMLElement
 * directly (no hydration, no template, no bindings) and exposes a
 * no-op `setState()` so the router's `isStateful()` check passes.
 *
 * This is the core of the islands architecture: app code only defines
 * components that need interactivity.  Everything else is server-
 * rendered static HTML with zero client-side overhead.
 */
function definePassiveStub(tag: string): void {
  if (customElements.get(tag)) return;
  customElements.define(tag, class extends HTMLElement {
    setState(_s: Record<string, unknown>): void { /* SSR-only: no-op */ }
  });
}

/**
 * Resolve static `loader()` methods on route component constructors.
 *
 * Called **before** commitNavigation so loader results are available
 * synchronously during the view transition. Components without a
 * static `loader()` are skipped — they use server-provided state.
 *
 * When `ssrBoot` is true (initial SSR navigation with `ssrFresh`),
 * only components whose constructor has `static ssrLoader = true`
 * have their loader invoked. All other components trust the
 * server-rendered state.
 *
 * On failure, the loader is skipped with a warning and the component
 * falls back to server-provided per-entry state.
 */
export async function resolveLoaders(
  chain: RouteChainEntry[],
  query: Record<string, string>,
  signal?: AbortSignal,
  ssrBoot?: boolean,
): Promise<Map<string, Record<string, unknown> | typeof LOADER_FAILED>> {
  const results = new Map<string, Record<string, unknown> | typeof LOADER_FAILED>();

  // Collect only entries that have loaders
  type LoaderEntry = {
    component: string;
    params: Record<string, string>;
    loaderFn: (ctx: RouteLoaderContext) => Promise<Record<string, unknown>>;
  };
  const loaderEntries: LoaderEntry[] = [];
  for (let i = 0; i < chain.length; i++) {
    const entry = chain[i];
    if (!entry.component) continue;

    // During SSR boot, skip loaders unless the component opts in via static ssrLoader
    if (ssrBoot) {
      const ssrCtor = customElements.get(entry.component) as
        (new () => HTMLElement) & { ssrLoader?: boolean } | undefined;
      if (!ssrCtor?.ssrLoader) continue;
    }

    const ctor = customElements.get(entry.component) as (
      (new () => HTMLElement) & { loader?: (ctx: RouteLoaderContext) => Promise<Record<string, unknown>> }
    ) | undefined;
    if (!ctor || typeof ctor.loader !== 'function') continue;

    loaderEntries.push({ component: entry.component, params: entry.params, loaderFn: ctor.loader });
  }

  // Early exit — no loaders in this chain
  if (loaderEntries.length === 0) return results;

  // Create a single fallback signal (not per-task)
  const effectiveSignal = signal ?? NOOP_SIGNAL;

  await Promise.all(loaderEntries.map(async ({ component, params, loaderFn }) => {
    try {
      const state = await loaderFn({ params, query, signal: effectiveSignal });
      if (!effectiveSignal.aborted && state) {
        results.set(component, state);
      }
    } catch (err: unknown) {
      if (err instanceof DOMException && err.name === 'AbortError') return;
      console.warn(
        `[Router] Loader failed for <${component}>, using server state:`,
        err,
      );
      // Mark as failed so callers fall back to server state (not local state)
      results.set(component, LOADER_FAILED);
    }
  }));

  return results;
}

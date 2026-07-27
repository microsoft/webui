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
const NOOP_SIGNAL = new AbortController().signal;

/**
 * Ensure a component's JS module is loaded. If a lazy loader is configured for
 * this tag, invoke it before consulting the registry. This gives authored lazy
 * components precedence over compiler-owned dormant hosts. The promise is
 * cached so each loader runs at most once.
 *
 * When no loader exists, HTML-only components are claimed by the framework's
 * dormant host tier and authored components register themselves.
 */
export async function ensureComponentLoaded(
  tag: string,
  loaders: Record<string, () => Promise<unknown>>,
  loaderPromises: Map<string, Promise<void>>,
): Promise<void> {
  const loader = loaders[tag];
  if (!loader) {
    return;
  }
  if (customElements.get(tag)) return;

  let promise = loaderPromises.get(tag);
  if (!promise) {
    promise = loader()
      .then(() => {})
      .catch((error: unknown) => {
        loaderPromises.delete(tag);
        throw error;
      });
    loaderPromises.set(tag, promise);
  }
  await promise;
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

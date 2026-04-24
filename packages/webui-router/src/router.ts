// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

/**
 * Core router orchestrator — uses the Navigation API to intercept
 * navigations and activates/deactivates `<webui-route>` elements.
 *
 * Heavy lifting is delegated to extracted modules:
 * - cache.ts      — NavigationCache (LRU + tag invalidation)
 * - templates.ts  — template & CSS registration
 * - streaming.ts  — NDJSON streaming reader
 * - actions.ts    — form interception (mutation actions)
 * - pending.ts    — pending/error boundary UI
 * - preload.ts    — speculative prefetch on hover
 * - loaders.ts    — lazy component loading & route loaders
 * - chain.ts      — route chain building & reconciliation
 */

import { buildNavigationTarget, prependBasePath } from './navigation-path.js';
import { isStateful } from './types.js';
import type { RouterConfig, NavigationEvent, CacheConfig } from './types.js';
import type { NavigationTarget } from './navigation-path.js';
import {
  ROUTE_SELECTOR,
  hasState,
  renderRoot,
  routePath,
  isExact,
  routeComponent,
  parseQuery,
  filterQuery,
  activateRoute,
  deactivateRoute,
  applyParamsAndQuery,
  applyParamsQueryState,
  setRouteMeta,
  getRouteMeta,
  WebUIRouteElement,
} from './route-element.js';

import { NavigationCache } from './cache.js';
import type { PartialResponse, RouteChainEntry } from './cache.js';
import { registerTemplatesAndStyles, injectCssLinks, fetchComponentTemplates } from './templates.js';
import { readStreamingPartial, applyDeferredStates } from './streaming.js';
import type { StreamingContext } from './streaming.js';
import { setupFormInterception } from './actions.js';
import { PendingState, findPendingComponent, findErrorComponent } from './pending.js';
import { setupPreloadListeners } from './preload.js';
import { ensureComponentLoaded, resolveLoaders, LOADER_FAILED } from './loaders.js';
import { buildChainFromSSR, findChangeLevel, findOrCreateRouteElement } from './chain.js';

export { parseQuery, filterQuery, WebUIRouteElement };

const SSR_PRELOAD_SELECTOR = 'link[data-webui-ssr-preload]';

export class WebUIRouter {
  private config: RouterConfig = {};
  private started = false;
  private cleanupFns: Array<() => void> = [];
  private isInitialNavigation = true;
  private loaders: Record<string, () => Promise<unknown>> = {};
  private loaderPromises = new Map<string, Promise<void>>();
  private activeChain: RouteChainEntry[] = [];
  private basePath = '';
  /** O(1) lookup sets backed by the global arrays — kept in sync. */
  private cssSet = new Set<string>();
  private stylesSet = new Set<string>();
  private navGeneration = 0;
  private currentRequestPath = '/';
  private navCache!: NavigationCache;
  private cacheConfig: Required<CacheConfig> = { staleTime: 0, gcTime: 300_000, maxEntries: 50 };
  private actionController: AbortController | null = null;
  private deferredReader: Promise<void> | null = null;
  private deferredGeneration = 0;
  private pending = new PendingState();
  private excludePaths: string[] = [];
  private loadPromises = new Map<string, Promise<void>>();
  private ssrPreloadsCleared = false;

  /** The component tag of the currently active leaf route. */
  get activeComponent(): string {
    const leaf = this.activeChain[this.activeChain.length - 1];
    return leaf?.component ?? '';
  }

  /** The bound params of the currently active leaf route. */
  get activeParams(): Record<string, string> {
    const leaf = this.activeChain[this.activeChain.length - 1];
    return leaf?.params ?? {};
  }

  /** Start the router. Lazily registers the `<webui-route>` custom element. */
  start(config: RouterConfig = {}): void {
    if (this.started) return;
    this.started = true;
    this.config = config;
    this.loaders = config.loaders ?? {};
    this.basePath = document.querySelector('base')?.getAttribute('href')?.replace(/\/+$/, '') ?? '';
    this.excludePaths = config.excludePaths ?? [];

    if (config.cache) {
      this.cacheConfig = {
        staleTime: config.cache.staleTime ?? 0,
        gcTime: config.cache.gcTime ?? 300_000,
        maxEntries: config.cache.maxEntries ?? 50,
      };
    }
    this.navCache = new NavigationCache(this.cacheConfig);

    if (!customElements.get(ROUTE_SELECTOR)) {
      customElements.define(ROUTE_SELECTOR, WebUIRouteElement);
    }

    // Normalize window.__webui — ensure it exists with sensible defaults.
    // Serves as the single source of truth for SSR metadata.
    if (!window.__webui) {
      // Legacy server fallback: populate from meta tags / DOM scan
      const inv = document.querySelector('meta[name="webui-inventory"]')?.getAttribute('content') ?? '';
      const nonce = document.querySelector('meta[name="webui-nonce"]')?.getAttribute('content') ?? '';
      const css: string[] = [];
      for (const link of document.querySelectorAll('link[rel="stylesheet"][href]')) {
        css.push(link.getAttribute('href')!);
      }
      const styles: string[] = [];
      for (const style of document.querySelectorAll('style[type="module"][specifier]')) {
        styles.push(style.getAttribute('specifier')!);
      }
      (window as any).__webui = { inventory: inv, nonce, css, styles, templates: {} };
    }
    const meta = window.__webui!;
    // Ensure sub-fields exist
    if (!meta.inventory) meta.inventory = '';
    if (!meta.nonce) meta.nonce = '';
    if (!meta.css) meta.css = [];
    if (!meta.styles) meta.styles = [];
    if (!meta.templates) meta.templates = {};

    // Build O(1) lookup Sets from the global arrays, then free the arrays —
    // they were one-shot SSR data; the Sets are the live lookup structure.
    for (const href of meta.css) this.cssSet.add(href);
    for (const spec of meta.styles) this.stylesSet.add(spec);
    delete meta.css;
    delete meta.styles;

    const nav = window.navigation;
    const handler = (event: NavigateEvent) => {
      if (!event.canIntercept || event.hashChange) return;
      const url = new URL(event.destination.url);
      if (url.origin !== location.origin) return;
      const pathname = url.pathname;
      for (let i = 0; i < this.excludePaths.length; i++) {
        if (pathname.startsWith(this.excludePaths[i])) return;
      }
      event.intercept({
        handler: async () => {
          try {
            await this.handleNavigation(buildNavigationTarget(url, this.basePath), event.signal);
          } catch (err) {
            if (err instanceof DOMException && err.name === 'AbortError') return;
            console.error('[Router] Navigation error:', err);
          }
        },
      });
    };
    nav.addEventListener('navigate', handler);
    this.cleanupFns.push(() => nav.removeEventListener('navigate', handler));

    if (config.preload) {
      const self = this;
      const cleanup = setupPreloadListeners({
        basePath: this.basePath,
        excludePaths: this.excludePaths,
        get currentRequestPath() { return self.currentRequestPath; },
        get inventory() { return window.__webui!.inventory!; },
        hasCache: (p) => this.navCache.has(p),
        storeCache: (p, d, pre) => this.navCache.store(p, d, pre),
        fetchPartial: (p, s, spec) => this.fetchPartial(p, s, spec),
      });
      this.cleanupFns.push(cleanup);
    }

    const selfAction = this;
    this.cleanupFns.push(setupFormInterception({
      get activeChain() { return selfAction.activeChain; },
      get currentRequestPath() { return selfAction.currentRequestPath; },
      setActionController: (c) => { this.actionController = c; },
      invalidateTags: (tags) => this.invalidateTags(tags),
    }));

    this.handleNavigation(this.currentTarget());
  }

  /** Navigate to a new path. */
  navigate(path: string): void {
    const fullPath = prependBasePath(path, this.basePath);
    window.navigation.navigate(fullPath);
  }

  /** Navigate back. */
  back(): void {
    window.navigation.back();
  }

  /** Invalidate all cache entries whose tags overlap with the given tags. */
  invalidateTags(tags: string[]): void {
    this.navCache.invalidateTags(tags);
  }

  /** Invalidate cache entries by path, or all entries if no path is given. */
  invalidate(path?: string): void {
    this.navCache.invalidate(path);
  }

  /**
   * Ensure one or more components' templates + CSS are loaded before use.
   * Batch-fetches missing templates from `/_webui/templates` in a single request.
   */
  async ensureLoaded(...tags: string[]): Promise<void> {
    const registry = window.__webui?.templates;

    const missing: string[] = [];
    for (const tag of tags) {
      if (!registry?.[tag] && !this.loadPromises.has(tag)) {
        missing.push(tag);
      }
    }

    const promises: Promise<void>[] = [];

    if (missing.length > 0) {
      const inv = window.__webui!.inventory!;
      const endpoint = this.config.templateEndpoint ?? '/_webui/templates';
      const fetchPromise = fetchComponentTemplates(
        missing, inv, endpoint, window.__webui!.nonce!, this.stylesSet,
        (inv) => this.updateInventory(inv),
      ).finally(() => {
        for (const tag of missing) this.loadPromises.delete(tag);
      });
      for (const tag of missing) this.loadPromises.set(tag, fetchPromise);
      promises.push(fetchPromise);
    }

    for (const tag of tags) {
      const existing = this.loadPromises.get(tag);
      if (existing) promises.push(existing);
    }

    if (promises.length > 0) await Promise.all(promises);
  }

  /** Garbage-collect all cached templates to free memory. */
  gc(): void {
    const registry = window.__webui?.templates;
    if (registry) {
      for (const tag of Object.keys(registry)) {
        delete registry[tag];
      }
    }
    if (window.__webui) window.__webui.inventory = '';
  }

  /** Tear down. */
  destroy(): void {
    this.loaderPromises.clear();
    this.loadPromises.clear();
    this.loaders = {};
    this.activeChain = [];
    for (const fn of this.cleanupFns) fn();
    this.cleanupFns = [];
    this.started = false;
    this.ssrPreloadsCleared = false;
    this.cssSet.clear();
    this.stylesSet.clear();

    this.currentRequestPath = '/';
    this.navCache.clear();
    this.actionController?.abort();
    this.actionController = null;
    this.pending.destroy();
    this.deferredReader = null;
  }

  // ── Core Navigation ─────────────────────────────────────────────

  private async handleNavigation(target: NavigationTarget, signal?: AbortSignal): Promise<void> {
    const { requestPath } = target;
    this.currentRequestPath = requestPath;
    const query = parseQuery(requestPath);

    if (this.isInitialNavigation) {
      this.isInitialNavigation = false;
      const thisGen = ++this.navGeneration;
      this.activeChain = buildChainFromSSR();
      // Chain was one-shot SSR data — free it now that we've hydrated
      delete window.__webui!.chain;

      await Promise.all(
        this.activeChain
          .filter(entry => entry.component)
          .map(entry => ensureComponentLoaded(entry.component, this.loaders, this.loaderPromises)),
      );
      if (thisGen !== this.navGeneration) return;

      const ssrFresh = this.config.ssrFresh !== false;
      const loaderStates = await resolveLoaders(this.activeChain, query, undefined, ssrFresh);
      if (thisGen !== this.navGeneration) return;

      for (const entry of this.activeChain) {
        const state = loaderStates.get(entry.component);
        if (state && state !== LOADER_FAILED && entry.el) {
          const compEl = entry.compEl ?? entry.el.querySelector(entry.component);
          if (compEl) entry.compEl = compEl;
          if (compEl && isStateful(compEl)) {
            compEl.setState(state);
          }
        }
      }

      // SSR state was consumed by framework $applySSRState() during
      // DOMContentLoaded — free it to reduce memory.
      delete window.__webui!.state;

      if (this.config.dev) {
        this.validateRoutes();
      }
    } else {
      this.clearSsrPreloads();
      this.actionController?.abort();
      this.actionController = null;
      this.pending.clearElements();
      this.navCache.gc();
      const thisGen = ++this.navGeneration;

      let partialData: (PartialResponse & { inventory?: string }) | null = null;
      const cached = this.navCache.lookup(requestPath);
      if (cached) {
        partialData = cached;
      } else {
        const pendingTag = findPendingComponent(this.activeChain, requestPath);
        if (pendingTag) {
          this.pending.pendingTimer = setTimeout(() => {
            this.pending.mountPending(pendingTag, this.activeChain);
          }, 150);
        }

        partialData = await this.fetchPartial(requestPath, signal);
        this.pending.clearTimer();

        if (!partialData && !signal?.aborted && thisGen === this.navGeneration) {
          const errorTag = findErrorComponent(this.activeChain, requestPath);
          if (errorTag) {
            this.pending.mountError(errorTag, {
              error: 'Navigation failed',
              status: 0,
              path: requestPath,
            }, this.activeChain);
            return;
          }
          console.warn('[Router] Navigation fetch failed for:', requestPath);
          return;
        }
      }

      if (!partialData || signal?.aborted || thisGen !== this.navGeneration) return;

      if (partialData.path) {
        const requestPathname = requestPath.split('?')[0];
        if (partialData.path !== requestPathname && partialData.path !== requestPath) {
          console.warn(`[Router] Response path mismatch: expected ${requestPathname}, got ${partialData.path}`);
          return;
        }
      }

      if (!cached) {
        const isStreaming = this.deferredReader !== null && this.deferredGeneration === thisGen;
        this.navCache.store(requestPath, partialData, undefined, isStreaming);
      }

      await this.commitWithData(partialData, requestPath, query, signal, thisGen);

      const deferredStates = (partialData as any)._deferredStates;
      if (deferredStates) {
        applyDeferredStates(deferredStates, requestPath, this.streamingContext());
      }
    }

    const leaf = this.activeChain[this.activeChain.length - 1];
    const detail: NavigationEvent = {
      component: leaf?.component ?? '',
      params: leaf?.params ?? {},
      query,
      path: requestPath,
    };
    window.dispatchEvent(new CustomEvent('webui:route:navigated', { detail }));
  }

  // ── Fetch + Mount ──────────────────────────────────────────────

  private async fetchPartial(
    requestPath: string,
    signal?: AbortSignal,
    speculative?: boolean,
  ): Promise<(PartialResponse & { inventory?: string }) | null> {
    const fullPath = prependBasePath(requestPath, this.basePath);
    const headers: Record<string, string> = { 'Accept': 'application/x-ndjson, application/json' };
    if (window.__webui!.inventory) headers['X-WebUI-Inventory'] = window.__webui!.inventory!;

    const resp = await fetch(fullPath, { headers, signal });
    if (!resp.ok) return null;

    const contentType = resp.headers.get('content-type') ?? '';

    if (!contentType.includes('json') && !contentType.includes('ndjson')) {
      if (speculative || signal?.aborted) return null;
      window.location.href = prependBasePath(requestPath, this.basePath);
      return null;
    }

    if (contentType.includes('ndjson') && resp.body) {
      return readStreamingPartial(resp, requestPath, this.streamingContext(), signal, speculative);
    }

    const data = await resp.json() as PartialResponse & { inventory?: string };
    if (signal?.aborted) return null;
    registerTemplatesAndStyles(data, window.__webui!.nonce!, this.stylesSet, (inv) => this.updateInventory(inv));
    injectCssLinks(data, this.cssSet);
    return data;
  }

  private mountComponent(
    routeEl: HTMLElement,
    componentTag: string,
    params: Record<string, string>,
    state?: Record<string, unknown> | null,
    query?: Record<string, string>,
  ): void {
    const component = document.createElement(componentTag);
    routeEl.textContent = '';
    routeEl.appendChild(component);
    applyParamsQueryState(component, routeEl, params, state, query);
  }

  private applyState(
    entry: RouteChainEntry,
    query?: Record<string, string>,
    loaderStates?: Map<string, Record<string, unknown> | typeof LOADER_FAILED>,
  ): void {
    if (!entry.component || !entry.el) return;
    const compEl = entry.compEl ?? entry.el.querySelector(entry.component);
    if (!compEl) return;
    entry.compEl = compEl;

    const override = loaderStates?.get(entry.component);
    const effectiveOverride = override === LOADER_FAILED ? undefined : override;
    const loaderExists = override !== undefined;
    const isKeepAlive = entry.keepAlive || getRouteMeta(entry.el)?.keepAlive || false;

    if (isKeepAlive) {
      if (effectiveOverride) {
        applyParamsQueryState(compEl, entry.el, entry.params, effectiveOverride, query);
      } else if (loaderExists) {
        applyParamsQueryState(compEl, entry.el, entry.params, entry.state, query);
      } else if (hasState(entry.state)) {
        applyParamsQueryState(compEl, entry.el, entry.params, entry.state, query);
      } else {
        applyParamsAndQuery(compEl, entry.el, entry.params, query);
      }
    } else {
      const stateToApply = effectiveOverride ?? entry.state;
      applyParamsQueryState(compEl, entry.el, entry.params, stateToApply, query);
    }
  }

  // ── Helpers ─────────────────────────────────────────────────────

  private streamingContext(): StreamingContext {
    const self = this;
    return {
      get navGeneration() { return self.navGeneration; },
      get currentRequestPath() { return self.currentRequestPath; },
      get activeChain() { return self.activeChain; },
      get nonce() { return window.__webui!.nonce!; },
      get injectedStyles() { return self.stylesSet; },
      get injectedCss() { return self.cssSet; },
      setDeferredReader(r) { self.deferredReader = r; },
      setDeferredGeneration(g) { self.deferredGeneration = g; },
      updateInventory(inv) { self.updateInventory(inv); },
      markCacheComplete(p) {
        const entry = self.navCache.getEntry(p);
        if (entry) entry.complete = true;
      },
    };
  }

  private validateRoutes(): void {
    for (const { el } of this.activeChain) {
      if (!el) continue;
      const comp = routeComponent(el);
      if (!comp) continue;
      const compEl = el.querySelector(comp);
      if (!compEl) continue;

      const hasOutlet = renderRoot(compEl).querySelector('outlet') !== null;
      const hasChildren = renderRoot(compEl).querySelector(ROUTE_SELECTOR) !== null;
      const path = routePath(el);

      if (!hasChildren && !hasOutlet && !isExact(el) && path !== '/') {
        console.warn(
          `[Router Dev] Route "${path}" (${comp}) is a leaf route without "exact". ` +
          `Add "exact" to prevent unintended prefix matching.`,
        );
      }

      if (hasOutlet && isExact(el)) {
        console.warn(
          `[Router Dev] Route "${path}" (${comp}) has <outlet /> with "exact". ` +
          `Remove "exact" — child routes will never match.`,
        );
      }
    }
  }

  private clearSsrPreloads(): void {
    if (this.ssrPreloadsCleared) return;
    this.ssrPreloadsCleared = true;
    for (const link of document.head.querySelectorAll(SSR_PRELOAD_SELECTOR)) {
      link.remove();
    }
  }

  private currentTarget(): NavigationTarget {
    return buildNavigationTarget(new URL(window.location.href), this.basePath);
  }

  private updateInventory(serverInventory: string): void {
    window.__webui!.inventory = serverInventory;
  }

  // ── Commit ─────────────────────────────────────────────────────

  private async commitWithData(
    partialData: PartialResponse & { inventory?: string },
    requestPath: string,
    query: Record<string, string>,
    signal?: AbortSignal,
    generation?: number,
  ): Promise<void> {
    const topState = partialData.state ?? null;
    const newChain: RouteChainEntry[] = (partialData.chain ?? []).map(e => ({
      component: e.component ?? '',
      path: e.path ?? '',
      params: e.params ?? {},
      exact: e.exact,
      allowedQuery: e.allowedQuery,
      keepAlive: e.keepAlive,
      pendingComponent: e.pendingComponent,
      errorComponent: e.errorComponent,
      invalidates: e.invalidates,
      // Per-entry state (streaming Chunk 2) > top-level state (non-streaming) > null
      state: e.state ?? topState,
    }));

    if (newChain.length === 0) {
      console.warn(`[Router] No route matched for path: ${requestPath}`);
      window.location.href = prependBasePath(requestPath, this.basePath);
      return;
    }

    // Pre-load component modules
    if (signal?.aborted || (generation !== undefined && generation !== this.navGeneration)) return;
    const preload = Promise.all(
      newChain
        .filter(entry => entry.component)
        .map(entry => ensureComponentLoaded(entry.component, this.loaders, this.loaderPromises)),
    );
    if (signal) {
      const aborted = new Promise<'aborted'>(resolve => {
        signal.addEventListener('abort', () => resolve('aborted'), { once: true });
      });
      const result = await Promise.race([preload.then(() => 'loaded' as const), aborted]);
      if (result === 'aborted') return;
    } else {
      await preload;
    }
    if (signal?.aborted || (generation !== undefined && generation !== this.navGeneration)) return;

    // Resolve static loader() methods on component constructors (pre-commit).
    // Loader results replace server state for those components.
    const loaderStates = await resolveLoaders(newChain, query, signal);
    if (signal?.aborted || (generation !== undefined && generation !== this.navGeneration)) return;

    const changeLevel = findChangeLevel(this.activeChain, newChain);
    const isQueryOnlyChange = changeLevel === newChain.length && newChain.length > 0;

    const commitNavigation = (): void => {
      // Deactivate old chain from leaf up
      for (let i = this.activeChain.length - 1; i >= changeLevel; i--) {
        if (this.activeChain[i].el) deactivateRoute(this.activeChain[i].el!);
      }
      for (let i = 0; i < changeLevel; i++) {
        newChain[i].el = this.activeChain[i].el;
        newChain[i].compEl = this.activeChain[i].compEl;
        if (newChain[i].el) {
          setRouteMeta(newChain[i].el!, {
            allowedQuery: newChain[i].allowedQuery,
            keepAlive: newChain[i].keepAlive ?? false,
          });
        }
      }
      if (changeLevel > 0 || isQueryOnlyChange) {
        const end = isQueryOnlyChange ? newChain.length : changeLevel;
        for (let i = 0; i < end; i++) {
          this.applyState(newChain[i], query, loaderStates);
        }
      }
      for (let i = changeLevel; i < newChain.length; i++) {
        const entry = newChain[i];
        const oldEntry = i < this.activeChain.length ? this.activeChain[i] : null;
        const parent = i > 0 ? newChain[i - 1] : null;
        if (oldEntry?.component === entry.component && oldEntry?.el) {
          entry.el = oldEntry.el;
          entry.compEl = oldEntry.compEl;
          setRouteMeta(entry.el, {
            allowedQuery: entry.allowedQuery,
            keepAlive: entry.keepAlive ?? false,
          });
          if (entry.component) this.applyState(entry, query, loaderStates);
          activateRoute(entry.el, entry.params);
          continue;
        }
        const routeEl = findOrCreateRouteElement(parent, entry);
        entry.el = routeEl;
        setRouteMeta(routeEl, {
          allowedQuery: entry.allowedQuery,
          keepAlive: entry.keepAlive ?? false,
        });
        if (entry.component) {
          const override = loaderStates.get(entry.component);
          const effectiveOverride = override === LOADER_FAILED ? undefined : override;

          const isKeepAlive = entry.keepAlive || getRouteMeta(routeEl)?.keepAlive || false;
          const existingComp = routeEl.firstElementChild;
          if (isKeepAlive && existingComp?.matches(entry.component)) {
            entry.compEl = existingComp;
            const stateToApply = effectiveOverride ?? entry.state;
            if (hasState(stateToApply)) {
              applyParamsQueryState(existingComp, routeEl, entry.params, stateToApply, query);
            } else {
              applyParamsAndQuery(existingComp, routeEl, entry.params, query);
            }
          } else {
            const stateToApply = effectiveOverride ?? entry.state;
            this.mountComponent(routeEl, entry.component, entry.params, stateToApply, query);
            entry.compEl = routeEl.firstElementChild ?? undefined;
          }
        }
        activateRoute(routeEl, entry.params);
      }
      this.activeChain = newChain;
    };

    if (document.startViewTransition && !isQueryOnlyChange) {
      const transition = document.startViewTransition(commitNavigation);
      await transition.updateCallbackDone;
    } else {
      commitNavigation();
    }
  }

}

/** Singleton router instance. */
export const Router = new WebUIRouter();

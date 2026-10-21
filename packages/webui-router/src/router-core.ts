// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

/**
 * Core router orchestrator — intercepts navigations and activates/deactivates
 * `<webui-route>` elements. The Navigation API is used when the host grants
 * interception; otherwise `fallback-navigation.ts` reproduces the same contract
 * with `history.pushState` and `popstate`.
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
 * - route-boundary.ts — destination boundary hint selection
 */

import { buildNavigationTarget, prependBasePath } from './navigation-path.js';
import {
  canInterceptNavigations,
  fallbackPush,
  setupFallbackNavigation,
} from './fallback-navigation.js';
import { isStateful } from './types.js';
import type { RouterConfig, NavigationEvent, CacheConfig } from './types.js';
import type { RouterCapabilities } from './capabilities.js';
import { readJsonPartial } from './partial-json.js';
import { acceptInventory, acceptStyleInventory, clearInventory, inventoryEpoch } from './inventory.js';
import { loadRuntimeBootstrap } from './runtime-bootstrap.js';
import type { NavigationTarget } from './navigation-path.js';
import {
  ROUTE_SELECTOR,
  hasState,
  parseQuery,
  filterQuery,
  activateRoute,
  deactivateRoute,
  applyParamsAndQuery,
  applyParamsQueryState,
  setRouteMeta,
  getRouteMeta,
  mountedRouteComponent,
  clearRouteContent,
  WebUIRouteElement,
} from './route-element.js';

import type { PartialResponse, RouteChainEntry } from './cache.js';
import type { PendingState } from './pending.js';
import type {
  PreparedRoutePreload,
  RawPreloadedPartial,
} from './prepared-preload.js';
import {
  fetchComponentTemplates,
  notifyTemplatesRegistered,
  registerInitialTemplatesAndStyles,
  waitForTemplateReadiness,
} from './templates.js';
import type { StreamingContext } from './streaming.js';
import {
  cancelDeferredStream,
  hasDeferredStream,
  startDeferredStream,
  type StreamingPartialResponse,
} from './partial-response.js';
import { ensureComponentLoaded, resolveLoaders, LOADER_FAILED } from './loaders.js';
import {
  buildChainFromSSR,
  findChangeLevel,
  findOrCreateRouteElement,
  sameRouteDeclaration,
} from './chain.js';

export { parseQuery, filterQuery, WebUIRouteElement };

const SSR_PRELOAD_SELECTOR = 'link[data-webui-ssr-preload]';
const PARTIAL_FETCH_TIMEOUT_MS = 10_000;
const DEFAULT_CACHE_CONFIG = { staleTime: 0, gcTime: 300_000, maxEntries: 50 };
const NO_CAPABILITIES: RouterCapabilities = {};

type RouterRuntimeGlobal = WebUIRuntimeGlobal & {
  templates?: Record<string, unknown>;
  templateFns?: Record<string, unknown>;
};

interface PartialReadResult {
  readonly data: StreamingPartialResponse | null;
  readonly hasDeferredReader: boolean;
}

interface PartialReadOptions {
  speculative?: boolean;
  epoch?: number;
}

export class WebUIRouter {
  /** Create a JSON-only router, adding only explicitly imported capabilities. */
  constructor(private readonly capabilities: RouterCapabilities = NO_CAPABILITIES) {}

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
  private navGeneration = 0;
  private currentRequestPath = '/';
  private navCache: import('./cache.js').NavigationCache | null = null;
  private navCacheLoad: Promise<import('./cache.js').NavigationCache> | null = null;
  private preparedPreload: PreparedRoutePreload | null = null;
  private lifecycleGeneration = 0;
  private cacheEnabled = false;
  private cacheConfig: Required<CacheConfig> = DEFAULT_CACHE_CONFIG;
  private actionController: AbortController | null = null;
  private deferredReader: Promise<void> | null = null;
  private deferredGeneration = 0;
  private partialControllers = new Set<AbortController>();
  private partialCleanups = new WeakMap<AbortController, () => void>();
  private boundaryGeneration = 0;
  private pending: PendingState | null = null;
  declare private pendingLoad: Promise<PendingState> | undefined;
  private pendingTimer: ReturnType<typeof setTimeout> | null = null;
  private excludePaths: string[] = [];
  private loadPromises = new Map<string, Promise<void>>();
  private ssrPreloadsCleared = false;
  private documentNavigationUrl: string | null = null;
  /** True when the host grants `NavigateEvent.intercept()`; see fallback-navigation.ts. */
  private nativeInterception = false;
  private startupNavigation: Promise<void> | null = null;

  /** The component tag of the currently active leaf route. */
  get activeComponent(): string {
    const leaf = this.activeChain[this.activeChain.length - 1];
    return leaf?.component ?? '';
  }

  private pendingState(): Promise<PendingState> {
    if (this.pending) return Promise.resolve(this.pending);
    if (this.pendingLoad) return this.pendingLoad;
    const create = this.capabilities.boundaries?.create;
    if (!create) throw new Error('[Router] Route boundaries require the boundaries capability.');
    const generation = this.lifecycleGeneration;
    const load = Promise.resolve().then(() => create()).then(pending => {
      if (generation !== this.lifecycleGeneration) {
        pending.destroy();
        throw new DOMException('Router destroyed during boundary preparation', 'AbortError');
      }
      this.pending = pending;
      return pending;
    });
    this.pendingLoad = load;
    return load.finally(() => {
      if (this.pendingLoad === load) this.pendingLoad = undefined;
    });
  }

  private clearPendingTimer(): void {
    if (this.pendingTimer) {
      clearTimeout(this.pendingTimer);
      this.pendingTimer = null;
    }
  }

  private abortPartialRequests(): void {
    for (const controller of this.partialControllers) {
      controller.abort();
      this.releasePartialRequest(controller);
    }
    this.partialControllers.clear();
  }

  private releasePartialRequest(controller: AbortController): void {
    this.partialControllers.delete(controller);
    this.partialCleanups.get(controller)?.();
    this.partialCleanups.delete(controller);
  }

  private trackPartialRequest(controller: AbortController, cleanup: () => void): void {
    this.partialControllers.add(controller);
    const release = (): void => this.releasePartialRequest(controller);
    controller.signal.addEventListener('abort', release, { once: true });
    this.partialCleanups.set(controller, () => {
      controller.signal.removeEventListener('abort', release);
      cleanup();
    });
  }

  private invalidateBoundaryState(expectedGeneration?: number): number | null {
    if (
      expectedGeneration !== undefined &&
      expectedGeneration !== this.boundaryGeneration
    ) {
      return null;
    }
    const generation = ++this.boundaryGeneration;
    this.clearPendingTimer();
    this.pending?.clearElements();
    return generation === this.boundaryGeneration ? generation : null;
  }

  private destroyPending(): void {
    this.clearPendingTimer();
    this.pending?.destroy();
    this.pending = null;
    this.pendingLoad = undefined;
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
    try {
      this.initialize(config);
    } catch (error) {
      this.destroy();
      throw error;
    }
  }

  private initialize(config: RouterConfig): void {
    this.validateCapabilities(config);
    this.config = config;
    this.loaders = config.loaders ?? {};
    this.basePath = document.querySelector('base')?.getAttribute('href')?.replace(/\/+$/, '') ?? '';
    this.excludePaths = config.excludePaths ?? [];
    this.preparedPreload =
      typeof config.preload === 'object' ? config.preload : null;
    this.preparedPreload?.detach();
    this.cacheEnabled = config.cache !== undefined || !!config.preload;

    if (config.cache) {
      this.cacheConfig = {
        staleTime: config.cache.staleTime ?? 0,
        gcTime: config.cache.gcTime ?? 300_000,
        maxEntries: config.cache.maxEntries ?? 50,
      };
    }
    if (this.cacheEnabled) {
      void this.ensureNavigationCache().catch(error => {
        console.error('[Router] Navigation cache initialization failed:', error);
      });
    }

    if (!customElements.get(ROUTE_SELECTOR)) {
      customElements.define(ROUTE_SELECTOR, WebUIRouteElement);
    }

    loadRuntimeBootstrap();

    // Normalize window.__webui — ensure it exists with sensible defaults.
    // Serves as the single source of truth for SSR metadata.
    if (!window.__webui) window.__webui = {};
    const meta = window.__webui as RouterRuntimeGlobal;
    // Ensure sub-fields exist
    if (!meta.inventory) meta.inventory = '';
    if (!meta.nonce) meta.nonce = '';
    if (!meta.css) meta.css = [];
    if (!meta.styles) meta.styles = [];
    if (!meta.templates) meta.templates = {};
    if (!meta.templateHostExclusions) meta.templateHostExclusions = new Set();
    const loaderTags = Object.keys(this.loaders);
    for (let i = 0; i < loaderTags.length; i++) {
      meta.templateHostExclusions.add(loaderTags[i]);
    }

    this.installDocumentTransitionOverride();

    // Build the router's O(1) CSS URL lookup, then free its source array.
    // Keep `styles`: the framework lazily consumes those SSR Module specifiers
    // when it initializes the per-Document import-map deduplication set.
    for (const href of meta.css) this.cssSet.add(href);
    delete meta.css;

    // The Navigation API cannot intercept navigations on non-HTTP schemes,
    // so desktop shells fall back to pushState/popstate interception.
    this.nativeInterception = canInterceptNavigations();

    if (this.nativeInterception) {
      const nav = window.navigation;
      const handler = (event: NavigateEvent) => {
        if (this.documentNavigationUrl === event.destination.url) {
          this.documentNavigationUrl = null;
          return;
        }
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
    } else {
      const self = this;
      this.cleanupFns.push(setupFallbackNavigation({
        get excludePaths() { return self.excludePaths; },
        navigate: (url, signal) =>
          this.handleNavigation(buildNavigationTarget(url, this.basePath), signal),
      }));
    }

    if (config.preload) {
      const self = this;
      let cleanup: (() => void) | undefined;
      let cancelled = false;
      this.cleanupFns.push(() => {
        cancelled = true;
        cleanup?.();
      });
      void this.ensureNavigationCache().then(async cache => {
        if (cancelled || !this.started) return;
        cleanup = await this.capabilities.preload!({
          basePath: this.basePath,
          excludePaths: this.excludePaths,
          get currentRequestPath() { return self.currentRequestPath; },
          get inventory() { return window.__webui!.inventory!; },
          hasCache: (p) => cache.has(p) || this.preparedPreload?.has(p) === true,
          storeCache: (p, d, pre, streaming) =>
            cache.store(p, d, pre, streaming),
          fetchPartial: (p, s, spec) => this.fetchPartial(p, s, spec),
        });
        if (cancelled || !this.started) cleanup();
      }).catch(error => {
        if (!cancelled) console.error('[Router] Preload initialization failed:', error);
      });
    }

    if (config.actions) {
      const selfAction = this;
      let cleanup: (() => void) | undefined;
      let cancelled = false;
      this.cleanupFns.push(() => {
        cancelled = true;
        cleanup?.();
      });
      void Promise.resolve(this.capabilities.actions!({
        get activeChain() { return selfAction.activeChain; },
        get currentRequestPath() { return selfAction.currentRequestPath; },
        setActionController: (c) => { this.actionController = c; },
        invalidateTags: (tags) => this.invalidateTags(tags),
      })).then(dispose => {
        cleanup = dispose;
        if (cancelled || !this.started) cleanup();
      }).catch(error => {
        if (!cancelled) console.error('[Router] Action initialization failed:', error);
      });
    }

    this.startInitialNavigation(meta);
  }

  private validateCapabilities(config: RouterConfig): void {
    if ((config.cache || config.preload) && !this.capabilities.cache) {
      throw new Error('[Router] Import and provide the cache capability before enabling cache or preload.');
    }
    if (config.preload && !this.capabilities.preload) {
      throw new Error('[Router] Import and provide the preload capability before enabling preload.');
    }
    if (config.actions && !this.capabilities.actions) {
      throw new Error('[Router] Import and provide the actions capability before enabling actions.');
    }
    if (config.dev && !this.capabilities.diagnostics) {
      throw new Error('[Router] Import and provide the diagnostics capability before enabling dev warnings.');
    }
  }

  /** Navigate to a new path. */
  navigate(path: string): void {
    const fullPath = prependBasePath(path, this.basePath);
    if (this.nativeInterception) {
      window.navigation.navigate(fullPath);
    } else {
      fallbackPush(fullPath);
    }
  }

  /** Navigate back. */
  back(): void {
    if (this.nativeInterception) {
      window.navigation.back();
    } else {
      history.back();
    }
  }

  /** Invalidate all cache entries whose tags overlap with the given tags. */
  invalidateTags(tags: string[]): void {
    this.navCache?.invalidateTags(tags);
  }

  /** Invalidate cache entries by path, or all entries if no path is given. */
  invalidate(path?: string): void {
    this.navCache?.invalidate(path);
  }

  /**
   * Ensure one or more components' templates + CSS are loaded before use.
   * Batch-fetches missing templates from `/_webui/templates` in a single request.
   */
  async ensureLoaded(...tags: string[]): Promise<void> {
    const registry = (window.__webui as RouterRuntimeGlobal | undefined)?.templates;

    const missing: string[] = [];
    for (const tag of tags) {
      if (!registry?.[tag] && !window.__webuiRetainTemplate?.(tag) && !this.loadPromises.has(tag)) {
        missing.push(tag);
      }
    }

    const promises: Promise<void>[] = [];

    if (missing.length > 0) {
      const epoch = inventoryEpoch();
      const inv = window.__webui!.inventory!;
      const endpoint = this.config.templateEndpoint ?? '/_webui/templates';
      const fetchPromise = fetchComponentTemplates(
        missing,
        inv,
        endpoint,
        window.__webui!.nonce!,
        this.cssSet,
        (inv, styles) => this.updateInventory(inv, epoch, styles),
      ).finally(() => {
        for (const tag of missing) {
          if (this.loadPromises.get(tag) === fetchPromise) this.loadPromises.delete(tag);
        }
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
    const collect = window.__webuiCollectTemplates;
    if (collect) {
      const keep: string[] = [];
      for (const entry of this.activeChain) {
        if (entry.pendingComponent) keep.push(entry.pendingComponent);
        if (entry.errorComponent) keep.push(entry.errorComponent);
      }
      collect(keep);
      this.navCache?.clear();
      return;
    }
    const runtime = window.__webui as RouterRuntimeGlobal | undefined;
    const registry = runtime?.templates;
    if (registry) {
      for (const template of Object.values(registry)) {
        if (template && typeof template === 'object') {
          throw new Error(
            '[Router] Template collection requires an ownership-aware cache adapter. ' +
            "Import your renderer's template-cache entry before component definitions.",
          );
        }
      }
      for (const tag of Object.keys(registry)) {
        delete registry[tag];
      }
    }
    const functionRegistry = runtime?.templateFns;
    if (functionRegistry) {
      for (const tag of Object.keys(functionRegistry)) {
        delete functionRegistry[tag];
      }
    }
    clearInventory();
    this.navCache?.clear();
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
    this.documentNavigationUrl = null;
    this.nativeInterception = false;
    this.startupNavigation = null;
    this.cssSet.clear();

    this.currentRequestPath = '/';
    this.navCache?.clear();
    this.navCache = null;
    this.navCacheLoad = null;
    this.preparedPreload?.destroy();
    this.preparedPreload = null;
    this.lifecycleGeneration += 1;
    this.cacheEnabled = false;
    this.cacheConfig = DEFAULT_CACHE_CONFIG;
    this.actionController?.abort();
    this.actionController = null;
    this.navGeneration++;
    this.invalidateBoundaryState();
    this.destroyPending();
    this.abortPartialRequests();
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
        const state = loaderStates.get(entry);
        if (state && state !== LOADER_FAILED && entry.el) {
          const compEl = entry.compEl ?? mountedRouteComponent(entry.el, entry.component);
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
        this.capabilities.diagnostics?.(this.activeChain);
      }
    } else {
      this.clearSsrPreloads();
      this.actionController?.abort();
      this.actionController = null;
      const thisGen = ++this.navGeneration;
      const boundaryGen = this.invalidateBoundaryState();
      if (boundaryGen === null) return;

      try {
        if (thisGen !== this.navGeneration) return;
        this.abortPartialRequests();
        const navCache = this.cacheEnabled ? await this.ensureNavigationCache() : null;
        if (thisGen !== this.navGeneration) return;
        navCache?.gc();

        let partialData: StreamingPartialResponse | null = null;
        const cached = navCache?.lookup(requestPath) ?? null;
        if (cached) {
          partialData = cached;
        } else {
          const pendingBoundary = this.capabilities.boundaries?.pending(this.activeChain, requestPath);
          if (pendingBoundary && !pendingBoundary.keepAlive) {
            this.pendingTimer = setTimeout(() => {
              if (boundaryGen !== this.boundaryGeneration) return;
              this.pendingTimer = null;
              void this.pendingState().then((pending) => {
                if (
                  thisGen === this.navGeneration &&
                  boundaryGen === this.boundaryGeneration
                ) {
                  pending.mountPending(
                    pendingBoundary.component,
                    pendingBoundary.container,
                  );
                }
              }).catch(error => {
                if (error instanceof DOMException && error.name === 'AbortError') return;
                console.error('[Router] Pending UI preparation failed:', error);
              });
            }, 150);
          }

          partialData =
            await this.takePreparedPartial(requestPath, signal)
            ?? await this.fetchPartial(requestPath, signal);

          if (!partialData && !signal?.aborted && thisGen === this.navGeneration) {
            const errorBoundary = this.capabilities.boundaries?.error(this.activeChain, requestPath);
            if (errorBoundary) {
              const errorBoundaryGen = this.invalidateBoundaryState(boundaryGen);
              if (errorBoundaryGen === null) return;
              const pending = await this.pendingState();
              if (
                thisGen !== this.navGeneration ||
                errorBoundaryGen !== this.boundaryGeneration
              ) {
                return;
              }
              pending.mountError(
                errorBoundary.component,
                {
                  error: 'Navigation failed',
                  status: 0,
                  path: requestPath,
                },
                errorBoundary.container,
              );
              return;
            }
            console.warn('[Router] Navigation fetch failed for:', requestPath);
            return;
          }
        }

        if (!partialData || signal?.aborted || thisGen !== this.navGeneration) {
          this.invalidateBoundaryState(boundaryGen);
          if (partialData) {
            await cancelDeferredStream(partialData);
          }
          this.preparedPreload?.release(requestPath);
          return;
        }

        if (partialData.path) {
          const requestPathname = requestPath.split('?')[0];
          if (partialData.path !== requestPathname && partialData.path !== requestPath) {
            console.warn(`[Router] Response path mismatch: expected ${requestPathname}, got ${partialData.path}`);
            this.invalidateBoundaryState(boundaryGen);
            await cancelDeferredStream(partialData);
            this.preparedPreload?.release(requestPath);
            return;
          }
        }

        if (!cached) {
          const isStreaming = hasDeferredStream(partialData);
          navCache?.store(requestPath, partialData, undefined, isStreaming);
          this.preparedPreload?.release(requestPath);
        }

        let committed = false;
        try {
          committed = await this.commitWithData(
            partialData,
            requestPath,
            query,
            thisGen,
            boundaryGen,
            signal,
          );
        } catch (error) {
          this.invalidateBoundaryState(boundaryGen);
          await cancelDeferredStream(partialData);
          if (!cached) navCache?.evict(requestPath);
          throw error;
        }
        if (!committed || signal?.aborted || thisGen !== this.navGeneration) {
          this.invalidateBoundaryState(boundaryGen);
          await cancelDeferredStream(partialData);
          if (!cached) navCache?.evict(requestPath);
          return;
        }
        startDeferredStream(partialData);
      } finally {
        this.invalidateBoundaryState(boundaryGen);
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
  ): Promise<StreamingPartialResponse | null> {
    const epoch = inventoryEpoch();
    const fullPath = prependBasePath(requestPath, this.basePath);
    const headers: Record<string, string> = {
      Accept: this.capabilities.streaming ? 'application/x-ndjson, application/json' : 'application/json',
    };
    if (window.__webui!.inventory) headers['X-WebUI-Inventory'] = window.__webui!.inventory!;

    const requestController = new AbortController();
    if (!speculative) this.abortPartialRequests();
    const timeout = setTimeout(
      () => requestController.abort(new DOMException('Partial response timed out', 'TimeoutError')),
      PARTIAL_FETCH_TIMEOUT_MS,
    );
    this.trackPartialRequest(requestController, () => clearTimeout(timeout));
    const requestSignal = signal
      ? AbortSignal.any([signal, requestController.signal])
      : requestController.signal;
    let hasDeferredReader = false;

    try {
      const resp = await fetch(fullPath, { headers, signal: requestSignal });
      const result = await this.readPartialResponse(
        resp,
        requestPath,
        requestController,
        requestSignal,
        { speculative, epoch },
      );
      hasDeferredReader = result.hasDeferredReader;
      return result.data;
    } catch (error) {
      if (signal?.aborted) throw error;
      return null;
    } finally {
      if (!hasDeferredReader) this.releasePartialRequest(requestController);
    }
  }

  private async takePreparedPartial(
    requestPath: string,
    signal?: AbortSignal,
  ): Promise<StreamingPartialResponse | null> {
    const epoch = inventoryEpoch();
    const raw = await this.preparedPreload?.take(
      requestPath,
      window.__webui!.inventory!,
      signal,
    );
    if (!raw) return null;

    const requestController = new AbortController();
    const timeout = setTimeout(
      () => requestController.abort(new DOMException('Partial response timed out', 'TimeoutError')),
      PARTIAL_FETCH_TIMEOUT_MS,
    );
    const requestSignal = signal
      ? AbortSignal.any([signal, requestController.signal])
      : requestController.signal;
    const abortRaw = (): void => raw.controller.abort(requestSignal.reason);
    this.trackPartialRequest(requestController, () => {
      // The owning controller can abort before AbortSignal.any dispatches.
      if (requestController.signal.aborted) raw.controller.abort(requestController.signal.reason);
      clearTimeout(timeout);
      requestSignal.removeEventListener('abort', abortRaw);
    });
    if (requestSignal.aborted) {
      abortRaw();
    } else {
      requestSignal.addEventListener('abort', abortRaw, { once: true });
    }
    let hasDeferredReader = false;
    try {
      const result = await this.readPartialResponse(
        rawResponse(raw),
        requestPath,
        requestController,
        requestSignal,
        { speculative: true, epoch },
      );
      hasDeferredReader = result.hasDeferredReader;
      if (!result.data) this.preparedPreload?.release(requestPath);
      return result.data;
    } catch (error) {
      this.preparedPreload?.release(requestPath);
      if (signal?.aborted) throw error;
      return null;
    } finally {
      if (!hasDeferredReader) this.releasePartialRequest(requestController);
    }
  }

  private async readPartialResponse(
    response: Response,
    requestPath: string,
    requestController: AbortController,
    signal: AbortSignal,
    options: PartialReadOptions = {},
  ): Promise<PartialReadResult> {
    if (!response.ok) {
      requestController.abort();
      return { data: null, hasDeferredReader: false };
    }
    const contentType = response.headers.get('content-type') ?? '';
    if (!contentType.includes('json') && !contentType.includes('ndjson')) {
      if (!options.speculative && !signal.aborted) this.navigateDocument(requestPath);
      requestController.abort();
      return { data: null, hasDeferredReader: false };
    }
    if (contentType.includes('ndjson')) {
      const streaming = this.capabilities.streaming;
      if (!streaming) {
        requestController.abort();
        throw new Error('[Router] The server returned NDJSON to a JSON-only router. Select the streaming capability or return JSON.');
      }
      const data = await streaming.read(
        response,
        requestPath,
        this.streamingContext(requestController, options.epoch),
        signal,
      );
      return {
        data,
        hasDeferredReader:
          data !== null && hasDeferredStream(data),
      };
    }

    const data = await readJsonPartial(response, this.streamingContext(requestController, options.epoch), signal);
    return {
      data,
      hasDeferredReader: false,
    };
  }

  private mountComponent(
    routeEl: HTMLElement,
    componentTag: string,
    params: Record<string, string>,
    state?: Record<string, unknown> | null,
    query?: Record<string, string>,
  ): void {
    // Destroy existing component bindings before clearing DOM
    const existing = mountedRouteComponent(routeEl);
    if (existing && typeof (existing as unknown as { $destroy?: () => void }).$destroy === 'function') {
      (existing as unknown as { $destroy: () => void }).$destroy();
    }
    const component = document.createElement(componentTag);
    clearRouteContent(routeEl);
    applyParamsQueryState(component, routeEl, params, state, query);
    routeEl.appendChild(component);
  }

  private applyState(
    entry: RouteChainEntry,
    query?: Record<string, string>,
    loaderStates?: Map<RouteChainEntry, Record<string, unknown> | typeof LOADER_FAILED>,
  ): void {
    if (!entry.component || !entry.el) return;
    const compEl = entry.compEl ?? mountedRouteComponent(entry.el, entry.component);
    if (!compEl) return;
    entry.compEl = compEl;

    const override = loaderStates?.get(entry);
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

  private streamingContext(requestController?: AbortController, epoch = inventoryEpoch()): StreamingContext {
    const self = this;
    let trackedReader: Promise<void> | null = null;
    return {
      get navGeneration() { return self.navGeneration; },
      get currentRequestPath() { return self.currentRequestPath; },
      get activeChain() { return self.activeChain; },
      get nonce() { return window.__webui!.nonce!; },
      get injectedCss() { return self.cssSet; },
      setDeferredReader(r) {
        if (r) {
          trackedReader = r;
          self.deferredReader = r;
          return;
        }
        if (self.deferredReader === trackedReader) self.deferredReader = null;
        if (requestController) self.releasePartialRequest(requestController);
      },
      setDeferredGeneration(g) { self.deferredGeneration = g; },
      updateInventory(inv, styles) { self.updateInventory(inv, epoch, styles); },
      markCacheComplete(p) {
        if (requestController?.signal.aborted) return;
        const entry = self.navCache?.getEntry(p);
        if (entry) entry.complete = true;
      },
    };
  }

  private ensureNavigationCache(): Promise<import('./cache.js').NavigationCache> {
    if (this.navCache) return Promise.resolve(this.navCache);
    if (!this.navCacheLoad) {
      const config = this.cacheConfig;
      const generation = this.lifecycleGeneration;
      const create = this.capabilities.cache;
      if (!create) return Promise.reject(new Error('[Router] Navigation caching requires the cache capability.'));
      const load = Promise.resolve().then(() => create(config)).then(cache => {
        if (generation === this.lifecycleGeneration) {
          this.navCache = cache;
        }
        return cache;
      });
      this.navCacheLoad = load;
      void load.catch(() => {
        if (this.navCacheLoad === load) this.navCacheLoad = null;
      });
    }
    return this.navCacheLoad;
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

  private updateInventory(serverInventory: string, epoch = inventoryEpoch(), styles?: string): void {
    acceptInventory(serverInventory, epoch);
    if (styles !== undefined) acceptStyleInventory(styles, epoch);
  }

  private navigateDocument(requestPath: string): void {
    const href = prependBasePath(requestPath, this.basePath);
    this.documentNavigationUrl = new URL(href, window.location.href).href;
    if (window.location.href === this.documentNavigationUrl) {
      window.location.reload();
    } else {
      window.location.href = href;
    }
  }

  private installDocumentTransitionOverride(): void {
    const transitions = this.capabilities.transitions;
    if (transitions) this.cleanupFns.push(transitions.install());
  }

  private startInitialNavigation(meta: RouterRuntimeGlobal): void {
    if (meta.componentStyles) {
      registerInitialTemplatesAndStyles(meta.templates ?? {}, meta.componentStyles);
    } else {
      notifyTemplatesRegistered(meta.templates);
    }
    const startup = this.handleNavigation(this.currentTarget());
    const startupGeneration = this.navGeneration;
    this.startupNavigation = startup;
    void startup.then(
      () => {
        if (this.startupNavigation === startup) {
          this.startupNavigation = null;
        }
      },
      (error) => {
        if (this.startupNavigation !== startup) return;
        this.startupNavigation = null;
        if (this.navGeneration !== startupGeneration) return;
        this.destroy();
        console.error('[Router] Initial navigation error:', error);
      },
    );
  }

  // ── Commit ─────────────────────────────────────────────────────

  private async commitWithData(
    partialData: PartialResponse & { inventory?: string },
    requestPath: string,
    query: Record<string, string>,
    navigationGeneration: number,
    boundaryGeneration: number,
    signal?: AbortSignal,
  ): Promise<boolean> {
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
      this.navigateDocument(requestPath);
      return false;
    }

    // Pre-load component modules
    if (signal?.aborted || navigationGeneration !== this.navGeneration) {
      return false;
    }
    const preload = Promise.all(
      newChain
        .filter(entry => entry.component)
        .map(entry => ensureComponentLoaded(entry.component, this.loaders, this.loaderPromises)),
    );
    if (!await waitForTemplateReadiness(preload, signal)) return false;
    if (signal?.aborted || navigationGeneration !== this.navGeneration) {
      return false;
    }

    // A component claimed by neither an authored module nor the framework's
    // dormant template-host runtime cannot be mounted safely.
    if (newChain.some(entry => entry.component && !customElements.get(entry.component))) {
      this.navigateDocument(requestPath);
      return false;
    }

    // Resolve static loader() methods on component constructors (pre-commit).
    // Loader results replace server state for those components.
    const loaderStates = await resolveLoaders(newChain, query, signal);
    if (signal?.aborted || navigationGeneration !== this.navGeneration) {
      return false;
    }

    const changeLevel = findChangeLevel(this.activeChain, newChain);
    const isQueryOnlyChange = changeLevel === newChain.length && newChain.length > 0;

    let committed = false;
    const commitNavigation = (): void => {
      if (
        signal?.aborted ||
        navigationGeneration !== this.navGeneration ||
        this.invalidateBoundaryState(boundaryGeneration) === null
      ) {
        return;
      }

      // Deactivate old chain from leaf up. Preserve route-owned style markers,
      // but tear down component state when the declaration will not be reused.
      for (let i = this.activeChain.length - 1; i >= changeLevel; i--) {
        const oldEntry = this.activeChain[i];
        if (oldEntry.el) {
          const newEntry = i < newChain.length ? newChain[i] : undefined;
          const willReuse = newEntry !== undefined && sameRouteDeclaration(oldEntry, newEntry);
          if (!willReuse) {
            const isKeepAlive = oldEntry.keepAlive || getRouteMeta(oldEntry.el)?.keepAlive || false;
            if (!isKeepAlive) {
              const existing = oldEntry.compEl ?? mountedRouteComponent(oldEntry.el);
              if (existing && typeof (existing as unknown as { $destroy?: () => void }).$destroy === 'function') {
                (existing as unknown as { $destroy: () => void }).$destroy();
              }
              clearRouteContent(oldEntry.el);
            }
          }
          deactivateRoute(oldEntry.el);
        }
        oldEntry.compEl = undefined; // Release component reference
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
        if (oldEntry?.el && sameRouteDeclaration(oldEntry, entry)) {
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
          const override = loaderStates.get(entry);
          const effectiveOverride = override === LOADER_FAILED ? undefined : override;

          const isKeepAlive = entry.keepAlive || getRouteMeta(routeEl)?.keepAlive || false;
          const existingComp = mountedRouteComponent(routeEl);
          if (isKeepAlive && existingComp?.localName === entry.component.toLowerCase()) {
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
            entry.compEl = mountedRouteComponent(routeEl) ?? undefined;
          }
        }
        activateRoute(routeEl, entry.params);
      }
      this.activeChain = newChain;
      committed = true;
    };

    if (this.capabilities.transitions?.supported() && !isQueryOnlyChange) {
      const transition = this.capabilities.transitions.startViewTransition(commitNavigation);
      // Animation skips are non-fatal; updateCallbackDone still owns commit errors.
      void transition.ready.catch(ignoreTransitionRejection);
      void transition.finished.catch(ignoreTransitionRejection);
      await transition.updateCallbackDone;
    } else {
      commitNavigation();
    }
    return committed;
  }

}

function ignoreTransitionRejection(): void {}

function rawResponse(raw: RawPreloadedPartial): Response {
  return raw.response;
}

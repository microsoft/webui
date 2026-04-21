// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

/**
 * Core router — uses the Navigation API to intercept navigations and
 * activates/deactivates `<webui-route>` elements in the DOM tree.
 *
 * Supports **nested routes**: routes inside matched route components are
 * resolved relative to their parent's consumed path. Parent content persists
 * when only child routes change.
 *
 * For routes with a `component` attribute, the router fetches a JSON
 * partial from the server (state + f-templates), registers any new
 * templates, instantiates the component, and mounts it into the route.
 */

import { buildNavigationTarget, prependBasePath, stripBaseFromPathname } from './navigation-path.js';
import type { RouterConfig, NavigationEvent, CacheConfig, RouteActionContext, RouteActionResult, ActionCompleteEvent } from './types.js';
import type { NavigationTarget } from './navigation-path.js';

const ROUTE_SELECTOR = 'webui-route';
const SSR_PRELOAD_SELECTOR = 'link[data-webui-ssr-preload]';

/** Sentinel value indicating a loader existed but failed — fall back to server state. */
const LOADER_FAILED = Symbol('LOADER_FAILED');

/** Shared never-aborted signal for loaders called without an external signal. */
const NOOP_SIGNAL = new AbortController().signal;

/** Maximum buffered NDJSON line size before aborting the stream (256 KiB). */
const MAX_NDJSON_BUFFER = 256 * 1024;

/**
 * Check if a state value is meaningful (non-null, non-empty).
 * Returns false for null, undefined, or `{}`.
 */
function hasState(state?: Record<string, unknown> | null): state is Record<string, unknown> {
  if (state == null) return false;
  const keys = Object.keys(state);
  return keys.length > 0;
}

/**
 * Get the render root of a component element.
 * Returns shadowRoot if present, otherwise the element itself.
 * This allows the router to work in both shadow and light DOM modes.
 */
function renderRoot(el: Element): Element | ShadowRoot {
  return (el as HTMLElement).shadowRoot ?? el;
}

/** Create a hidden `<webui-route>` stub element. */
function createRouteStub(entry: { path?: string; component?: string; exact?: boolean }): HTMLElement {
  const el = document.createElement(ROUTE_SELECTOR);
  if (entry.path) el.setAttribute('path', entry.path);
  if (entry.component) el.setAttribute('component', entry.component);
  if (entry.exact) el.setAttribute('exact', '');
  el.style.display = 'none';
  return el;
}

// ── Route element helpers ────────────────────────────────────────

function routePath(el: Element): string {
  return el.getAttribute('path') ?? '';
}

function isExact(el: Element): boolean {
  return el.hasAttribute('exact');
}

function routeComponent(el: Element): string {
  return el.getAttribute('component') ?? '';
}

/** Type-safe route param storage — avoids expando properties on DOM elements. */
const routeParamsMap = new WeakMap<Element, Record<string, string>>();

/**
 * Tracks which query-param attribute names (kebab-case) were last applied to
 * each component element. Used to remove stale attributes when query params
 * change (e.g. navigating from `?subject=foo` to no `subject`).
 */
const queryAttrsMap = new WeakMap<Element, Set<string>>();

/** Cached parsed query allowlist per route element — avoids re-splitting on every navigation. */
const allowedQueryCache = new WeakMap<Element, Set<string> | null>();

/** Convert a camelCase key to a kebab-case attribute name. */
const toKebab = (k: string): string => k.replace(/[A-Z]/g, m => `-${m.toLowerCase()}`);

/** Parse query-string parameters from a request path (e.g. `/compose?action=reply&to=x`). */
export function parseQuery(requestPath: string): Record<string, string> {
  const qIdx = requestPath.indexOf('?');
  if (qIdx < 0) return {};
  const query: Record<string, string> = {};
  const params = new URLSearchParams(requestPath.slice(qIdx));
  for (const [k, v] of params) {
    query[k] = v;
  }
  return query;
}

/**
 * Read the comma-separated `query` allowlist attribute from a `<route>` element.
 * Returns null if no `query` attribute is present (deny-by-default).
 */
function routeAllowedQuery(el: Element): Set<string> | null {
  const cached = allowedQueryCache.get(el);
  if (cached !== undefined) return cached;
  const raw = el.getAttribute('query');
  if (raw == null) {
    allowedQueryCache.set(el, null);
    return null;
  }
  const set = new Set<string>();
  for (const part of raw.split(',')) {
    const trimmed = part.trim();
    if (trimmed) set.add(trimmed);
  }
  allowedQueryCache.set(el, set);
  return set;
}

/**
 * Filter query params through an allowlist. Returns only key-value pairs
 * whose keys appear in `allowed`. If `allowed` is null (no `query` attr
 * on the route), returns an empty object (deny-by-default).
 *
 * Keys whose kebab-case form collides with a route param's kebab-case
 * form are always excluded so that path parameters cannot be overridden
 * via query string.
 */
export function filterQuery(
  query: Record<string, string>,
  allowed: Set<string> | null,
  routeParams?: Record<string, string>,
): Record<string, string> {
  if (!allowed) return {};
  // Build a set of kebab-cased route param attribute names for collision check
  let paramAttrNames: Set<string> | undefined;
  if (routeParams) {
    paramAttrNames = new Set<string>();
    const rpKeys = Object.keys(routeParams);
    for (let i = 0; i < rpKeys.length; i++) paramAttrNames.add(toKebab(rpKeys[i]));
  }
  const result: Record<string, string> = {};
  const qKeys = Object.keys(query);
  for (let i = 0; i < qKeys.length; i++) {
    const k = qKeys[i];
    if (allowed.has(k) && !(paramAttrNames && paramAttrNames.has(toKebab(k)))) {
      result[k] = query[k];
    }
  }
  return result;
}

function activateRoute(el: HTMLElement, params: Record<string, string>): void {
  routeParamsMap.set(el, params);
  el.setAttribute('active', '');
  el.style.display = '';
}

function deactivateRoute(el: HTMLElement): void {
  routeParamsMap.set(el, {});
  el.removeAttribute('active');
  el.style.display = 'none';
}

function getRouteParams(el: Element): Record<string, string> {
  return routeParamsMap.get(el) ?? {};
}

// ── WebUIRouteElement custom element ─────────────────────────────

/** Custom element backing `<webui-route>`. */
export class WebUIRouteElement extends HTMLElement {
  get path(): string { return this.getAttribute('path') ?? ''; }
  get exact(): boolean { return this.hasAttribute('exact'); }
  get component(): string { return this.getAttribute('component') ?? ''; }
  get isActive(): boolean { return this.hasAttribute('active'); }
  get keepAlive(): boolean { return this.hasAttribute('keep-alive'); }
  get params(): Record<string, string> { return getRouteParams(this); }
  /** Comma-separated allowlist of query params forwarded as attributes. */
  get query(): string { return this.getAttribute('query') ?? ''; }
}

// ── Route Chain Types ────────────────────────────────────────────

/** An entry in the matched route chain, one per nesting level. */
interface RouteChainEntry {
  /** Component tag name for this route level. */
  component: string;
  /** Route path pattern as declared in the template. */
  path: string;
  /** Bound route parameters at this level. */
  params: Record<string, string>;
  /** Whether this route requires an exact match. */
  exact?: boolean;
  /** DOM element, populated during mount or SSR chain build. */
  el?: HTMLElement;
  /** Comma-separated allowlist of query params forwarded as attributes. */
  allowedQuery?: string;
  /** When true, the component is preserved across navigations instead of re-created. */
  keepAlive?: boolean;
  /** Component tag for pending/loading UI. */
  pendingComponent?: string;
  /** Component tag for error boundary UI. */
  errorComponent?: string;
  /** Invalidation tags from the build-time proto (already resolved with params). */
  invalidates?: string[];
  /**
   * Per-component state from the server.
   * - `undefined` → skip setState (preserve component's current state)
   * - `null` → skip setState (preserve component's current state)
   * - `{...}` → call setState with this data
   */
  state?: Record<string, unknown> | null;
}

/** Static route manifest entry — built once at startup from <webui-route> tree. */
interface RouteManifestEntry {
  component: string;
  path: string;
  exact: boolean;
  allowedQuery?: string;
  children: RouteManifestEntry[];
}

// ── Router ───────────────────────────────────────────────────────

/** JSON partial response from the server. */
interface PartialResponse {
  /** Top-level application state (non-streaming responses). */
  state?: Record<string, unknown>;
  /** Module CSS definitions to append before executing template scripts. */
  templateStyles?: string[];
  templates: string[];
  path: string;
  chain?: RouteChainEntry[];
  /** CSS stylesheet URLs to inject into `<head>` for this route's components. */
  css?: string[];
  /** Resolved cache tags for this route chain (union of all levels). */
  cacheTags?: string[];
  /** Server-provided cache control overrides. */
  cacheControl?: { staleTime?: number };
}

/** A single entry in the navigation cache. */
interface CacheEntry {
  /** The full partial response data. */
  data: PartialResponse & { inventory?: string };
  /** Cache tags associated with this entry (from server response). */
  tags: string[];
  /** Timestamp when this entry was stored. */
  ts: number;
  /** Server-provided stale time override (ms), or undefined to use config default. */
  staleTime?: number;
  /** True if this entry came from a speculative preload fetch. */
  preload?: boolean;
  /** Whether both streaming chunks have been received. */
  complete: boolean;
}

/**
 * Apply route params and query params as HTML attributes on a component.
 * Does NOT call setState — used for keep-alive reactivation where local
 * state should be preserved. Stale query-param attributes from a previous
 * navigation are automatically removed.
 */
function applyParamsAndQuery(
  component: Element,
  routeEl: HTMLElement,
  params: Record<string, string>,
  query?: Record<string, string>,
): void {
  const paramKeys = Object.keys(params);
  for (let i = 0; i < paramKeys.length; i++) {
    component.setAttribute(toKebab(paramKeys[i]), params[paramKeys[i]]);
  }

  const allowed = routeAllowedQuery(routeEl);
  if (!allowed || !query) {
    // Fast path: no query params to process — just clean up stale attrs
    const prevAttrs = queryAttrsMap.get(component);
    if (prevAttrs) {
      for (const attr of prevAttrs) component.removeAttribute(attr);
      queryAttrsMap.delete(component);
    }
    return;
  }

  const filtered = filterQuery(query, allowed, params);
  const newAttrs = new Set<string>();
  const filteredKeys = Object.keys(filtered);
  for (let i = 0; i < filteredKeys.length; i++) {
    const key = filteredKeys[i];
    const attr = toKebab(key);
    component.setAttribute(attr, filtered[key]);
    newAttrs.add(attr);
  }

  const prevAttrs = queryAttrsMap.get(component);
  if (prevAttrs) {
    for (const attr of prevAttrs) {
      if (!newAttrs.has(attr)) {
        component.removeAttribute(attr);
      }
    }
  }
  if (newAttrs.size > 0) {
    queryAttrsMap.set(component, newAttrs);
  } else {
    queryAttrsMap.delete(component);
  }
}

/**
 * Apply route params, allowed query params, and state to a component.
 * Shared by both initial mount and subsequent state updates. Stale query-param
 * attributes from a previous navigation are automatically removed.
 *
 * For keep-alive reactivation without a loader, use {@link applyParamsAndQuery}
 * instead — it updates attributes without overwriting component state.
 */
function applyParamsQueryState(
  component: Element,
  routeEl: HTMLElement,
  params: Record<string, string>,
  state?: Record<string, unknown> | null,
  query?: Record<string, string>,
): void {
  applyParamsAndQuery(component, routeEl, params, query);

  if (hasState(state) && typeof (component as any).setState === 'function') {
    (component as any).setState(state);
  }
}

export class WebUIRouter {
  private config: RouterConfig = {};
  private started = false;
  private cleanupFns: Array<() => void> = [];
  private isInitialNavigation = true;
  /** Comma-separated list tracking which component templates are loaded. */
  private inventory = '';
  /** CSP nonce read from `<meta name="webui-nonce">` — used for dynamic script creation. */
  private nonce = '';
  /** Opt-in lazy loaders: component tag → async import function. */
  private loaders: Record<string, () => Promise<unknown>> = {};
  /** Deduplication cache for in-flight / completed loader promises. */
  private loaderPromises = new Map<string, Promise<void>>();
  /** Current active route chain for reconciliation on next navigation. */
  private activeChain: RouteChainEntry[] = [];
  /** Cached base path from config (avoids repeated nullish coalescing). */
  private basePath = '';
  /** In-memory dedup for injected CSS link hrefs (avoids DOM queries). */
  private injectedCss = new Set<string>();
  /** In-memory dedup for injected module style specifiers (avoids DOM queries). */
  private injectedStyles = new Set<string>();
  /** Monotonic navigation generation — guards against stale async completions. */
  private navGeneration = 0;

  /** Cached current request path — updated on every navigation. Avoids URL allocation in hot paths. */
  private currentRequestPath = '/';
  /** Tagged navigation cache — keyed by requestPath. */
  private cache = new Map<string, CacheEntry>();
  /** Reverse index: tag → set of cache keys. Enables O(1) tag invalidation. */
  private tagIndex = new Map<string, Set<string>>();
  /** Cache configuration (staleTime/gcTime/maxEntries). */
  private cacheConfig: Required<CacheConfig> = { staleTime: 0, gcTime: 300_000, maxEntries: 50 };
  /** AbortController for the in-flight preload fetch. */
  private preloadController: AbortController | null = null;
  /** Monotonic preload generation — prevents stale hover fetches from overwriting the cache. */
  private preloadGeneration = 0;
  /** Pending UI timer handle — cleared on commit or abort. */
  private pendingTimer: ReturnType<typeof setTimeout> | null = null;
  /** AbortController for the in-flight mutation action. */
  private actionController: AbortController | null = null;
  /** In-flight deferred state reader from streaming Chunk 2. */
  private deferredReader: Promise<void> | null = null;
  /** Generation at which the deferred reader was started. */
  private deferredGeneration = 0;
  /** Direct reference to mounted pending element for O(1) cleanup. */
  private pendingElement: HTMLElement | null = null;
  /** Direct reference to mounted error element for O(1) cleanup. */
  private errorElement: HTMLElement | null = null;

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
    this.basePath = config.basePath ?? '';

    if (config.cache) {
      this.cacheConfig = {
        staleTime: config.cache.staleTime ?? 0,
        gcTime: config.cache.gcTime ?? 300_000,
        maxEntries: config.cache.maxEntries ?? 50,
      };
    }

    if (!customElements.get(ROUTE_SELECTOR)) {
      customElements.define(ROUTE_SELECTOR, WebUIRouteElement);
    }

    this.inventory = document.querySelector('meta[name="webui-inventory"]')?.getAttribute('content') ?? '';
    this.nonce = document.querySelector('meta[name="webui-nonce"]')?.getAttribute('content') ?? '';

    // Seed dedup sets from SSR-injected elements
    for (const link of document.querySelectorAll('link[rel="stylesheet"][href]')) {
      this.injectedCss.add(link.getAttribute('href')!);
    }
    for (const style of document.querySelectorAll('style[type="module"][specifier]')) {
      this.injectedStyles.add(style.getAttribute('specifier')!);
    }

    const nav = window.navigation;
    const handler = (event: NavigateEvent) => {
      if (!event.canIntercept || event.hashChange) return;
      const url = new URL(event.destination.url);
      if (url.origin !== location.origin) return;
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
      this.setupPreloadListeners();
    }

    this.setupFormInterception();

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

  /**
   * Invalidate all cache entries whose tags overlap with the given tags.
   * Use after mutations to ensure stale data is evicted.
   */
  invalidateTags(tags: string[]): void {
    if (tags.length === 0) return;
    const pathsToEvict = new Set<string>();
    for (const tag of tags) {
      const paths = this.tagIndex.get(tag);
      if (paths) {
        for (const path of paths) pathsToEvict.add(path);
      }
    }
    for (const path of pathsToEvict) {
      this.evictCacheEntry(path);
    }
  }

  /**
   * Invalidate cache entries by path, or all entries if no path is given.
   * Escape hatch for cases where tag-based invalidation isn't sufficient.
   */
  invalidate(path?: string): void {
    if (path) {
      this.evictCacheEntry(path);
    } else {
      for (const key of [...this.cache.keys()]) {
        this.evictCacheEntry(key);
      }
    }
  }

  /** In-flight ensureLoaded promises — deduplicates concurrent requests. */
  private loadPromises = new Map<string, Promise<void>>();

  /**
   * Ensure one or more components' templates + CSS are loaded before use.
   * For non-route components (dialogs, overlays) that need their template
   * and CSS registered before the element is created — avoids FOUC.
   *
   * Batch-fetches missing templates from `/_webui/templates` in a single
   * request. Reuses the same template/style registration pipeline as
   * partial navigation.
   *
   * @example
   * ```ts
   * await Router.ensureLoaded('settings-dialog');
   * await Router.ensureLoaded('modal-a', 'modal-b');
   * await Router.ensureLoaded(...componentList);
   * ```
   */
  async ensureLoaded(...tags: string[]): Promise<void> {
    const registry = window.__webui_templates;

    // Split into already-registered vs missing
    const missing: string[] = [];
    for (const tag of tags) {
      if (!registry?.[tag] && !this.loadPromises.has(tag)) {
        missing.push(tag);
      }
    }

    const promises: Promise<void>[] = [];

    // Batch-fetch missing templates in one request
    if (missing.length > 0) {
      const inv = this.inventory;
      const fetchPromise = this.fetchComponentTemplates(missing, inv).finally(() => {
        for (const tag of missing) this.loadPromises.delete(tag);
      });
      for (const tag of missing) this.loadPromises.set(tag, fetchPromise);
      promises.push(fetchPromise);
    }

    // Wait for any in-flight requests from previous calls
    for (const tag of tags) {
      const existing = this.loadPromises.get(tag);
      if (existing) promises.push(existing);
    }

    if (promises.length > 0) await Promise.all(promises);
  }

  /**
   * Fetch component templates + CSS from the server and register them.
   * Reuses the same registration logic as fetchPartial.
   * Throws on network or server errors so callers can handle failures.
   */
  private async fetchComponentTemplates(tags: string[], inventoryHex: string): Promise<void> {
    const endpoint = this.config.templateEndpoint ?? '/_webui/templates';
    const url = `${endpoint}?t=${tags.join(',')}&inv=${encodeURIComponent(inventoryHex)}`;
    const resp = await fetch(url);
    if (!resp.ok) {
      throw new Error(`[Router] ensureLoaded failed: ${resp.status} ${resp.statusText}`);
    }
    const data = await resp.json();

    // Register using the same pipeline as partial navigation
    this.registerTemplatesAndStyles(data);
  }

  /**
   * Register templates + inject CSS from a server response.
   * Shared by fetchPartial and fetchComponentTemplates.
   */
  private registerTemplatesAndStyles(data: {
    templates?: string[];
    templateStyles?: string[];
    inventory?: string;
  }): void {
    if (data.inventory) {
      this.updateInventory(data.inventory);
    }

    // 1. Module CSS: inject <style type="module"> definitions into <head>
    if (data.templateStyles) {
      for (const styleMarkup of data.templateStyles) {
        const trimmed = styleMarkup.trim();
        if (!trimmed.startsWith('<style')) continue;

        const openTagEnd = trimmed.indexOf('>');
        const closeTagStart = trimmed.lastIndexOf('</style>');
        if (openTagEnd < 0 || closeTagStart <= openTagEnd) continue;

        const specifierToken = 'specifier="';
        const specStart = trimmed.indexOf(specifierToken);
        let specifier: string | null = null;
        if (specStart >= 0) {
          const valStart = specStart + specifierToken.length;
          const valEnd = trimmed.indexOf('"', valStart);
          if (valEnd > valStart) specifier = trimmed.substring(valStart, valEnd);
        }

        if (specifier && this.injectedStyles.has(specifier)) {
          continue;
        }

        const style = document.createElement('style');
        style.type = 'module';
        if (specifier) {
          style.setAttribute('specifier', specifier);
          this.injectedStyles.add(specifier);
        }
        style.textContent = trimmed.substring(openTagEnd + 1, closeTagStart);
        document.head.appendChild(style);
      }
    }

    // 2. Template registration: execute JS IIFEs / insert DOM templates.
    //    TRUST BOUNDARY: template scripts come from the same-origin server
    //    that compiled the protocol. The CSP nonce gates script execution.
    //    If the server endpoint is compromised, this is an XSS vector —
    //    same risk as the existing fetchPartial pipeline.
    if (data.templates) {
      let scriptBody = '';
      for (const tmpl of data.templates) {
        if (tmpl.startsWith('<')) {
          const container = document.createDocumentFragment();
          const temp = document.createElement('div');
          temp.innerHTML = tmpl;
          while (temp.firstChild) container.appendChild(temp.firstChild);
          document.body.appendChild(container);
        } else {
          if (scriptBody) scriptBody += '\n';
          scriptBody += tmpl;
        }
      }
      if (scriptBody) {
        const script = document.createElement('script');
        if (this.nonce) script.nonce = this.nonce;
        script.textContent = scriptBody;
        document.head.appendChild(script);
        document.head.removeChild(script);
      }
    }
  }

  /**
   * Garbage-collect all cached templates to free memory. Clears every entry
   * from `window.__webui_templates` and resets the inventory so the server
   * re-sends needed templates on the next navigation.
   */
  gc(): void {
    const registry = window.__webui_templates;
    if (registry) {
      for (const tag of Object.keys(registry)) {
        delete registry[tag];
      }
    }
    this.inventory = '';
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
    this.injectedCss.clear();
    this.injectedStyles.clear();

    this.currentRequestPath = '/';
    this.cache.clear();
    this.tagIndex.clear();
    this.preloadController?.abort();
    this.preloadController = null;
    this.actionController?.abort();
    this.actionController = null;
    if (this.pendingTimer) {
      clearTimeout(this.pendingTimer);
      this.pendingTimer = null;
    }
    this.pendingElement = null;
    this.errorElement = null;
    this.deferredReader = null;
  }

  // ── Navigation Cache ──────────────────────────────────────────

  /** Maximum age (ms) for a preloaded partial before it's considered stale. */
  private static readonly PRELOAD_TTL = 5_000;

  /** Look up a cache entry. Returns null if missing, stale, or incomplete. */
  private lookupCache(requestPath: string): (PartialResponse & { inventory?: string }) | null {
    const entry = this.cache.get(requestPath);
    if (!entry || !entry.complete) return null;

    const age = Date.now() - entry.ts;
    const staleTime = entry.staleTime ?? this.cacheConfig.staleTime;

    // Preload entries get a minimum 5s freshness window; normal entries use staleTime as-is
    const effectiveStaleTime = entry.preload ? Math.max(staleTime, WebUIRouter.PRELOAD_TTL) : staleTime;
    if (age > effectiveStaleTime) {
      return null; // Stale — let handleNavigation refetch
    }

    // LRU: delete + reinsert to move to end (most recently used)
    this.cache.delete(requestPath);
    this.cache.set(requestPath, entry);
    return entry.data;
  }

  /** Store a partial response in the cache with its tags. */
  private storeCacheEntry(
    requestPath: string,
    data: PartialResponse & { inventory?: string },
    preload?: boolean,
    streaming?: boolean,
  ): void {
    const tags = data.cacheTags ?? [];
    const staleTime = data.cacheControl?.staleTime;

    // Clean up old tag-index references before overwriting
    this.evictCacheEntry(requestPath);

    // Evict LRU entries if at capacity
    while (this.cache.size >= this.cacheConfig.maxEntries) {
      const oldest = this.cache.keys().next().value;
      if (oldest !== undefined) {
        this.evictCacheEntry(oldest);
      } else {
        break;
      }
    }

    this.cache.set(requestPath, {
      data, tags, ts: Date.now(), staleTime, preload,
      complete: !preload && !streaming,
    });

    // Build reverse tag index
    for (const tag of tags) {
      let paths = this.tagIndex.get(tag);
      if (!paths) {
        paths = new Set();
        this.tagIndex.set(tag, paths);
      }
      paths.add(requestPath);
    }
  }

  /** Evict a single cache entry and clean up its tag index references. */
  private evictCacheEntry(requestPath: string): void {
    const entry = this.cache.get(requestPath);
    if (!entry) return;
    this.cache.delete(requestPath);
    for (const tag of entry.tags) {
      const paths = this.tagIndex.get(tag);
      if (paths) {
        paths.delete(requestPath);
        if (paths.size === 0) this.tagIndex.delete(tag);
      }
    }
  }

  /** Run GC: evict entries older than gcTime. */
  private gcCache(): void {
    const now = Date.now();
    const gcTime = this.cacheConfig.gcTime;
    for (const [path, entry] of this.cache) {
      if (now - entry.ts > gcTime) {
        this.evictCacheEntry(path);
      }
    }
  }

  // ── Form Interception (Mutation Actions) ──────────────────────

  /**
   * Set up delegated form submission interception.
   * Intercepts `<form method="post">` submissions, finds the nearest
   * route component's `static action()`, and auto-invalidates cache.
   */
  private setupFormInterception(): void {
    const onSubmit = (e: SubmitEvent): void => {
      // Walk composedPath to find the form — works across shadow boundaries
      const path = e.composedPath();
      let form: HTMLFormElement | undefined;
      for (let i = 0; i < path.length; i++) {
        const el = path[i] as Element;
        if (el?.tagName === 'FORM' && (el as HTMLFormElement).method?.toLowerCase() === 'post') {
          form = el as HTMLFormElement;
          break;
        }
      }
      if (!form) return;

      // Only intercept forms without an explicit action or targeting same-origin.
      // Forms with external action URLs (payment, auth, etc.) must not be hijacked.
      const formAction = form.action; // resolved absolute URL
      if (formAction) {
        try {
          const actionUrl = new URL(formAction);
          if (actionUrl.origin !== location.origin) return;
        } catch {
          return; // malformed action — don't intercept
        }
      }
      // Forms with a target attribute submit to a different browsing context
      if (form.target && form.target !== '_self') return;

      // Find the nearest ancestor <webui-route> with a component
      let routeEl: HTMLElement | null = null;
      for (let i = 0; i < path.length; i++) {
        const el = path[i] as Element;
        if (el?.tagName === 'WEBUI-ROUTE' && el.getAttribute('component')) {
          routeEl = el as HTMLElement;
          break;
        }
      }
      if (!routeEl) return;

      const componentTag = routeEl.getAttribute('component');
      if (!componentTag) return;

      // Check if the component has a static action() method
      const ctor = customElements.get(componentTag) as (
        (new () => HTMLElement) & { action?: (ctx: RouteActionContext) => Promise<RouteActionResult | void> }
      ) | undefined;
      if (!ctor || typeof ctor.action !== 'function') return;

      // Prevent default form submission
      e.preventDefault();

      const formData = new FormData(form);
      const params = getRouteParams(routeEl);
      const controller = new AbortController();
      this.actionController = controller;

      // Get resolved invalidation tags from the active chain entry
      // (not from DOM attr which has unresolved {param} templates)
      const chainEntry = this.activeChain.find(e => e.component === componentTag);
      const routeInvalidates = chainEntry?.invalidates ?? [];

      ctor.action({ formData, params, signal: controller.signal })
        .then((result: RouteActionResult | void) => {
          if (controller.signal.aborted) return;

          // Apply optimistic state if provided
          if (result?.state) {
            const compEl = routeEl!.querySelector(componentTag!) as any;
            if (compEl && typeof compEl.setState === 'function') {
              compEl.setState(result.state);
            }
          }

          // Merge action-returned tags with route's build-time invalidates
          const allTags = new Set<string>();
          for (const tag of routeInvalidates) allTags.add(tag);
          if (result?.invalidateTags) {
            for (const tag of result.invalidateTags) allTags.add(tag);
          }
          const mergedTags = [...allTags];

          // Invalidate cache
          if (mergedTags.length > 0) {
            this.invalidateTags(mergedTags);
          }

          // Dispatch completion event
          const detail: ActionCompleteEvent = {
            component: componentTag!,
            invalidatedTags: mergedTags,
            path: this.currentRequestPath,
          };
          window.dispatchEvent(new CustomEvent('webui:route:action-complete', { detail }));
        })
        .catch((err: unknown) => {
          if (err instanceof DOMException && err.name === 'AbortError') return;
          console.error(`[Router] Action failed for <${componentTag}>:`, err);
        });
    };

    document.addEventListener('submit', onSubmit);
    this.cleanupFns.push(() => document.removeEventListener('submit', onSubmit));
  }

  // ── Pending UI ────────────────────────────────────────────────

  /** Clear the pending UI timer. */
  private clearPendingTimer(): void {
    if (this.pendingTimer) {
      clearTimeout(this.pendingTimer);
      this.pendingTimer = null;
    }
  }

  /**
   * Find the pending component for a target route.
   * Walks SSR'd `<webui-route>` stubs looking for ones whose path
   * could match the target, preferring the deepest match.
   */
  private findPendingComponent(requestPath: string): string | null {
    // Check active chain (parent routes that already have metadata from partial)
    for (let i = this.activeChain.length - 1; i >= 0; i--) {
      if (this.activeChain[i].pendingComponent) {
        return this.activeChain[i].pendingComponent!;
      }
    }
    // Walk SSR'd route stubs scoped to the deepest active leaf's children
    const leaf = this.activeChain[this.activeChain.length - 1];
    if (leaf?.el) {
      const compEl = leaf.el.querySelector(leaf.component);
      if (compEl) {
        const root = (compEl as HTMLElement).shadowRoot ?? compEl;
        for (const el of root.querySelectorAll(ROUTE_SELECTOR)) {
          const pending = el.getAttribute('pending');
          if (pending) return pending;
        }
      }
    }
    return null;
  }

  /**
   * Find the error component for a target route.
   * Same scoping strategy as findPendingComponent.
   */
  private findErrorComponent(requestPath: string): string | null {
    for (let i = this.activeChain.length - 1; i >= 0; i--) {
      if (this.activeChain[i].errorComponent) {
        return this.activeChain[i].errorComponent!;
      }
    }
    const leaf = this.activeChain[this.activeChain.length - 1];
    if (leaf?.el) {
      const compEl = leaf.el.querySelector(leaf.component);
      if (compEl) {
        const root = (compEl as HTMLElement).shadowRoot ?? compEl;
        for (const el of root.querySelectorAll(ROUTE_SELECTOR)) {
          const error = el.getAttribute('error');
          if (error) return error;
        }
      }
    }
    return null;
  }

  /** Remove any pending/error elements left over from a previous navigation. */
  private clearPendingElements(): void {
    if (this.pendingElement) {
      this.pendingElement.remove();
      this.pendingElement = null;
    }
    if (this.errorElement) {
      this.errorElement.remove();
      this.errorElement = null;
    }
  }

  /**
   * Find the target route's `<webui-route>` element in the DOM.
   * Searches hidden stubs for a route matching the component tag.
   */
  private findTargetRouteElement(componentTag: string): HTMLElement | null {
    // Search SSR'd route stubs for the target component
    for (const el of document.querySelectorAll(ROUTE_SELECTOR)) {
      if (el.getAttribute('component') === componentTag) {
        return el as HTMLElement;
      }
    }
    return null;
  }

  /**
   * Mount a pending/loading component in the outlet area.
   * Finds the target route's parent (deepest active leaf) and appends
   * the pending component in its outlet container.
   */
  private mountPendingComponent(componentTag: string): void {
    // Find the deepest active route's component, then look for the
    // outlet area to mount the pending component.
    const leaf = this.activeChain[this.activeChain.length - 1];
    if (!leaf?.el) return;

    // Don't show pending for keep-alive routes (they activate instantly)
    if (leaf.keepAlive) return;

    const existing = leaf.el.querySelector(componentTag);
    if (existing) return; // Already showing

    // Mount inside the leaf's component's outlet area (where child routes go)
    const compEl = leaf.el.querySelector(leaf.component);
    if (!compEl) return;

    const root = (compEl as HTMLElement).shadowRoot ?? compEl;

    // Find existing sibling route elements or an outlet marker
    const siblingRoutes = root.querySelectorAll(ROUTE_SELECTOR);
    const container = siblingRoutes.length > 0
      ? siblingRoutes[siblingRoutes.length - 1].parentElement
      : (root.querySelector('outlet')?.parentElement ?? root);
    if (!container) return;

    const pending = document.createElement(componentTag);
    pending.setAttribute('data-webui-pending', '');
    container.appendChild(pending);
    this.pendingElement = pending;
  }

  /**
   * Mount an error boundary component in the outlet area.
   * Passes error details as state.
   */
  private mountErrorComponent(
    componentTag: string,
    errorState: { error: string; status: number; path: string },
  ): void {
    const leaf = this.activeChain[this.activeChain.length - 1];
    if (!leaf?.el) return;

    const compEl = leaf.el.querySelector(leaf.component);
    if (!compEl) return;

    const root = (compEl as HTMLElement).shadowRoot ?? compEl;

    // Find existing sibling route elements or an outlet marker
    const siblingRoutes = root.querySelectorAll(ROUTE_SELECTOR);
    const container = siblingRoutes.length > 0
      ? siblingRoutes[siblingRoutes.length - 1].parentElement
      : (root.querySelector('outlet')?.parentElement ?? root);
    if (!container) return;

    // Hide all existing route children
    for (const child of container.querySelectorAll(ROUTE_SELECTOR)) {
      (child as HTMLElement).style.display = 'none';
    }

    const errorEl = document.createElement(componentTag);
    errorEl.setAttribute('data-webui-error', '');
    container.appendChild(errorEl);
    this.errorElement = errorEl;
    if (typeof (errorEl as any).setState === 'function') {
      (errorEl as any).setState(errorState);
    }
  }

  /**
   * Register a delegated `pointerover` listener that speculatively fetches
   * the JSON partial for internal links on mouse hover.
   *
   * Uses `pointermove` (bubbles + composed + fires continuously) because
   * `pointerover` only fires once when entering a shadow host — subsequent
   * moves between child elements inside the shadow root don't re-trigger it
   * at the document level. `pointermove` fires on every position change,
   * giving us reliable detection across all shadow DOM boundaries.
   *
   * The handler is naturally debounced: the cache lookup
   * ensures we only fetch once per unique link, and the early returns
   * for non-anchor targets keep the hot path fast (one `composedPath()`
   * walk that exits immediately when no `<a>` is found).
   *
   * Only mouse pointers trigger preload — touch fires too late to benefit.
   */
  private setupPreloadListeners(): void {
    const onPointerMove = (e: PointerEvent): void => {
      if (e.pointerType !== 'mouse') return;

      // Walk composedPath to find the nearest <a> — works across shadow boundaries.
      // Use a for-loop instead of .find() to avoid closure allocation.
      const path = e.composedPath();
      let anchor: HTMLAnchorElement | undefined;
      for (let i = 0; i < path.length; i++) {
        if ((path[i] as Element)?.tagName === 'A') {
          anchor = path[i] as HTMLAnchorElement;
          break;
        }
      }
      if (!anchor) return;

      // Use anchor's pre-parsed URL properties to avoid new URL() allocation.
      const href = anchor.getAttribute('href');
      if (!href || href.startsWith('#')) return;
      if (anchor.origin !== location.origin) return;

      // Build request path from anchor properties — no URL allocation needed.
      const stripped = stripBaseFromPathname(anchor.pathname, this.basePath);
      const requestPath = (stripped + anchor.search) || '/';

      // Skip if already on this path or already cached for it
      if (requestPath === this.currentRequestPath) return;
      if (this.cache.has(requestPath)) return;

      // Abort any in-flight speculative fetch and start a new one
      this.preloadController?.abort();
      const controller = new AbortController();
      this.preloadController = controller;
      const gen = ++this.preloadGeneration;

      this.fetchPartial(requestPath, controller.signal, true)
        .then(data => {
          // Only cache if this is still the latest preload request
          if (data && gen === this.preloadGeneration && !controller.signal.aborted) {
            this.storeCacheEntry(requestPath, data, true);
          }
        })
        .catch(() => {}); // Speculative — silently discard errors
    };

    document.addEventListener('pointermove', onPointerMove);
    this.cleanupFns.push(() => document.removeEventListener('pointermove', onPointerMove));
  }

  // ── Route matching ──────────────────────────────────────────────

  /**
   * Core navigation handler — called on initial load and every client-side navigation.
   *
   * On initial load: builds the active chain from SSR'd `<webui-route active>` elements.
   * On subsequent navigations: fetches a JSON partial from the server (which includes
   * the matched route chain), diffs against the current chain to find the first changed
   * level, deactivates old routes from the leaf up, and mounts new components from the
   * change level down. Parent components above the change level are preserved and
   * receive fresh state from the server.
   */
  private async handleNavigation(target: NavigationTarget, signal?: AbortSignal): Promise<void> {
    const { requestPath } = target;
    this.currentRequestPath = requestPath;
    const query = parseQuery(requestPath);

    if (this.isInitialNavigation) {
      // Set the flag BEFORE any async work — if a new navigation
      // supersedes this one (user clicks a folder while components
      // are loading), the new navigation must NOT take the initial
      // SSR-bootstrap path again.
      this.isInitialNavigation = false;
      this.activeChain = this.buildChainFromSSR();
      await Promise.all(
        this.activeChain
          .filter(entry => entry.component)
          .map(entry => this.ensureComponentLoaded(entry.component)),
      );

      // Run static loaders for SSR-bootstrapped components.
      // This ensures SSR and SPA navigations use the same data source
      // when a component defines a loader.
      const loaderStates = await this.resolveLoaders(this.activeChain, query);
      for (const entry of this.activeChain) {
        const state = loaderStates.get(entry.component);
        if (state && entry.el) {
          const compEl = entry.el.querySelector(entry.component);
          if (compEl && typeof (compEl as any).setState === 'function') {
            (compEl as any).setState(state);
          }
        }
      }

      if (this.config.dev) {
        this.validateRoutes();
      }
    } else {
      this.clearSsrPreloads();
      this.actionController?.abort();
      this.actionController = null;
      this.clearPendingElements();
      this.gcCache();
      const thisGen = ++this.navGeneration;

      // Check unified cache for a fresh hit
      let partialData: (PartialResponse & { inventory?: string }) | null = null;
      const cached = this.lookupCache(requestPath);
      if (cached) {
        partialData = cached;
      } else {
        // Cancel any in-flight speculative fetch — we're doing a real navigation
        this.preloadController?.abort();

        // Pending UI: start a timer. If the fetch takes longer than 150ms,
        // mount the pending component (if the target route declares one).
        const pendingTag = this.findPendingComponent(requestPath);
        if (pendingTag) {
          this.pendingTimer = setTimeout(() => {
            this.mountPendingComponent(pendingTag);
          }, 150);
        }

        partialData = await this.fetchPartial(requestPath, signal);
        this.clearPendingTimer();

        // Error boundary: if fetch returned null (HTTP error), mount error component
        if (!partialData && !signal?.aborted && thisGen === this.navGeneration) {
          const errorTag = this.findErrorComponent(requestPath);
          if (errorTag) {
            this.mountErrorComponent(errorTag, {
              error: 'Navigation failed',
              status: 0,
              path: requestPath,
            });
            return;
          }
          console.warn('[Router] Navigation fetch failed for:', requestPath);
          return;
        }
      }

      if (!partialData || signal?.aborted || thisGen !== this.navGeneration) return;

      // Verify response pathname matches request to prevent cache poisoning.
      // Compare only the pathname (before '?') since the server path omits query strings.
      if (partialData.path) {
        const requestPathname = requestPath.split('?')[0];
        if (partialData.path !== requestPathname && partialData.path !== requestPath) {
          console.warn(`[Router] Response path mismatch: expected ${requestPathname}, got ${partialData.path}`);
          return;
        }
      }

      // Store in cache with tags
      if (!cached) {
        // Streaming entries are incomplete until Chunk 2 finishes (has _deferredStates or active reader)
        const isStreaming = this.deferredReader !== null && this.deferredGeneration === thisGen;
        this.storeCacheEntry(requestPath, partialData, undefined, isStreaming);
      }

      await this.commitWithData(partialData, requestPath, query, signal, thisGen);

      // Apply deferred states from streaming Chunk 2 (if both chunks arrived together)
      const deferredStates = (partialData as any)._deferredStates;
      if (deferredStates) {
        this.applyDeferredStates(deferredStates, requestPath);
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

  private ssrPreloadsCleared = false;

  private clearSsrPreloads(): void {
    if (this.ssrPreloadsCleared) return;
    this.ssrPreloadsCleared = true;
    for (const link of document.head.querySelectorAll(SSR_PRELOAD_SELECTOR)) {
      link.remove();
    }
  }

  /**
   * Find or create a `<webui-route>` DOM element for a chain entry.
   * For top-level routes, searches direct children of `<body>`.
   * For nested routes, searches the parent component's render root
   * (shadow root or light DOM).
   *
   * When creating a new stub, placement strategy:
   *  1. Sibling routes: insert next to existing `<webui-route>` elements
   *     (handles SSR'd shadow roots where `<outlet>` was replaced by routes).
   *  2. `<outlet>` marker: insert after it (handles freshly created components
   *     whose f-template still contains `<outlet></outlet>`).
   *  3. Fallback: append to render root.
   */
  private findOrCreateRouteElement(
    parent: RouteChainEntry | null,
    entry: RouteChainEntry,
  ): HTMLElement {
    // For top-level routes, search direct children of body
    if (!parent) {
      for (const child of document.body.children) {
        if (child.tagName === 'WEBUI-ROUTE' &&
            child.getAttribute('component') === entry.component) {
          return child as HTMLElement;
        }
      }
      const el = createRouteStub(entry);
      document.body.appendChild(el);
      return el;
    }

    // For nested routes, search in parent component's render root
    if (parent.el) {
      const compEl = parent.el.querySelector(parent.component);
      if (compEl) {
        const root = renderRoot(compEl);
        const allRoutes = root.querySelectorAll(ROUTE_SELECTOR);

        // Match by component + path for stronger identity
        for (const child of allRoutes) {
          if (child.getAttribute('component') === entry.component &&
              child.getAttribute('path') === (entry.path || null)) {
            return child as HTMLElement;
          }
        }
        // Fallback: match by component only (backwards compat)
        for (const child of allRoutes) {
          if (child.getAttribute('component') === entry.component) {
            return child as HTMLElement;
          }
        }

        // Not found — create stub and place in the correct container
        const stub = createRouteStub(entry);

        // Strategy 1: use the parent container of existing sibling routes.
        // In SSR'd shadow roots, <outlet> is replaced by <webui-route> elements
        // so we infer the outlet container from their parentElement.
        if (allRoutes.length > 0) {
          const container = allRoutes[allRoutes.length - 1].parentElement;
          if (container) {
            container.appendChild(stub);
            return stub;
          }
        }

        // Strategy 2: insert after the <outlet> marker (f-template components)
        const outletMarker = root.querySelector('outlet');
        if (outletMarker?.parentElement) {
          outletMarker.parentElement.insertBefore(stub, outletMarker.nextSibling);
          return stub;
        }

        // Strategy 3: fallback — append to render root
        root.appendChild(stub);
        return stub;
      }
    }

    // Fallback: create and append to body
    const el = createRouteStub(entry);
    document.body.appendChild(el);
    return el;
  }

  /**
   * Build the initial route chain from SSR'd active routes in the DOM.
   * Walks down through active `<webui-route>` elements.
   * Uses the native URLPattern API (Chromium 95+) to extract params.
   */
  private buildChainFromSSR(): RouteChainEntry[] {
    const chain: RouteChainEntry[] = [];
    let currentRoutes = this.discoverTopRoutes();
    let currentBase = '/';

    while (currentRoutes.length > 0) {
      const activeEl = currentRoutes.find(el => el.hasAttribute('active'));
      if (!activeEl) break;

      const rawPath = routePath(activeEl);
      const comp = routeComponent(activeEl);
      const pathname = window.location.pathname;

      // Resolve relative path against current base
      const resolvedPath = this.resolveRoutePath(rawPath, currentBase);

      // Use URLPattern to extract params
      const params = this.extractParams(resolvedPath, pathname);

      // Compute new base from the resolved pattern (consumed segments)
      currentBase = this.computeRouteBase(resolvedPath, pathname);

      chain.push({
        component: comp,
        path: rawPath,
        params,
        exact: isExact(activeEl),
        el: activeEl,
        allowedQuery: activeEl.getAttribute('query') ?? undefined,
        keepAlive: activeEl.hasAttribute('keep-alive'),
        pendingComponent: activeEl.getAttribute('pending') ?? undefined,
        errorComponent: activeEl.getAttribute('error') ?? undefined,
        invalidates: activeEl.getAttribute('invalidates')?.split(',').map(s => s.trim()).filter(Boolean) ?? undefined,
      });
      activateRoute(activeEl, params);

      currentRoutes = this.discoverChildRoutes(activeEl);
    }

    return chain;
  }

  /** Resolve a relative route path against a base. */
  private resolveRoutePath(path: string, base: string): string {
    if (path.length === 0) return base;
    if (path.startsWith('/')) return path;
    const relative = path.startsWith('./') ? path.slice(2) : path;
    if (relative.length === 0) return base;
    const b = base.endsWith('/') ? base : `${base}/`;
    return `${b}${relative}`;
  }

  /** Extract params from a pathname using URLPattern. */
  private extractParams(pattern: string, pathname: string): Record<string, string> {
    try {
      const urlPattern = new URLPattern({ pathname: pattern });
      const result = urlPattern.exec({ pathname });
      if (!result) return {};
      const groups = result.pathname.groups;
      const params: Record<string, string> = {};
      for (const [k, v] of Object.entries(groups)) {
        if (v !== undefined) params[k] = v;
      }
      return params;
    } catch {
      return {};
    }
  }

  /** Compute the route base from the matched portion of the pathname. */
  private computeRouteBase(pattern: string, pathname: string): string {
    // Count non-param segments in the pattern to determine how many
    // pathname segments this route consumes
    const patternParts = pattern.split('/').filter(Boolean);
    const pathParts = pathname.split('/').filter(Boolean);
    const consumed = Math.min(patternParts.length, pathParts.length);
    if (consumed === 0) return '/';
    return '/' + pathParts.slice(0, consumed).join('/');
  }

  /**
   * Compare old and new chains to find the first level that differs.
   * Returns the index of the first changed level.
   */
  private findChangeLevel(oldChain: RouteChainEntry[], newChain: RouteChainEntry[]): number {
    const len = Math.min(oldChain.length, newChain.length);
    for (let i = 0; i < len; i++) {
      if (
        oldChain[i].component !== newChain[i].component ||
        !this.paramsEqual(oldChain[i].params, newChain[i].params)
      ) {
        return i;
      }
    }
    // If chains differ in length, change starts at the shorter length
    return len;
  }

  private paramsEqual(a: Record<string, string>, b: Record<string, string>): boolean {
    const aKeys = Object.keys(a);
    if (aKeys.length !== Object.keys(b).length) return false;
    for (let i = 0; i < aKeys.length; i++) {
      if (a[aKeys[i]] !== b[aKeys[i]]) return false;
    }
    return true;
  }

  // ── Fetch + Mount ──────────────────────────────────────────────

  private async fetchPartial(
    requestPath: string,
    signal?: AbortSignal,
    speculative?: boolean,
  ): Promise<(PartialResponse & { inventory?: string }) | null> {
    const fullPath = prependBasePath(requestPath, this.basePath);
    const headers: Record<string, string> = { 'Accept': 'application/x-ndjson, application/json' };
    if (this.inventory) headers['X-WebUI-Inventory'] = this.inventory;

    const resp = await fetch(fullPath, { headers, signal });

    if (!resp.ok) return null;

    const contentType = resp.headers.get('content-type') ?? '';

    // HTML response (e.g. login page) — redirect
    if (!contentType.includes('json') && !contentType.includes('ndjson')) {
      if (speculative || signal?.aborted) return null;
      window.location.href = prependBasePath(requestPath, this.basePath);
      return null;
    }

    // Streaming NDJSON response — read Chunk 1, spawn background Chunk 2 reader
    if (contentType.includes('ndjson') && resp.body) {
      return this.readStreamingPartial(resp, requestPath, signal, speculative);
    }

    // Fallback: standard JSON response (non-streaming server)
    const data = await resp.json() as PartialResponse & { inventory?: string };
    if (signal?.aborted) return null;
    this.registerTemplatesAndStyles(data);
    this.injectCssLinks(data);
    return data;
  }

  /**
   * Read a streaming NDJSON partial response.
   * Returns after Chunk 1 (chain + templates) for immediate navigation commit.
   * Spawns a background reader for Chunk 2 (deferred per-component state).
   */
  private async readStreamingPartial(
    resp: Response,
    requestPath: string,
    signal?: AbortSignal,
    speculative?: boolean,
  ): Promise<(PartialResponse & { inventory?: string }) | null> {
    const reader = resp.body!.getReader();
    const decoder = new TextDecoder();
    let buffer = '';
    let chunk1: (PartialResponse & { inventory?: string }) | null = null;

    // Read until we get Chunk 1 (has 'chain' field)
    while (!chunk1) {
      const { done, value } = await reader.read();
      if (signal?.aborted) break;
      if (done) {
        // Flush remaining buffer on stream end
        buffer += decoder.decode();
        break;
      }
      buffer += decoder.decode(value, { stream: true });

      if (buffer.length > MAX_NDJSON_BUFFER) {
        console.warn('[Router] NDJSON buffer exceeded limit, aborting stream');
        reader.cancel().catch(() => {});
        return null;
      }

      const lines = buffer.split('\n');
      buffer = lines.pop()!; // keep incomplete last line

      for (const line of lines) {
        if (!line.trim()) continue;
        try {
          const parsed = JSON.parse(line);
          if (parsed.chain) {
            chunk1 = parsed;
          } else if (parsed.states && chunk1) {
            // Chunk 2 arrived in same read batch — store for post-commit application
            (chunk1 as any)._deferredStates = parsed.states;
          }
        } catch {
          // Malformed line — skip
        }
      }
    }

    // Process any final incomplete line left in buffer
    if (!chunk1 && buffer.trim()) {
      try {
        const parsed = JSON.parse(buffer);
        if (parsed.chain) chunk1 = parsed;
      } catch { /* ignore */ }
      buffer = '';
    }

    if (!chunk1 || signal?.aborted) {
      reader.cancel().catch(() => {});
      return null;
    }

    // Register templates/styles from Chunk 1
    this.registerTemplatesAndStyles(chunk1);
    this.injectCssLinks(chunk1);

    // Spawn background reader for remaining chunks (Chunk 2 state)
    const gen = this.navGeneration;
    this.deferredGeneration = gen;
    this.deferredReader = this.continueDeferredRead(reader, decoder, buffer, requestPath, gen, signal)
      .catch((err) => {
        if (!signal?.aborted) {
          console.warn('[Router] Deferred state reader failed:', err);
        }
      });

    return chunk1;
  }

  /**
   * Continue reading the NDJSON stream for Chunk 2 (deferred state).
   * Runs in the background after Chunk 1 has been committed.
   */
  private async continueDeferredRead(
    reader: ReadableStreamDefaultReader<Uint8Array>,
    decoder: TextDecoder,
    initialBuffer: string,
    requestPath: string,
    generation: number,
    signal?: AbortSignal,
  ): Promise<void> {
    let buffer = initialBuffer;
    try {
      while (true) {
        if (signal?.aborted || generation !== this.navGeneration) {
          reader.cancel().catch(() => {});
          return;
        }
        const { done, value } = await reader.read();
        if (done) {
          // Flush remaining bytes from the decoder
          buffer += decoder.decode();
          break;
        }
        buffer += decoder.decode(value, { stream: true });

        if (buffer.length > MAX_NDJSON_BUFFER) {
          console.warn('[Router] NDJSON deferred buffer exceeded limit, aborting');
          reader.cancel().catch(() => {});
          return;
        }

        const lines = buffer.split('\n');
        buffer = lines.pop()!;

        for (const line of lines) {
          if (!line.trim()) continue;
          if (generation !== this.navGeneration) return; // Stale — stop
          try {
            const parsed = JSON.parse(line);
            if (parsed.states) {
              this.applyDeferredStates(parsed.states, requestPath);
            } else if (parsed.error) {
              console.warn('[Router] Streaming state error:', parsed.error);
            }
          } catch {
            // Malformed line — skip
          }
        }
      }

      // Process final incomplete line
      if (buffer.trim() && generation === this.navGeneration) {
        try {
          const parsed = JSON.parse(buffer);
          if (parsed.states) {
            this.applyDeferredStates(parsed.states, requestPath);
          } else if (parsed.error) {
            console.warn('[Router] Streaming state error:', parsed.error);
          }
        } catch { /* ignore */ }
      }
    } finally {
      // Mark cache entry as complete
      const cacheEntry = this.cache.get(requestPath);
      if (cacheEntry) cacheEntry.complete = true;
    }
  }

  /**
   * Apply deferred per-component states from streaming Chunk 2.
   * States array is matched 1:1 to activeChain entries by position.
   * null entries are skipped (component keeps current state).
   */
  private applyDeferredStates(
    states: (Record<string, unknown> | null)[],
    requestPath: string,
  ): void {
    if (requestPath !== this.currentRequestPath) return; // Stale
    for (let i = 0; i < states.length && i < this.activeChain.length; i++) {
      const state = states[i];
      if (!hasState(state)) continue;
      const entry = this.activeChain[i];
      if (!entry.el || !entry.component) continue;

      // Don't override loader results
      const ctor = customElements.get(entry.component) as
        ((new () => HTMLElement) & { loader?: Function }) | undefined;
      if (ctor && typeof ctor.loader === 'function') continue;

      const compEl = entry.el.querySelector(entry.component);
      if (compEl && typeof (compEl as any).setState === 'function') {
        (compEl as any).setState(state);
      }
      // Update the chain entry's state for cache consistency
      entry.state = state;
    }
  }

  /** Inject CSS stylesheet links from a partial response. */
  private injectCssLinks(data: PartialResponse): void {
    if (data.css) {
      for (const href of data.css) {
        if (!this.injectedCss.has(href)) {
          this.injectedCss.add(href);
          const link = document.createElement('link');
          link.rel = 'stylesheet';
          link.href = href;
          document.head.appendChild(link);
        }
      }
    }
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

  /**
   * Apply state to a mounted route component using per-entry state.
   * State resolution: loader override > per-entry state > keep-alive preserve.
   */
  private applyState(
    entry: RouteChainEntry,
    query?: Record<string, string>,
    loaderStates?: Map<string, Record<string, unknown> | typeof LOADER_FAILED>,
  ): void {
    if (!entry.component || !entry.el) return;
    const compEl = entry.el.querySelector(entry.component) as any;
    if (!compEl) return;

    const override = loaderStates?.get(entry.component);
    const effectiveOverride = override === LOADER_FAILED ? undefined : override;
    const loaderExists = override !== undefined;
    const isKeepAlive = entry.keepAlive || entry.el.hasAttribute('keep-alive');

    if (isKeepAlive) {
      if (effectiveOverride) {
        applyParamsQueryState(compEl, entry.el, entry.params, effectiveOverride, query);
      } else if (loaderExists) {
        // Loader failed → fall back to per-entry server state
        applyParamsQueryState(compEl, entry.el, entry.params, entry.state, query);
      } else if (hasState(entry.state)) {
        // Server provided meaningful per-entry state → apply it
        applyParamsQueryState(compEl, entry.el, entry.params, entry.state, query);
      } else {
        // null or empty state → preserve local state, only update attrs
        applyParamsAndQuery(compEl, entry.el, entry.params, query);
      }
    } else {
      // Non-keep-alive: apply state
      const stateToApply = effectiveOverride ?? entry.state;
      applyParamsQueryState(compEl, entry.el, entry.params, stateToApply, query);
    }
  }

  // ── Lazy Loading ────────────────────────────────────────────────

  /**
   * Ensure a component's JS module is loaded. If a lazy loader is
   * configured for this tag and the element isn't already registered,
   * invoke the loader. The promise is cached so each loader runs at
   * most once.
   */
  private async ensureComponentLoaded(tag: string): Promise<void> {
    if (customElements.get(tag)) return;

    const loader = this.loaders[tag];
    if (!loader) return;

    let promise = this.loaderPromises.get(tag);
    if (!promise) {
      promise = loader().then(() => {}).finally(() => { this.loaderPromises.delete(tag); });
      this.loaderPromises.set(tag, promise);
    }
    await promise;
  }

  // ── Route Loaders ──────────────────────────────────────────────

  /**
   * Resolve static `loader()` methods on route component constructors.
   *
   * Called **before** commitNavigation so loader results are available
   * synchronously during the view transition. Components without a
   * static `loader()` are skipped — they use server-provided state.
   *
   * On failure, the loader is skipped with a warning and the component
   * falls back to server-provided per-entry state.
   */
  private async resolveLoaders(
    chain: RouteChainEntry[],
    query: Record<string, string>,
    signal?: AbortSignal,
  ): Promise<Map<string, Record<string, unknown> | typeof LOADER_FAILED>> {
    const results = new Map<string, Record<string, unknown> | typeof LOADER_FAILED>();

    // Collect only entries that have loaders — avoids creating promises for non-loader components
    type LoaderEntry = { component: string; params: Record<string, string>; loaderFn: (ctx: import('./types.js').RouteLoaderContext) => Promise<Record<string, unknown>> };
    const loaderEntries: LoaderEntry[] = [];
    for (let i = 0; i < chain.length; i++) {
      const entry = chain[i];
      if (!entry.component) continue;
      const ctor = customElements.get(entry.component) as (
        (new () => HTMLElement) & { loader?: (ctx: import('./types.js').RouteLoaderContext) => Promise<Record<string, unknown>> }
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

  // ── Dev-mode Validation ─────────────────────────────────────────

  /**
   * Development-mode validation of the route configuration.
   * Warns about common mistakes via console.warn.
   */
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

  // ── Discovery ───────────────────────────────────────────────────

  /**
   * Find top-level route elements — direct children of `<body>` or
   * the document (not nested inside another route's outlet).
   */
  private discoverTopRoutes(): HTMLElement[] {
    const results: HTMLElement[] = [];
    // Top-level routes are direct light DOM children of the body
    for (const child of document.body.children) {
      if (child.tagName === 'WEBUI-ROUTE') {
        results.push(child as HTMLElement);
      }
    }
    return results;
  }

  /**
   * Find child route elements inside a parent route's component light DOM.
   * Traverses: parent route → component → component's children → <webui-route> elements.
   */
  private discoverChildRoutes(parentRoute: HTMLElement): HTMLElement[] {
    const results: HTMLElement[] = [];
    const comp = routeComponent(parentRoute);
    if (!comp) return results;

    const compEl = parentRoute.querySelector(comp);
    if (!compEl) return results;

    // Child <webui-route> elements are in the component's render root
    const root = renderRoot(compEl);
    for (const child of root.querySelectorAll(ROUTE_SELECTOR)) {
      results.push(child as HTMLElement);
    }

    return results;
  }

  private currentTarget(): NavigationTarget {
    return buildNavigationTarget(new URL(window.location.href), this.basePath);
  }

  // ── Component Inventory ────────────────────────────────────────

  private updateInventory(serverInventory: string): void {
    this.inventory = serverInventory;
  }

  /**
   * Commit a navigation using server-confirmed data (the standard fetch-first path).
   * Used both as fallback for first visits and when the server responds within the
   * optimistic delay budget.
   */
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
        .map(entry => this.ensureComponentLoaded(entry.component)),
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
    const loaderStates = await this.resolveLoaders(newChain, query, signal);
    if (signal?.aborted || (generation !== undefined && generation !== this.navGeneration)) return;

    const changeLevel = this.findChangeLevel(this.activeChain, newChain);
    const isQueryOnlyChange = changeLevel === newChain.length && newChain.length > 0;

    const commitNavigation = (): void => {
      // Deactivate old chain from leaf up
      for (let i = this.activeChain.length - 1; i >= changeLevel; i--) {
        if (this.activeChain[i].el) deactivateRoute(this.activeChain[i].el!);
      }
      for (let i = 0; i < changeLevel; i++) {
        newChain[i].el = this.activeChain[i].el;
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
          if (entry.component) this.applyState(entry, query, loaderStates);
          activateRoute(entry.el, entry.params);
          continue;
        }
        const routeEl = this.findOrCreateRouteElement(parent, entry);
        entry.el = routeEl;
        if (entry.component) {
          const override = loaderStates.get(entry.component);
          const effectiveOverride = override === LOADER_FAILED ? undefined : override;

          const isKeepAlive = entry.keepAlive || routeEl.hasAttribute('keep-alive');
          const existingComp = routeEl.firstElementChild;
          if (isKeepAlive && existingComp?.matches(entry.component)) {
            const stateToApply = effectiveOverride ?? entry.state;
            if (hasState(stateToApply)) {
              applyParamsQueryState(existingComp, routeEl, entry.params, stateToApply, query);
            } else {
              applyParamsAndQuery(existingComp, routeEl, entry.params, query);
            }
          } else {
            const stateToApply = effectiveOverride ?? entry.state;
            this.mountComponent(routeEl, entry.component, entry.params, stateToApply, query);
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

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

import { buildNavigationTarget, prependBasePath } from './navigation-path.js';
import type { RouterConfig, NavigationEvent } from './types.js';
import type { NavigationTarget } from './navigation-path.js';

const ROUTE_SELECTOR = 'webui-route';
const SSR_PRELOAD_SELECTOR = 'link[data-webui-ssr-preload]';

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
  const raw = el.getAttribute('query');
  if (raw == null) return null;
  const set = new Set<string>();
  for (const part of raw.split(',')) {
    const trimmed = part.trim();
    if (trimmed) set.add(trimmed);
  }
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
  const paramAttrNames = routeParams
    ? new Set(Object.keys(routeParams).map(toKebab))
    : undefined;
  const result: Record<string, string> = {};
  for (const [k, v] of Object.entries(query)) {
    if (allowed.has(k) && !(paramAttrNames && paramAttrNames.has(toKebab(k)))) {
      result[k] = v;
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
  state: Record<string, unknown>;
  /** Module CSS definitions to append before executing template scripts. */
  templateStyles?: string[];
  templates: string[];
  path: string;
  chain?: RouteChainEntry[];
  /** CSS stylesheet URLs to inject into `<head>` for this route's components. */
  css?: string[];
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
  for (const [key, value] of Object.entries(params)) {
    component.setAttribute(toKebab(key), value);
  }

  const allowed = routeAllowedQuery(routeEl);
  const filtered = query ? filterQuery(query, allowed, params) : {};
  const newAttrs = new Set<string>();
  for (const [key, value] of Object.entries(filtered)) {
    const attr = toKebab(key);
    component.setAttribute(attr, value);
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
  queryAttrsMap.set(component, newAttrs);
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
  data: PartialResponse,
  query?: Record<string, string>,
  stateOverride?: Record<string, unknown>,
): void {
  applyParamsAndQuery(component, routeEl, params, query);

  if (typeof (component as any).setState === 'function') {
    (component as any).setState(stateOverride ?? data.state);
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
  /** Set of component tags known to have a static loader() method. */
  private loaderComponents = new Set<string>();
  /** Cached preload result from a speculative hover fetch. */
  private preloadCache: { path: string; data: PartialResponse & { inventory?: string }; ts: number } | null = null;
  /** AbortController for the in-flight preload fetch. */
  private preloadController: AbortController | null = null;
  /** Monotonic preload generation — prevents stale hover fetches from overwriting the cache. */
  private preloadGeneration = 0;

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
    this.loaderComponents.clear();
    this.preloadCache = null;
    this.preloadController?.abort();
    this.preloadController = null;
  }

  // ── Preload on hover ──────────────────────────────────────────

  /** Maximum age (ms) for a preloaded partial before it's considered stale. */
  private static readonly PRELOAD_TTL = 5_000;

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
   * The handler is naturally debounced: the `preloadCache` path check
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
      const anchor = (e.composedPath() as Element[]).find(
        el => el?.tagName === 'A',
      ) as HTMLAnchorElement | undefined;
      if (!anchor) return;

      const href = anchor.getAttribute('href');
      if (!href || href.startsWith('#')) return;

      let url: URL;
      try {
        url = new URL(href, location.href);
      } catch {
        return;
      }
      if (url.origin !== location.origin) return;

      const target = buildNavigationTarget(url, this.basePath);

      // Skip if already on this path or already cached for it
      if (target.requestPath === this.currentTarget().requestPath) return;
      if (this.preloadCache?.path === target.requestPath) return;

      // Abort any in-flight speculative fetch and start a new one
      this.preloadController?.abort();
      const controller = new AbortController();
      this.preloadController = controller;
      const gen = ++this.preloadGeneration;

      this.fetchPartial(target.requestPath, controller.signal, true)
        .then(data => {
          // Only cache if this is still the latest preload request
          if (data && gen === this.preloadGeneration && !controller.signal.aborted) {
            this.preloadCache = { path: target.requestPath, data, ts: Date.now() };
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
      const thisGen = ++this.navGeneration;

      // Use preloaded data if we have a fresh cache hit for this path
      let partialData: (PartialResponse & { inventory?: string }) | null = null;
      const cached = this.preloadCache;
      if (cached && cached.path === requestPath &&
          Date.now() - cached.ts < WebUIRouter.PRELOAD_TTL) {
        partialData = cached.data;
        this.preloadCache = null;
      } else {
        // Cancel any in-flight speculative fetch — we're doing a real navigation
        this.preloadController?.abort();
        this.preloadCache = null;
        partialData = await this.fetchPartial(requestPath, signal);
      }

      if (!partialData || signal?.aborted || thisGen !== this.navGeneration) return;
      await this.commitWithData(partialData, requestPath, query, signal, thisGen);
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
    const keysA = Object.keys(a);
    const keysB = Object.keys(b);
    if (keysA.length !== keysB.length) return false;
    for (const key of keysA) {
      if (a[key] !== b[key]) return false;
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
    const headers: Record<string, string> = { 'Accept': 'application/json' };
    if (this.inventory) headers['X-WebUI-Inventory'] = this.inventory;

    // Send the set of component tags that have static loaders. The host
    // server does its own route matching and can check whether the TARGET
    // leaf component is in this list. If so, it may skip expensive state
    // computation and return state: {}.
    if (this.loaderComponents.size > 0) {
      headers['X-WebUI-Has-Loader'] = [...this.loaderComponents].join(',');
    }

    const resp = await fetch(fullPath, { headers, signal });

    if (!resp.ok) return null;

    const contentType = resp.headers.get('content-type') ?? '';
    if (!contentType.includes('application/json')) {
      // Server returned HTML (e.g. login page) instead of JSON partial.
      // Speculative fetches never redirect — just bail out silently.
      if (speculative || signal?.aborted) return null;
      window.location.href = prependBasePath(requestPath, this.basePath);
      return null;
    }

    const data = await resp.json() as PartialResponse & { inventory?: string };

    // Bail out before applying side effects if this navigation was superseded.
    if (signal?.aborted) return null;

    // Register templates, styles, and CSS using the shared pipeline
    this.registerTemplatesAndStyles(data);

    // Inject CSS stylesheet links (used by some server implementations)
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

    return data;
  }

  private mountComponent(
    routeEl: HTMLElement,
    componentTag: string,
    data: PartialResponse,
    params: Record<string, string>,
    query?: Record<string, string>,
    stateOverride?: Record<string, unknown>,
  ): void {
    const component = document.createElement(componentTag);
    routeEl.textContent = '';
    routeEl.appendChild(component);

    // Component module is pre-loaded and defined before commitNavigation.
    // connectedCallback fires synchronously on appendChild, populating
    // the component's light DOM immediately.

    applyParamsQueryState(component, routeEl, params, data, query, stateOverride);
  }

  /**
   * Apply partial state to a mounted route component.
   * Calls the framework's built-in `setState` which sets
   * `@observable` properties and flushes DOM updates synchronously.
   * Route params are set as HTML attributes for `@attr` reflection.
   * Allowed query params (declared via `query` attr on the route) are
   * also set as attributes; stale ones from the previous navigation
   * are removed.
   */
  private applyState(
    entry: RouteChainEntry,
    data: PartialResponse,
    query?: Record<string, string>,
    loaderStates?: Map<string, Record<string, unknown>>,
  ): void {
    if (!entry.component || !entry.el) return;
    const compEl = entry.el.querySelector(entry.component) as any;
    if (!compEl) return;

    applyParamsQueryState(compEl, entry.el, entry.params, data, query, loaderStates?.get(entry.component));
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
      promise = loader().then(() => {});
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
   * falls back to `data.state` from the server partial.
   */
  private async resolveLoaders(
    chain: RouteChainEntry[],
    query: Record<string, string>,
    signal?: AbortSignal,
  ): Promise<Map<string, Record<string, unknown>>> {
    const results = new Map<string, Record<string, unknown>>();

    const tasks = chain
      .filter(entry => entry.component)
      .map(async entry => {
        const ctor = customElements.get(entry.component) as (
          (new () => HTMLElement) & { loader?: (ctx: import('./types.js').RouteLoaderContext) => Promise<Record<string, unknown>> }
        ) | undefined;
        if (!ctor || typeof ctor.loader !== 'function') return;

        // Track this component as having a loader for the X-WebUI-Has-Loader header
        this.loaderComponents.add(entry.component);

        try {
          const ctx = {
            params: entry.params,
            query,
            signal: signal ?? new AbortController().signal,
          };
          const state = await ctor.loader(ctx);
          if (!signal?.aborted && state) {
            results.set(entry.component, state);
          }
        } catch (err: unknown) {
          if (err instanceof DOMException && err.name === 'AbortError') return;
          console.warn(
            `[Router] Loader failed for <${entry.component}>, using server state:`,
            err,
          );
        }
      });

    await Promise.all(tasks);
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
    const newChain: RouteChainEntry[] = (partialData.chain ?? []).map(e => ({
      component: e.component ?? '',
      path: e.path ?? '',
      params: e.params ?? {},
      exact: e.exact,
      allowedQuery: e.allowedQuery,
      keepAlive: e.keepAlive,
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
          this.applyState(newChain[i], partialData, query, loaderStates);
        }
      }
      for (let i = changeLevel; i < newChain.length; i++) {
        const entry = newChain[i];
        const oldEntry = i < this.activeChain.length ? this.activeChain[i] : null;
        const parent = i > 0 ? newChain[i - 1] : null;
        if (oldEntry?.component === entry.component && oldEntry?.el) {
          entry.el = oldEntry.el;
          if (entry.component) this.applyState(entry, partialData, query, loaderStates);
          activateRoute(entry.el, entry.params);
          continue;
        }
        const routeEl = this.findOrCreateRouteElement(parent, entry);
        entry.el = routeEl;
        if (entry.component) {
          const override = loaderStates.get(entry.component);
          // Keep-alive: if the route has keep-alive and already has the correct
          // component mounted (from a previous visit), reuse it and apply fresh
          // state instead of destroying and recreating the component.
          const isKeepAlive = entry.keepAlive || routeEl.hasAttribute('keep-alive');
          const existingComp = routeEl.firstElementChild;
          if (isKeepAlive && existingComp?.matches(entry.component)) {
            // Keep-alive reactivation: preserve local state by default.
            // Only call setState if a loader provided fresh data (override).
            if (override) {
              applyParamsQueryState(existingComp, routeEl, entry.params, partialData, query, override);
            } else {
              applyParamsAndQuery(existingComp, routeEl, entry.params, query);
            }
          } else {
            this.mountComponent(routeEl, entry.component, partialData, entry.params, query, override);
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

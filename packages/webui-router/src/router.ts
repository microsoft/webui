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
 * Apply route params, allowed query params, and initial state to a component.
 * Shared by both initial mount and subsequent state updates. Stale query-param
 * attributes from a previous navigation are automatically removed.
 */
function applyParamsQueryState(
  component: Element,
  routeEl: HTMLElement,
  params: Record<string, string>,
  data: PartialResponse,
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

  if (typeof (component as any).setInitialState === 'function') {
    (component as any).setInitialState(data.state);
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
      if (this.config.dev) {
        this.validateRoutes();
      }
    } else {
      this.clearSsrPreloads();
      const partialData = await this.fetchPartial(requestPath, signal);
      if (!partialData) return;

      // Bail out if a newer navigation has superseded this one.
      if (signal?.aborted) return;

      const newChain: RouteChainEntry[] = (partialData.chain ?? []).map(e => ({
        component: e.component ?? '',
        path: e.path ?? '',
        params: e.params ?? {},
        exact: e.exact,
        allowedQuery: e.allowedQuery,
      }));

      if (newChain.length === 0) {
        console.warn(`[Router] No route matched for path: ${requestPath}`);
        window.location.href = prependBasePath(requestPath, this.basePath);
        return;
      }

      // Pre-load all component modules in parallel before the DOM swap so
      // the view transition only covers the synchronous mount.
      // Race against abort so a superseding navigation doesn't wait for
      // in-flight imports from the aborted one. ensureComponentLoaded
      // caches module promises, so any work done here is reused by the
      // winning navigation.
      if (signal?.aborted) return;
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
      if (signal?.aborted) return;

      const changeLevel = this.findChangeLevel(this.activeChain, newChain);

      // When only query params change (same route, different ?sort= etc.),
      // changeLevel equals chain length so nothing remounts. Detect this
      // and re-apply state to all components in the chain from the server's
      // fresh partial response.
      const isQueryOnlyChange = changeLevel === newChain.length && newChain.length > 0;

      // DOM swap — synchronous, safe inside view transitions.
      // All async work (fetch, import) is done above.
      const commitNavigation = (): void => {
        // Deactivate old chain from leaf up
        for (let i = this.activeChain.length - 1; i >= changeLevel; i--) {
          if (this.activeChain[i].el) {
            deactivateRoute(this.activeChain[i].el!);
          }
        }

        // Transfer DOM elements to retained levels
        for (let i = 0; i < changeLevel; i++) {
          newChain[i].el = this.activeChain[i].el;
        }

        // Re-apply state to retained (non-remounted) parent components.
        if (changeLevel > 0 || isQueryOnlyChange) {
          const end = isQueryOnlyChange ? newChain.length : changeLevel;
          for (let i = 0; i < end; i++) {
            this.applyState(newChain[i], partialData, query);
          }
        }

        // Mount from the change level down
        for (let i = changeLevel; i < newChain.length; i++) {
          const entry = newChain[i];
          const oldEntry = i < this.activeChain.length ? this.activeChain[i] : null;
          const parent = i > 0 ? newChain[i - 1] : null;

          // Same component tag at this level → reuse instance, update state
          if (
            oldEntry &&
            oldEntry.component === entry.component &&
            oldEntry.el
          ) {
            entry.el = oldEntry.el;
            if (entry.component && partialData) {
              this.applyState(entry, partialData, query);
            }
            activateRoute(entry.el, entry.params);
            continue;
          }

          // Different component (or no old entry) → full mount
          const routeEl = this.findOrCreateRouteElement(parent, entry);
          entry.el = routeEl;

          if (entry.component && partialData) {
            this.mountComponent(routeEl, entry.component, partialData, entry.params, query);
          }

          activateRoute(routeEl, entry.params);
        }

        this.activeChain = newChain;
      };

      // DOM swap — wrapped in a view transition when available.
      // Skip view transitions for query-only changes (issue #235): no
      // components remount so there is nothing to animate, and the
      // transition would blur the active element (e.g. search input).
      // Await updateCallbackDone (not .finished) so the Navigation API
      // handler resolves as soon as the DOM commit completes, without
      // waiting for the CSS animation to finish. This allows rapid
      // navigations to supersede each other without queuing.
      if (document.startViewTransition && !isQueryOnlyChange) {
        const transition = document.startViewTransition(commitNavigation);
        await transition.updateCallbackDone;
      } else {
        commitNavigation();
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
        for (const child of root.querySelectorAll(ROUTE_SELECTOR)) {
          if (child.getAttribute('component') === entry.component) {
            return child as HTMLElement;
          }
        }

        // Not found — create in the outlet area of parent component
        const stub = createRouteStub(entry);
        const outletMarker = root.querySelector('outlet');
        if (outletMarker?.parentElement) {
          outletMarker.parentElement.insertBefore(stub, outletMarker.nextSibling);
        } else {
          root.appendChild(stub);
        }
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

  private async fetchPartial(requestPath: string, signal?: AbortSignal): Promise<(PartialResponse & { inventory?: string }) | null> {
    const fullPath = prependBasePath(requestPath, this.basePath);
    const headers: Record<string, string> = { 'Accept': 'application/json' };
    if (this.inventory) headers['X-WebUI-Inventory'] = this.inventory;
    const resp = await fetch(fullPath, { headers, signal });

    if (!resp.ok) return null;

    const contentType = resp.headers.get('content-type') ?? '';
    if (!contentType.includes('application/json')) {
      // Server returned HTML (e.g. login page) instead of JSON partial.
      // Trigger a full page navigation so the browser handles it.
      if (signal?.aborted) return null;
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
  ): void {
    const component = document.createElement(componentTag);
    routeEl.textContent = '';
    routeEl.appendChild(component);

    // Component module is pre-loaded and defined before commitNavigation.
    // connectedCallback fires synchronously on appendChild, populating
    // the component's light DOM immediately.

    applyParamsQueryState(component, routeEl, params, data, query);
  }

  /**
   * Apply partial state to a mounted route component.
   * Calls the framework's built-in `setInitialState` which sets
   * `@observable` properties and flushes DOM updates synchronously.
   * Route params are set as HTML attributes for `@attr` reflection.
   * Allowed query params (declared via `query` attr on the route) are
   * also set as attributes; stale ones from the previous navigation
   * are removed.
   */
  private applyState(entry: RouteChainEntry, data: PartialResponse, query?: Record<string, string>): void {
    if (!entry.component || !entry.el) return;
    const compEl = entry.el.querySelector(entry.component) as any;
    if (!compEl) return;

    applyParamsQueryState(compEl, entry.el, entry.params, data, query);
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
}

/** Singleton router instance. */
export const Router = new WebUIRouter();

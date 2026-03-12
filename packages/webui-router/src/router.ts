/**
 * Core router — uses the Navigation API to intercept navigations and
 * activates/deactivates `<webui-route>` elements in the DOM tree.
 *
 * For routes with a `component` attribute, the router fetches a JSON
 * partial from the server (state + f-templates), registers any new
 * templates, instantiates the component, and mounts it into the route.
 */

import { matchPath, specificity } from './matcher.js';
import type { RouterConfig, NavigationEvent } from './types.js';

const ROUTE_SELECTOR = 'webui-route';

// ── Route element helpers ────────────────────────────────────────

function routePath(el: Element): string {
  return el.getAttribute('path') ?? '';
}

function routeName(el: Element): string {
  return el.getAttribute('name') ?? '';
}

function isExact(el: Element): boolean {
  return el.hasAttribute('exact');
}

function routeComponent(el: Element): string {
  return el.getAttribute('component') ?? '';
}

function activateRoute(el: HTMLElement, params: Record<string, string>): void {
  (el as any)._routeParams = params;
  el.setAttribute('active', '');
  el.style.display = '';
}

function deactivateRoute(el: HTMLElement): void {
  (el as any)._routeParams = {};
  el.removeAttribute('active');
  el.style.display = 'none';
}

function getRouteParams(el: Element): Record<string, string> {
  return (el as any)._routeParams ?? {};
}

// ── WebUIRouteElement custom element ─────────────────────────────

/** Custom element backing `<webui-route>`. Self-registers with the Router. */
export class WebUIRouteElement extends HTMLElement {
  get path(): string { return this.getAttribute('path') ?? ''; }
  get routeName(): string { return this.getAttribute('name') ?? ''; }
  get exact(): boolean { return this.hasAttribute('exact'); }
  get component(): string { return this.getAttribute('component') ?? ''; }
  get isActive(): boolean { return this.hasAttribute('active'); }
  get params(): Record<string, string> { return (this as any)._routeParams ?? {}; }

  connectedCallback(): void {
    routeRegistry.add(this);
  }

  disconnectedCallback(): void {
    routeRegistry.delete(this);
  }

  activate(params: Record<string, string> = {}): void {
    (this as any)._routeParams = params;
    this.setAttribute('active', '');
    this.style.display = '';
  }

  deactivate(): void {
    (this as any)._routeParams = {};
    this.removeAttribute('active');
    this.style.display = 'none';
  }
}

/** Global registry — route elements self-register via connectedCallback. */
const routeRegistry = new Set<WebUIRouteElement>();

// ── Router ───────────────────────────────────────────────────────

/** JSON partial response from the server. */
interface PartialResponse {
  state: Record<string, unknown>;
  templates: string[];
  path: string;
}

export class WebUIRouter {
  private config: RouterConfig = {};
  private started = false;
  private cleanupFns: Array<() => void> = [];
  private isInitialNavigation = true;
  /** Hex string tracking which component templates are loaded. */
  private inventory = '';
  /** Opt-in lazy loaders: component tag → async import function. */
  private loaders: Record<string, () => Promise<unknown>> = {};
  /** Deduplication cache for in-flight / completed loader promises. */
  private loaderPromises = new Map<string, Promise<void>>();

  /** The name of the currently active leaf route. */
  get activeRouteName(): string {
    const el = this.findActiveLeaf();
    return el ? routeName(el) : '';
  }

  /** The bound params of the currently active leaf route. */
  get activeParams(): Record<string, string> {
    const el = this.findActiveLeaf();
    return el ? getRouteParams(el) : {};
  }

  /** Start the router. Lazily registers the `<webui-route>` custom element. */
  start(config: RouterConfig = {}): void {
    if (this.started) return;
    this.started = true;
    this.config = config;
    this.loaders = config.loaders ?? {};

    if (!customElements.get('webui-route')) {
      customElements.define('webui-route', WebUIRouteElement);
    }

    this.inventory = this.buildInventoryFromDOM();

    const nav = (window as any).navigation;
    const handler = (event: any) => {
      if (!event.canIntercept || event.hashChange) return;
      const url = new URL(event.destination.url);
      if (url.origin !== location.origin) return;
      event.intercept({
        handler: async () => {
          await this.handleNavigation(this.stripBase(url.pathname));
        },
      });
    };
    nav.addEventListener('navigate', handler);
    this.cleanupFns.push(() => nav.removeEventListener('navigate', handler));

    this.handleNavigation(this.currentPath());
  }

  /** Navigate to a new path. */
  navigate(path: string): void {
    const fullPath = (this.config.basePath ?? '') + path;
    (window as any).navigation.navigate(fullPath);
  }

  /** Navigate back. */
  back(): void {
    (window as any).navigation.back();
  }

  /** Tear down. */
  destroy(): void {
    this.loaderPromises.clear();
    this.loaders = {};
    for (const fn of this.cleanupFns) fn();
    this.cleanupFns = [];
    this.started = false;
  }

  // ── Route matching ──────────────────────────────────────────────

  private async handleNavigation(pathname: string): Promise<void> {
    const topRoutes = this.discoverTopRoutes();

    if (topRoutes.length > 0) {
      if (!this.isInitialNavigation) {
        this.deactivateAll();
      }
      const matched = await this.activateMatching(topRoutes, pathname);
      if (!matched && !this.isInitialNavigation) {
        // No client route matched — fall through to server
        window.location.href = (this.config.basePath ?? '') + pathname;
        return;
      }
    }

    this.isInitialNavigation = false;

    const active = this.findActiveLeaf();
    const detail: NavigationEvent = {
      routeName: active ? routeName(active) : '',
      params: active ? getRouteParams(active) : {},
      path: pathname,
    };
    window.dispatchEvent(new CustomEvent('webui:route:navigated', { detail }));
  }

  private async activateMatching(routes: HTMLElement[], pathname: string): Promise<boolean> {
    let best: { el: HTMLElement; params: Record<string, string>; score: number } | null = null;

    for (const el of routes) {
      const m = matchPath(routePath(el), pathname, isExact(el));
      if (m) {
        const score = specificity(routePath(el));
        if (!best || score > best.score) {
          best = { el, params: m.params, score };
        }
      }
    }

    if (!best) return false;

    const comp = routeComponent(best.el);
    if (comp) {
      const existing = best.el.querySelector(comp);
      const isSSRd = existing && existing.shadowRoot;

      if (isSSRd && this.isInitialNavigation) {
        // SSR'd content is visually correct — skip fetchAndMount (which would
        // re-fetch and replace the DOM), but still load the component JS so
        // the custom element is defined and FAST can hydrate it.
        await this.ensureComponentLoaded(comp);
      } else if (!isSSRd) {
        await this.ensureComponentLoaded(comp);
        await this.fetchAndMount(best.el, comp, pathname, best.params);
      } else if (typeof (existing as any).setInitialState === 'function') {
        await this.ensureComponentLoaded(comp);
        const fullPath = (this.config.basePath ?? '') + pathname;
        const headers: Record<string, string> = { 'Accept': 'application/json' };
        if (this.inventory) headers['X-WebUI-Inventory'] = this.inventory;
        const resp = await fetch(fullPath, { headers });
        if (resp.ok) {
          const data = await resp.json();
          (existing as any).setInitialState(data.state, best.params);
        }
      }
    }

    activateRoute(best.el, best.params);
    return true;
  }

  // ── Fetch + Mount ──────────────────────────────────────────────

  private async fetchAndMount(
    routeEl: HTMLElement,
    componentTag: string,
    pathname: string,
    params: Record<string, string> = {},
  ): Promise<void> {
    const fullPath = (this.config.basePath ?? '') + pathname;
    const headers: Record<string, string> = { 'Accept': 'application/json' };
    if (this.inventory) headers['X-WebUI-Inventory'] = this.inventory;
    const resp = await fetch(fullPath, { headers });

    if (!resp.ok) return;

    const data = await resp.json() as PartialResponse & { inventory?: string };

    if (data.inventory) {
      this.updateInventory(data.inventory);
    }

    for (const tmpl of data.templates) {
      const container = document.createElement('div');
      container.innerHTML = tmpl;
      while (container.firstChild) {
        document.body.appendChild(container.firstChild);
      }
    }

    const component = document.createElement(componentTag);
    routeEl.textContent = '';
    routeEl.appendChild(component);

    // Ensure the component's JS module is loaded (lazy loader or already eager)
    await this.ensureComponentLoaded(componentTag);
    await customElements.whenDefined(componentTag);
    if (typeof (component as any).setInitialState === 'function') {
      (component as any).setInitialState(data.state, params);
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
      promise = loader().then(() => {});
      this.loaderPromises.set(tag, promise);
    }
    await promise;
  }

  // ── Discovery ───────────────────────────────────────────────────

  private discoverTopRoutes(): HTMLElement[] {
    const results: HTMLElement[] = [];
    for (const el of routeRegistry) {
      if (!el.parentElement?.closest(ROUTE_SELECTOR)) {
        results.push(el);
      }
    }
    return results;
  }

  private deactivateAll(): void {
    for (const el of routeRegistry) {
      if (el.hasAttribute('active')) {
        deactivateRoute(el);
      }
    }
  }

  private findActiveLeaf(): HTMLElement | null {
    let last: HTMLElement | null = null;
    for (const el of routeRegistry) {
      if (el.hasAttribute('active')) {
        last = el;
      }
    }
    return last;
  }

  private currentPath(): string {
    return this.stripBase(window.location.pathname);
  }

  private stripBase(path: string): string {
    const base = this.config.basePath ?? '';
    if (base && path.startsWith(base)) return path.slice(base.length) || '/';
    return path;
  }

  // ── Component Inventory ────────────────────────────────────────

  private static componentBitPosition(name: string): number {
    let hash = 0x811c9dc5 | 0;
    for (let i = 0; i < name.length; i++) {
      hash ^= name.charCodeAt(i);
      hash = Math.imul(hash, 0x01000193);
    }
    return ((hash >>> 0) % 256);
  }

  private buildInventoryFromDOM(): string {
    const inv = new Uint8Array(32);
    for (const tmpl of document.querySelectorAll('f-template[name]')) {
      const name = tmpl.getAttribute('name');
      if (name) {
        const bit = WebUIRouter.componentBitPosition(name);
        inv[bit >> 3] |= 1 << (bit & 7);
      }
    }
    return Array.from(inv, b => b.toString(16).padStart(2, '0')).join('');
  }

  private updateInventory(serverInventory: string): void {
    this.inventory = serverInventory;
  }
}

/** Singleton router instance. */
export const Router = new WebUIRouter();

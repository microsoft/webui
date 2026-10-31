// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import type { NavigationCache, RouteChainEntry } from './cache.js';
import type { RouteBoundaries, RouteTransitions } from './capabilities.js';
import type { ActionContext } from './actions.js';
import type { RoutePreload } from './preload.js';
import type { PartialTransport } from './transport.js';

declare global {
  interface WebUIRuntimeGlobal {
    /** Base of compiler-emitted asset hrefs, such as the `css` list, when not the document. */
    assetBase?: string;
    chain?: unknown[];
    inventory?: string;
    buildId?: string;
    styleInventory?: string;
    inventoryEpoch?: number;
    nonce?: string;
    css?: string[];
    styles?: string[];
    state?: Record<string, unknown>;
    templateHostExclusions?: Set<string>;
    componentStyles?: ComponentStyles;
  }

  interface Window {
    __webui?: WebUIRuntimeGlobal;
    __webuiRegisterComponentStyles?: (value: unknown) => Promise<void> | undefined;
    __webuiPrepareTemplates?: (templates: Record<string, unknown>) => void;
    __webuiPrepareResources?: (
      ids: readonly string[],
      priority?: RequestPriority,
    ) => Promise<void> | undefined;
    __webuiCollectTemplates?: (keep?: readonly string[]) => number;
    __webuiRetainTemplate?: (tag: string) => unknown;
  }
}

/**
 * Public type definitions for @microsoft/webui-delivery.
 */

export type ComponentStyleResource = (
  | { kind: 'link'; href: string }
  | { kind: 'style'; css: string }
  | { kind: 'module'; specifier: string; css: string }
) & {
  /** Component resource IDs whose rules this bundled resource covers. */
  members?: string[];
};

export interface ComponentStyles {
  version: 1;
  strategy: 'link' | 'style' | 'module';
  resources: Record<string, ComponentStyleResource>;
  closures: Record<string, string[]>;
}

/** Configure and select routing capabilities once in `WebUIRouter.start()`. */
export interface RouterConfig {
  /**
   * Optional lazy-loading map: component tag → async loader function.
   *
   * When a route's `component` attribute matches a key in this map, the
   * loader is called before the component is mounted. The loader should
   * dynamically import the component's JS module (which registers the
   * custom element via `define`).
   *
   * Components NOT in this map are assumed to be eagerly loaded (existing
   * behavior). Each loader runs at most once — the promise is cached.
   *
   * @example
   * ```ts
   * router.start({
   *   loaders: {
   *     'user-detail': () => import('./pages/user-detail.js'),
   *     'user-settings': () => import('./pages/user-settings.js'),
   *   },
   * });
   * ```
   */
  loaders?: Record<string, () => Promise<unknown>>;

  /** JSON by default. Select ndjson() for two-phase partial responses. */
  transport?: PartialTransport;
  /** Deadline for the whole partial response, including deferred state. */
  timeoutMs?: number;
  /** Optional compiler-authored pending/error presentation. */
  boundaries?: RouteBoundaries;
  /** Optional same-document view transitions. */
  transitions?: RouteTransitions;
  /** Optional route diagnostics; omitted implementations are not installed. */
  diagnostics?: (chain: RouteChainEntry[]) => void;

  /**
   * URL for the component template endpoint used by `router.ensureLoaded()`.
   * Component tags are appended as a comma-separated `t=` query parameter.
   *
   * @default "/_webui/templates"
   * @example
   * ```ts
   * router.start({ templateEndpoint: '/api/templates' });
   * // ensureLoaded fetches: /api/templates?t=tag1,tag2&inv=...
   * ```
   */
  templateEndpoint?: string;

  /**
   * Preload routes on link hover. When enabled, the router listens for
   * pointer events on internal `<a>` links and speculatively fetches the
   * JSON partial response. If the user clicks the link, the cached result
   * is used instantly — eliminating the navigation fetch latency.
   *
   * Only mouse pointers trigger preload (touch taps fire too late to benefit).
   *
   * @default undefined
   * @example
   * ```ts
   * router.start({ cache: navigationCache(), preload: routePreload() });
   * ```
   */
  preload?: RoutePreload;

  /**
   * Intercept same-origin POST forms and dispatch component `static action()`
   * handlers. Disabled by default so the core router does not load the action
   * runtime unless an app opts into route actions.
   *
   * @default undefined
   */
  actions?: (context: ActionContext) => () => void;

  /**
   * Path prefixes that the router should NOT intercept.
   * Navigations to these paths are handled as full-page loads by the browser.
   * Useful for server-side auth endpoints (e.g. `/auth/`) that are not SPA routes.
   *
   * @default []
   * @example
   * ```ts
   * router.start({ excludePaths: ['/auth/', '/api/'] });
   * // Navigating to /auth/logout does a full-page load instead of SPA fetch
   * ```
   */
  excludePaths?: string[];

  /**
   * Navigation cache configuration. When enabled, partial responses are
   * cached by path and tagged with server-provided cache tags for
   * tag-based invalidation.
   *
   * @default undefined (caching disabled — staleTime defaults to 0)
   * @example
   * ```ts
   * router.start({
   *   cache: navigationCache({ staleTime: 30_000, gcTime: 300_000, maxEntries: 50 }),
   * });
   * ```
   */
  cache?: NavigationCache;

  /**
   * Skip running route loaders on the initial SSR-bootstrapped navigation.
   * When true, the server-rendered state is considered authoritative.
   * Loaders still run on subsequent client-side navigations.
   * Components that declare `static ssrLoader = true` still have their
   * loader invoked even when this is enabled.
   *
   * @default true
   */
  ssrFresh?: boolean;
}

/**
 * Context passed to a component's static `loader()` method.
 *
 * Route loaders let components fetch their own data instead of using
 * the server-provided state. The router calls the loader during
 * navigation (before the view transition) and passes the result to
 * `setState()`.
 */
export interface RouteLoaderContext {
  /** Bound route parameters (e.g. `{ id: '42' }` for `/contacts/:id`). */
  params: Record<string, string>;
  /** Parsed query-string parameters. */
  query: Record<string, string>;
  /** Abort signal tied to the navigation — cancelled if the user navigates away. */
  signal: AbortSignal;
}

/** Detail payload of the `webui:route:navigated` CustomEvent. */
export interface NavigationEvent {
  component: string;
  params: Record<string, string>;
  /** Parsed query-string parameters (e.g. `?action=reply&to=x` → `{ action: 'reply', to: 'x' }`). */
  query: Record<string, string>;
  /** The navigated path, including the query string when present. */
  path: string;
}

/** Configuration for the router's navigation cache. */
export interface CacheConfig {
  /**
   * Maximum age (ms) before a cached response is considered stale and refetched.
   * @default 0 (always fresh — caching disabled)
   */
  staleTime?: number;
  /**
   * Maximum age (ms) before a cached entry is evicted from memory.
   * @default 300000 (5 minutes)
   */
  gcTime?: number;
  /**
   * Maximum number of entries in the cache. Evicts LRU when exceeded.
   * @default 50
   */
  maxEntries?: number;
}

/**
 * Context passed to a component's static `action()` method.
 *
 * Route actions handle form submissions (the write counterpart to loaders).
 * The router intercepts `<form method="post">` submissions and calls the
 * nearest route component's `static action()`.
 */
export interface RouteActionContext {
  /** The submitted form data. */
  formData: FormData;
  /** Bound route parameters (e.g. `{ id: '42' }` for `/contacts/:id`). */
  params: Record<string, string>;
  /** Abort signal — cancelled if the user navigates away during the action. */
  signal: AbortSignal;
}

/**
 * Result returned from a component's static `action()` method.
 *
 * The router uses this to determine what to invalidate and optionally
 * apply optimistic state updates.
 */
export interface RouteActionResult {
  /**
   * Tags to invalidate after the action completes.
   * Merged with the route's `invalidates` attribute (declared at build time).
   */
  invalidateTags?: string[];
  /**
   * Optimistic state to apply immediately (before server confirmation).
   * Passed to the component's `setState()`.
   */
  state?: Record<string, unknown>;
}

/** Detail payload of the `webui:route:action-complete` CustomEvent. */
export interface ActionCompleteEvent {
  /** The component tag that handled the action. */
  component: string;
  /** Tags that were invalidated. */
  invalidatedTags: string[];
  /** The route path where the action occurred. */
  path: string;
}

/**
 * An HTML element that supports WebUI's `setState()` protocol.
 * Used by the router to update component state during navigation.
 */
export interface StatefulElement extends HTMLElement {
  setState(state: Record<string, unknown>): void;
}

/** Type guard for elements that implement setState. */
export function isStateful(el: Element): el is StatefulElement {
  return typeof (el as StatefulElement).setState === 'function';
}

// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

declare global {
  interface Window {
    __webui_templates?: Record<string, unknown>;
  }
}

/**
 * Public type definitions for @microsoft/webui-router.
 */

/** Configuration passed to `Router.start()`. */
export interface RouterConfig {
  /**
   * Base path prepended to all route URLs.
   * @default ""
   */
  basePath?: string;

  /**
   * Optional lazy-loading map: component tag → async loader function.
   *
   * When a route's `component` attribute matches a key in this map, the
   * loader is called before the component is mounted. The loader should
   * dynamically import the component's JS module (which registers the
   * custom element via `defineAsync`).
   *
   * Components NOT in this map are assumed to be eagerly loaded (existing
   * behavior). Each loader runs at most once — the promise is cached.
   *
   * @example
   * ```ts
   * Router.start({
   *   loaders: {
   *     'user-detail': () => import('./pages/user-detail.js'),
   *     'user-settings': () => import('./pages/user-settings.js'),
   *   },
   * });
   * ```
   */
  loaders?: Record<string, () => Promise<unknown>>;

  /** Enable development mode warnings for common routing mistakes. */
  dev?: boolean;

  /**
   * URL for the component template endpoint used by `Router.ensureLoaded()`.
   * Component tags are appended as a comma-separated `t=` query parameter.
   *
   * @default "/_webui/templates"
   * @example
   * ```ts
   * Router.start({ templateEndpoint: '/api/templates' });
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
   * @default false
   * @example
   * ```ts
   * Router.start({ preload: true });
   * ```
   */
  preload?: boolean;

  /**
   * Navigation cache configuration. When enabled, partial responses are
   * cached by path and tagged with server-provided cache tags for
   * tag-based invalidation.
   *
   * @default undefined (caching disabled — staleTime defaults to 0)
   * @example
   * ```ts
   * Router.start({
   *   cache: { staleTime: 30_000, gcTime: 300_000, maxEntries: 50 },
   * });
   * ```
   */
  cache?: CacheConfig;
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

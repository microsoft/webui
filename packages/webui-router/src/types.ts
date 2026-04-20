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
   * The cache holds a single entry with a 5-second TTL.
   *
   * @default false
   * @example
   * ```ts
   * Router.start({ preload: true });
   * ```
   */
  preload?: boolean;
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

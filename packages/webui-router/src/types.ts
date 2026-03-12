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
}

/** Detail payload of the `webui:route:navigated` CustomEvent. */
export interface NavigationEvent {
  routeName: string;
  params: Record<string, string>;
  path: string;
}

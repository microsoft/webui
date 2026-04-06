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

  /**
   * Base custom element class for auto-registering SSR-only components.
   *
   * When a route's component tag is not a registered custom element but
   * has template metadata in `window.__webui_templates`, the router
   * auto-registers a bare subclass of this base. This enables pure-SSR
   * components (no client JS class) to render on SPA navigations.
   *
   * Pass `WebUIElement` from `@microsoft/webui-framework`.
   *
   * @example
   * ```ts
   * import { WebUIElement } from '@microsoft/webui-framework';
   * Router.start({ elementBase: WebUIElement });
   * ```
   */
  elementBase?: CustomElementConstructor;

  /** Enable development mode warnings for common routing mistakes. */
  dev?: boolean;
}

/** Detail payload of the `webui:route:navigated` CustomEvent. */
export interface NavigationEvent {
  component: string;
  params: Record<string, string>;
  /** The navigated path, including the query string when present. */
  path: string;
}

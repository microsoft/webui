// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

export { WebUIRouter } from './router-core.js';
export { parseQuery, filterQuery, WebUIRouteElement } from './route-element.js';
export { navigationCache } from './cache.js';
export { routeActions } from './actions.js';
export { routePreload } from './preload.js';
export { routeBoundaries } from './boundaries.js';
export { viewTransitions } from './view-transitions.js';
export { validateRoutes as routeDiagnostics } from './diagnostics.js';
export { isStateful } from './types.js';
export type { RouteBoundaries, RouteTransitions } from './capabilities.js';
export type { RoutePreload } from './preload.js';
export type {
  RouterConfig, NavigationEvent, RouteLoaderContext, CacheConfig,
  RouteActionContext, RouteActionResult, ActionCompleteEvent, StatefulElement,
} from './types.js';
export type { PartialResponse, RouteChainEntry } from './cache.js';

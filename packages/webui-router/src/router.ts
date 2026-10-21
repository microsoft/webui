// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { WebUIRouter as RouterCore } from './router-core.js';
import { findErrorComponent, findPendingComponent } from './route-boundary.js';
import { validateRoutes } from './diagnostics.js';
import { viewTransitions } from './view-transitions.js';
import type { RouterCapabilities } from './capabilities.js';

const defaultCapabilities: RouterCapabilities = {
  streaming: {
    read: async (...args) => (await import('./streaming.js')).readStreamingPartial(...args),
  },
  boundaries: {
    pending: findPendingComponent,
    error: findErrorComponent,
    create: async () => new (await import('./pending.js')).PendingState(),
  },
  transitions: viewTransitions,
  diagnostics: validateRoutes,
  cache: async config => new (await import('./cache.js')).NavigationCache(config),
  actions: async context => (await import('./actions.js')).setupFormInterception(context),
  preload: async context => (await import('./preload.js')).setupPreloadListeners(context),
};

/** Convenience router retaining the default package's existing optional behavior. */
export class WebUIRouter extends RouterCore {
  constructor() {
    super(defaultCapabilities);
  }
}

/** Singleton for applications using the default router entrypoint. */
export const Router = new WebUIRouter();

export { parseQuery, filterQuery, WebUIRouteElement } from './route-element.js';

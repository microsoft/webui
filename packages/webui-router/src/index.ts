// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

/**
 * @microsoft/webui-router — DOM-native client-side router for WebUI apps.
 *
 * Routes are `<webui-route>` custom elements (transformed from `<route>` at build time).
 * The router uses the Navigation API to intercept navigations and show/hide matching routes.
 *
 * @example
 * ```ts
 * import { Router } from '@microsoft/webui-router';
 * Router.start();
 * Router.navigate('/contacts/42');
 * ```
 *
 * @packageDocumentation
 */

export { Router, WebUIRouter, WebUIRouteElement } from './router.js';
export type { RouterConfig, NavigationEvent } from './types.js';

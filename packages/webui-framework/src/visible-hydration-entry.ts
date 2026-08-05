// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

/**
 * Public visible-hydration entry point.
 *
 * Importing this module installs the shared viewport/interaction coordinator
 * for `hydration = 'visible'` components as a one-time side effect. It is
 * published separately (as `@microsoft/webui-framework/visible-hydration.js`,
 * mirroring `@microsoft/webui-framework/streaming.js`) so the default
 * framework entry (`@microsoft/webui-framework`) carries no static or dynamic
 * dependency on the coordinator — an application with no `hydration =
 * 'visible'` components never loads `visible-hydration.js` at all, and pays
 * nothing for it.
 *
 * An application using `hydration = 'visible'` imports this entry once,
 * before its component side-effect imports, so the coordinator is installed
 * synchronously and ready before any authored `.define()` call runs in the
 * same module graph:
 *
 * ```ts
 * import '@microsoft/webui-framework/visible-hydration.js';
 * import './todo-row.js';
 * ```
 *
 * Without this entry (or without `IntersectionObserver`), a `hydration =
 * 'visible'` component falls back to eager hydration automatically; it is
 * never left inert. Installing is idempotent — importing it more than once
 * (ESM caches the module anyway) or calling the installer directly more than
 * once is a no-op after the first call.
 */

import { installVisibleHydrationCoordinator } from './visible-hydration-coordinator.js';

installVisibleHydrationCoordinator();

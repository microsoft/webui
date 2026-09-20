// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

/**
 * Public lazy-hydration entry point.
 *
 * Importing this module installs the shared viewport/interaction coordinator
 * for components compiled with lazy hydration as a one-time side effect. It is
 * published separately (as `@microsoft/webui-framework/lazy-hydration.js`,
 * mirroring `@microsoft/webui-framework/streaming.js`) so the default
 * framework entry (`@microsoft/webui-framework`) carries no static or dynamic
 * dependency on the coordinator — an application with no `hydration =
 * 'lazy'` components never loads `lazy-hydration.js` at all, and pays
 * nothing for it.
 *
 * An application using `w-hydrate="lazy"` or `w-render="lazy"` imports this entry once,
 * before its component side-effect imports, so the coordinator is installed
 * synchronously and ready before any authored `.define()` call runs in the
 * same module graph:
 *
 * ```ts
 * import '@microsoft/webui-framework/lazy-hydration.js';
 * import './todo-row.js';
 * ```
 *
 * Without this entry (or without `IntersectionObserver`), a `hydration =
 * 'lazy'` component falls back to eager hydration automatically; it is
 * never left inert. Installing is idempotent — importing it more than once
 * (ESM caches the module anyway) or calling the installer directly more than
 * once is a no-op after the first call.
 */

import { installLazyHydrationCoordinator } from './lazy-hydration-coordinator.js';

installLazyHydrationCoordinator();

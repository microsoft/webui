// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

/**
 * Speculative prefetch — preloads route data on link hover so that
 * subsequent navigations are instant (cache hit).
 */

import { stripBaseFromPathname } from './navigation-path.js';
import type { PartialResponse } from './cache.js';

/** Context needed by preload listeners to interact with router state. */
export interface PreloadContext {
  readonly basePath: string;
  readonly excludePaths: string[];
  readonly currentRequestPath: string;
  readonly inventory: string;
  hasCache(requestPath: string): boolean;
  storeCache(requestPath: string, data: PartialResponse & { inventory?: string }, preload: boolean): void;
  fetchPartial(requestPath: string, signal: AbortSignal, speculative: boolean): Promise<(PartialResponse & { inventory?: string }) | null>;
}

/**
 * Register a delegated `pointermove` listener that speculatively fetches
 * the JSON partial for internal links on mouse hover.
 *
 * Returns a cleanup function to remove the listener.
 */
export function setupPreloadListeners(ctx: PreloadContext): () => void {
  let preloadController: AbortController | null = null;
  let preloadGeneration = 0;

  const onPointerMove = (e: PointerEvent): void => {
    if (e.pointerType !== 'mouse') return;

    // Walk composedPath to find the nearest <a> — works across shadow boundaries.
    const path = e.composedPath();
    let anchor: HTMLAnchorElement | undefined;
    for (let i = 0; i < path.length; i++) {
      if ((path[i] as Element)?.tagName === 'A') {
        anchor = path[i] as HTMLAnchorElement;
        break;
      }
    }
    if (!anchor) return;

    // Use anchor's pre-parsed URL properties to avoid new URL() allocation.
    const href = anchor.getAttribute('href');
    if (!href || href.startsWith('#')) return;
    if (anchor.origin !== location.origin) return;

    // Skip excluded paths (e.g. /auth/ endpoints) — no speculative fetch.
    const anchorPathname = anchor.pathname;
    for (let i = 0; i < ctx.excludePaths.length; i++) {
      if (anchorPathname.startsWith(ctx.excludePaths[i])) return;
    }

    // Build request path from anchor properties — no URL allocation needed.
    const stripped = stripBaseFromPathname(anchor.pathname, ctx.basePath);
    const requestPath = (stripped + anchor.search) || '/';

    // Skip if already on this path or already cached for it
    if (requestPath === ctx.currentRequestPath) return;
    if (ctx.hasCache(requestPath)) return;

    // Abort any in-flight speculative fetch and start a new one
    preloadController?.abort();
    const controller = new AbortController();
    preloadController = controller;
    const gen = ++preloadGeneration;

    ctx.fetchPartial(requestPath, controller.signal, true)
      .then(data => {
        // Only cache if this is still the latest preload request
        if (data && gen === preloadGeneration && !controller.signal.aborted) {
          ctx.storeCache(requestPath, data, true);
        }
      })
      .catch(() => {}); // Speculative — silently discard errors
  };

  document.addEventListener('pointermove', onPointerMove);
  return () => {
    document.removeEventListener('pointermove', onPointerMove);
    preloadController?.abort();
  };
}

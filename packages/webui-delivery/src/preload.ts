// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

/**
 * Speculative prefetch — preloads route data on link hover so that
 * subsequent navigations are instant (cache hit).
 */

import { pointerPreloadPath } from './preload-path.js';
import { PreparedRoutePreload, type PreparedRoutePreloadOptions } from './prepared-preload.js';
import type { PartialResponse } from './cache.js';
import type { StreamingPartialResponse } from './streaming.js';
import { cancelDeferredStream, startDeferredStream } from './partial-response.js';

/** One intent policy, optionally prepared before application activation and adopted at start. */
export interface RoutePreload {
  readonly prepared: PreparedRoutePreload | undefined;
  prepare(): void;
  destroy(): void;
  install(context: PreloadContext): () => void;
}

/** Select intent preloading; call prepare() only to preload before router startup. */
export function routePreload(options: PreparedRoutePreloadOptions = {}): RoutePreload {
  let prepared: PreparedRoutePreload | undefined;
  let active: (() => void) | undefined;
  const destroy = (): void => {
    active?.();
    active = undefined;
    prepared?.destroy();
    prepared = undefined;
  };
  return {
    get prepared() { return prepared; },
    prepare() {
      if (active) throw new Error('[Router] Prepare intent preloading before router.start(), not after it.');
      prepared ??= new PreparedRoutePreload(options);
    },
    destroy,
    install(context) {
      if (active) throw new Error('[Router] A preload policy is already installed.');
      prepared?.detach();
      active = setupPreloadListeners(context);
      return destroy;
    },
  };
}

/** Context needed by preload listeners to interact with router state. */
export interface PreloadContext {
  readonly basePath: string;
  readonly excludePaths: string[];
  readonly currentRequestPath: string;
  readonly inventory: string;
  hasCache(requestPath: string): boolean;
  storeCache(
    requestPath: string,
    data: PartialResponse & { inventory?: string },
    preload: boolean,
    streaming: boolean,
  ): void;
  fetchPartial(requestPath: string, signal: AbortSignal, speculative: boolean): Promise<StreamingPartialResponse | null>;
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
  let preloadPath: string | null = null;

  const onPointerMove = (e: PointerEvent): void => {
    const requestPath = pointerPreloadPath(e, ctx.basePath, ctx.excludePaths);
    if (!requestPath || requestPath === preloadPath) return;

    // Skip if already on this path or already cached for it
    if (requestPath === ctx.currentRequestPath) return;
    if (ctx.hasCache(requestPath)) return;

    // Abort any in-flight speculative fetch and start a new one
    preloadController?.abort();
    const controller = new AbortController();
    preloadController = controller;
    preloadPath = requestPath;
    const gen = ++preloadGeneration;

    ctx.fetchPartial(requestPath, controller.signal, true)
      .then(async data => {
        // Only cache if this is still the latest preload request
        if (data && gen === preloadGeneration && !controller.signal.aborted) {
          ctx.storeCache(
            requestPath,
            data,
            true,
            data._deferredStream === true,
          );
        }
        if (!data?._deferredStream) return;

        if (gen === preloadGeneration && !controller.signal.aborted) {
          startDeferredStream(data);
        } else {
          await cancelDeferredStream(data);
        }
      })
      .catch(() => {}) // Speculative — silently discard errors
      .finally(() => {
        if (gen === preloadGeneration) preloadPath = null;
      });
  };

  document.addEventListener('pointermove', onPointerMove);
  return () => {
    document.removeEventListener('pointermove', onPointerMove);
    preloadController?.abort();
  };
}

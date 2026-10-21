// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import type { ActionContext } from './actions.js';
import type { NavigationCache } from './cache.js';
import type { PendingState } from './pending.js';
import type { PreloadContext } from './preload.js';
import type { RouteChainEntry } from './cache.js';
import type { RouteBoundary } from './route-boundary.js';
import type { StreamingContext, StreamingPartialResponse } from './streaming.js';
import type { CacheConfig } from './types.js';

/** Optional route-state streaming; absent means JSON-only content negotiation. */
export interface RouteStreaming {
  read(
    response: Response,
    requestPath: string,
    context: StreamingContext,
    signal?: AbortSignal,
  ): Promise<StreamingPartialResponse | null>;
}

/** Optional presentation policy for compiled route pending/error declarations. */
export interface RouteBoundaries {
  pending(chain: RouteChainEntry[], path: string): RouteBoundary | null;
  error(chain: RouteChainEntry[], path: string): RouteBoundary | null;
  create(): PendingState | Promise<PendingState>;
}

/** Optional same-document transition policy, independent of navigation correctness. */
export interface RouteTransitions {
  supported(): boolean;
  install(): () => void;
  startViewTransition(commit: () => void): ViewTransition;
}

/**
 * Explicit capabilities for the lean router entry.
 *
 * Import only the implementations the application selects. Configuration flags
 * such as actions/cache/preload still control their activation and options.
 * These coarse request/lifecycle adapters are not per-binding rendering hooks.
 */
export interface RouterCapabilities {
  streaming?: RouteStreaming;
  boundaries?: RouteBoundaries;
  transitions?: RouteTransitions;
  cache?: (config: Required<CacheConfig>) => NavigationCache | Promise<NavigationCache>;
  actions?: (context: ActionContext) => (() => void) | Promise<() => void>;
  preload?: (context: PreloadContext) => (() => void) | Promise<() => void>;
  diagnostics?: (chain: RouteChainEntry[]) => void;
}

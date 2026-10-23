// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import type { PendingState } from './pending.js';
import type { RouteChainEntry } from './cache.js';
import type { RouteBoundary } from './route-boundary.js';

/** Optional presentation policy for compiled route pending/error declarations. */
export interface RouteBoundaries {
  pending(chain: RouteChainEntry[], path: string): RouteBoundary | null;
  error(chain: RouteChainEntry[], path: string): RouteBoundary | null;
  create(): PendingState;
}

/** Optional same-document transition policy, independent of navigation correctness. */
export interface RouteTransitions {
  supported(): boolean;
  install(): () => void;
  startViewTransition(commit: () => void): ViewTransition;
}

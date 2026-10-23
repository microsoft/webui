// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { PendingState } from './pending.js';
import { findErrorComponent, findPendingComponent } from './route-boundary.js';
import type { RouteBoundaries } from './capabilities.js';

/** Opt-in pending/error UI for compiler-authored route boundaries. */
export const routeBoundaries: RouteBoundaries = {
  pending: findPendingComponent,
  error: findErrorComponent,
  create: () => new PendingState(),
};

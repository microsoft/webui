// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import {
  abandonDeferredRange,
} from './streaming-cleanup.js';
import {
  activateDeferredTree,
} from './streaming-deferred.js';
import type {
  PendingBoundaryUpdates,
} from './streaming-deferred.js';

/**
 * Activate marked roots in one allocation-free, parent-first range walk.
 *
 * Undefined outer roots are barriers. Their retained descendants are counted
 * against the same work limits but are activated only after the outer defines.
 */
export function activateRootsBetween(
  startMarker: Comment,
  endMarker: Comment,
  state: Record<string, unknown> | undefined,
  updates?: PendingBoundaryUpdates,
  bypassSpanInstanceId?: number,
): void {
  const root = startMarker.parentNode;
  if (!root) {
    throw new Error(
      'boundary start marker was detached before activation',
    );
  }

  const failure = activateDeferredTree(
    startMarker.nextSibling,
    root,
    endMarker,
    state,
    updates || bypassSpanInstanceId !== undefined
      ? {
        updates,
        countRetention: updates !== undefined,
        bypassSpanInstanceId,
      }
      : undefined,
  );
  if (!failure) return;
  abandonDeferredRange(startMarker, endMarker);
  throw new Error(failure);
}

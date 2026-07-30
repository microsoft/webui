// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

/**
 * Progressive streaming hydration facade.
 *
 * The runtime is split by concern so protocol parsing, DOM range resolution,
 * deferred activation, bootstrap merging, and installation stay independently
 * reviewable while bundling into the same opt-in streaming entry.
 */

import {
  resetStreamingCoordinatorStateForTests,
} from './streaming-coordinator.js';
import { resetStreamingInstallForTests } from './streaming-install.js';

export {
  enqueueStreamingSentinel as __enqueueSentinelForTests,
  elementHasPendingStateForTests as __elementHasPendingStateForTests,
  installStreamingTruncationGuard as __installTruncationGuardForTests,
  isStreamingHaltedForTests as __isHaltedForTests,
  pendingTagWaiterCountForTests as __pendingTagWaiterCountForTests,
  pendingUndefinedRootCountForTests as __pendingUndefinedRootCountForTests,
} from './streaming-coordinator.js';
export { installStreamingCoordinator } from './streaming-install.js';
export { parseBoundaryEnvelope } from './streaming-protocol.js';
export type {
  BoundaryBootstrap,
  BoundaryEnvelope,
  ParseBoundaryEnvelopeResult,
} from './streaming-protocol.js';

/** Reset every document-scoped streaming singleton between pipeline tests. */
export function __resetStreamingCoordinatorForTests(): void {
  resetStreamingCoordinatorStateForTests();
  resetStreamingInstallForTests();
}

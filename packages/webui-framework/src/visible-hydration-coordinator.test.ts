// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { strict as assert } from 'node:assert';
import { describe, test } from 'node:test';

const {
  observeLazyHydration,
  registerVisibleHydrationCoordinator,
  isVisibleHydrationCoordinatorInstalled,
  __resetLazyHydrationContractForTests,
} = await import('./lazy-hydration.js');
const {
  installVisibleHydrationCoordinator,
  __resetVisibleHydrationCoordinatorForTests,
} = await import('./visible-hydration-coordinator.js');

describe('installVisibleHydrationCoordinator — idempotency', () => {
  test('a repeated call never re-registers, so a later manual registration survives it', () => {
    __resetLazyHydrationContractForTests();
    __resetVisibleHydrationCoordinatorForTests();

    // First import/call: installs the real coordinator implementation.
    installVisibleHydrationCoordinator();
    assert.equal(isVisibleHydrationCoordinatorInstalled(), true);

    // Swap in a distinguishable fake, standing in for whatever a duplicate
    // module evaluation would observe through the shared contract module.
    let observeCalls = 0;
    registerVisibleHydrationCoordinator({
      supportsVisibleHydration: () => true,
      observe: () => {
        observeCalls++;
      },
      observeStreamed: () => {},
      disconnect: () => {},
      isStreamedActivation: () => false,
    });

    // A second `installVisibleHydrationCoordinator()` call — e.g. from the
    // optional entry being reachable twice in a graph — must be a no-op. If
    // it were not idempotent, it would re-register the real implementation
    // here and silently discard the fake above.
    installVisibleHydrationCoordinator();

    observeLazyHydration({} as unknown as Parameters<typeof observeLazyHydration>[0]);
    assert.equal(observeCalls, 1, 'the fake registered after the first install must still be active');
  });
});

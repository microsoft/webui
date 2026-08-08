// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { strict as assert } from 'node:assert';
import { describe, test } from 'node:test';

function replaceGlobal(name: string, value: unknown): () => void {
  const previous = Object.getOwnPropertyDescriptor(globalThis, name);
  Object.defineProperty(globalThis, name, {
    value,
    configurable: true,
    writable: true,
  });
  return () => {
    if (previous) {
      Object.defineProperty(globalThis, name, previous);
    } else {
      Reflect.deleteProperty(globalThis, name);
    }
  };
}

describe('installLazyHydrationCoordinator — idempotency', () => {
  test('seals startup observation at load when DOMContentLoaded was missed', async () => {
    let resetContract: (() => void) | undefined;
    let resetCoordinator: (() => void) | undefined;

    class FakeDocument extends EventTarget {
      readonly readyState = 'interactive';
    }
    class FakeIntersectionObserver {
      observe(): void {}
      unobserve(): void {}
    }
    const fakeWindow = new EventTarget();
    let completionCount = 0;
    fakeWindow.addEventListener('webui:hydration-complete', () => {
      completionCount++;
    });
    const restoreGlobals = [
      replaceGlobal('Document', FakeDocument),
      replaceGlobal('document', new FakeDocument()),
      replaceGlobal('IntersectionObserver', FakeIntersectionObserver),
      replaceGlobal('performance', {
        getEntriesByType: () => [],
        mark() {},
        measure() {},
        now: () => 0,
      }),
      replaceGlobal('window', fakeWindow),
    ];

    try {
      const contract = await import('./lazy-hydration-contract.js');
      const coordinator = await import('./lazy-hydration-coordinator.js');
      resetContract = contract.__resetLazyHydrationContractForTests;
      resetCoordinator = coordinator.__resetLazyHydrationCoordinatorForTests;
      coordinator.installLazyHydrationCoordinator();

      const target = {
        isConnected: true,
        [contract.LAZY_HYDRATION_ACTIVATE]() {},
      } as unknown as Parameters<typeof contract.observeLazyHydration>[0];
      contract.observeLazyHydration(
        target,
        contract.LAZY_HYDRATION_VIEWPORT,
      );
      contract.disconnectLazyHydration(target);
      assert.equal(completionCount, 0);
      fakeWindow.dispatchEvent(new Event('load'));
      assert.equal(completionCount, 1);
    } finally {
      resetContract?.();
      resetCoordinator?.();
      for (let i = restoreGlobals.length - 1; i >= 0; i--) {
        restoreGlobals[i]();
      }
    }
  });

  test('a repeated call never re-registers, so a later manual registration survives it', async () => {
    const {
      observeLazyHydration,
      registerLazyHydrationCoordinator,
      isLazyHydrationCoordinatorInstalled,
      __resetLazyHydrationContractForTests,
    } = await import('./lazy-hydration-contract.js');
    const {
      installLazyHydrationCoordinator,
      __resetLazyHydrationCoordinatorForTests,
    } = await import('./lazy-hydration-coordinator.js');
    __resetLazyHydrationContractForTests();
    __resetLazyHydrationCoordinatorForTests();

    // First import/call: installs the real coordinator implementation.
    installLazyHydrationCoordinator();
    assert.equal(isLazyHydrationCoordinatorInstalled(), true);

    // Swap in a distinguishable fake, standing in for whatever a duplicate
    // module evaluation would observe through the shared contract module.
    let observeCalls = 0;
    registerLazyHydrationCoordinator({
      supportsLazyHydration: () => true,
      observe: () => {
        observeCalls++;
      },
      observeStreamed: () => {},
      disconnect: () => {},
      isStreamedActivation: () => false,
    });

    // A second `installLazyHydrationCoordinator()` call — e.g. from the
    // optional entry being reachable twice in a graph — must be a no-op. If
    // it were not idempotent, it would re-register the real implementation
    // here and silently discard the fake above.
    installLazyHydrationCoordinator();

    observeLazyHydration(
      {} as unknown as Parameters<typeof observeLazyHydration>[0],
      1,
    );
    assert.equal(observeCalls, 1, 'the fake registered after the first install must still be active');
  });
});

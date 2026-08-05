// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { strict as assert } from 'node:assert';
import { describe, test } from 'node:test';

/**
 * `lifecycle.ts` keeps its counters as module-level mutable state (by
 * design — see its own doc comment). Each test below imports it via a
 * unique query string so Node's ESM loader treats it as a fresh module
 * instance with its own state, instead of sharing counters across tests.
 */
async function freshLifecycle(): Promise<typeof import('./lifecycle.js')> {
  return import(`./lifecycle.js?case=${Math.random()}`);
}

function withPerformanceAndWindow<T>(run: (dispatched: Event[]) => T): T {
  const previousPerformance = Object.getOwnPropertyDescriptor(globalThis, 'performance');
  const previousWindow = Object.getOwnPropertyDescriptor(globalThis, 'window');
  const previousEvent = Object.getOwnPropertyDescriptor(globalThis, 'Event');
  const marks = new Set<string>();
  const dispatched: Event[] = [];

  class FakeEvent {
    type: string;
    constructor(type: string) {
      this.type = type;
    }
  }

  Object.defineProperty(globalThis, 'Event', { value: FakeEvent, configurable: true, writable: true });
  Object.defineProperty(globalThis, 'performance', {
    value: {
      mark(name: string) {
        marks.add(name);
      },
      // Mirrors the real Performance API: measuring against a mark that was
      // never placed throws — this is what makes the `if (started)` guard
      // in tryComplete() load-bearing for a component-less streaming page.
      measure(_name: string, start: string, end: string) {
        if (!marks.has(start) || !marks.has(end)) {
          throw new Error(`Failed to execute 'measure': the mark '${start}' does not exist`);
        }
      },
    },
    configurable: true,
    writable: true,
  });
  Object.defineProperty(globalThis, 'window', {
    value: {
      dispatchEvent(event: Event) {
        dispatched.push(event);
        return true;
      },
    },
    configurable: true,
    writable: true,
  });

  try {
    return run(dispatched);
  } finally {
    for (const [key, descriptor] of [
      ['performance', previousPerformance],
      ['window', previousWindow],
      ['Event', previousEvent],
    ] as const) {
      if (descriptor) {
        Object.defineProperty(globalThis, key, descriptor);
      } else {
        Reflect.deleteProperty(globalThis, key);
      }
    }
  }
}

describe('hydration lifecycle — non-streaming pages', () => {
  test('treats an interactive document as ready after DOMContentLoaded', async () => {
    const previousDocumentConstructor = Object.getOwnPropertyDescriptor(globalThis, 'Document');
    const previousDocument = Object.getOwnPropertyDescriptor(globalThis, 'document');
    const previousPerformance = Object.getOwnPropertyDescriptor(globalThis, 'performance');
    const previousWindow = Object.getOwnPropertyDescriptor(globalThis, 'window');
    const previousEvent = Object.getOwnPropertyDescriptor(globalThis, 'Event');
    const dispatched: Event[] = [];

    class FakeDocument {}
    const fakeDocument = new FakeDocument() as FakeDocument & {
      readyState: DocumentReadyState;
      addEventListener(): void;
    };
    fakeDocument.readyState = 'interactive';
    fakeDocument.addEventListener = () => {
      throw new Error('DOMContentLoaded must not be awaited after it has fired');
    };

    class FakeEvent {
      type: string;
      constructor(type: string) {
        this.type = type;
      }
    }

    Object.defineProperty(globalThis, 'Document', {
      value: FakeDocument,
      configurable: true,
      writable: true,
    });
    Object.defineProperty(globalThis, 'document', {
      value: fakeDocument,
      configurable: true,
      writable: true,
    });
    Object.defineProperty(globalThis, 'performance', {
      value: {
        getEntriesByType() {
          return [{ domContentLoadedEventStart: 1 }];
        },
        mark() {},
        measure() {},
      },
      configurable: true,
      writable: true,
    });
    Object.defineProperty(globalThis, 'window', {
      value: {
        dispatchEvent(event: Event) {
          dispatched.push(event);
          return true;
        },
      },
      configurable: true,
      writable: true,
    });
    Object.defineProperty(globalThis, 'Event', {
      value: FakeEvent,
      configurable: true,
      writable: true,
    });

    try {
      const lifecycle = await freshLifecycle();
      assert.equal(lifecycle.isHydrationStartupPending(), false);
      lifecycle.hydrationStart();
      lifecycle.hydrationEnd();
      assert.equal(dispatched.length, 1);
      assert.equal(dispatched[0].type, 'webui:hydration-complete');
    } finally {
      for (const [key, descriptor] of [
        ['Document', previousDocumentConstructor],
        ['document', previousDocument],
        ['performance', previousPerformance],
        ['window', previousWindow],
        ['Event', previousEvent],
      ] as const) {
        if (descriptor) {
          Object.defineProperty(globalThis, key, descriptor);
        } else {
          Reflect.deleteProperty(globalThis, key);
        }
      }
    }
  });

  test('fires webui:hydration-complete once pending count reaches zero', async () => {
    const lifecycle = await freshLifecycle();
    withPerformanceAndWindow((dispatched) => {
      lifecycle.hydrationStart();
      lifecycle.hydrationStart();
      assert.equal(dispatched.length, 0);
      lifecycle.hydrationEnd();
      assert.equal(dispatched.length, 0, 'must wait for every started component');
      lifecycle.hydrationEnd();
      assert.equal(dispatched.length, 1);
      assert.equal(dispatched[0].type, 'webui:hydration-complete');
    });
  });

  test('fires only once even if hydrationEnd is called again', async () => {
    const lifecycle = await freshLifecycle();
    withPerformanceAndWindow((dispatched) => {
      lifecycle.hydrationStart();
      lifecycle.hydrationEnd();
      lifecycle.hydrationEnd();
      assert.equal(dispatched.length, 1);
      assert.equal(lifecycle.__getLifecycleStateForTests().pendingCount, 0);
    });
  });
});

describe('hydration lifecycle — streaming gate', () => {
  test('does not fire once components finish if the terminal boundary has not committed', async () => {
    const lifecycle = await freshLifecycle();
    withPerformanceAndWindow((dispatched) => {
      lifecycle.beginStreamingGate();
      lifecycle.markBoundaryPending();
      lifecycle.hydrationStart();
      lifecycle.hydrationEnd();
      lifecycle.markBoundaryCommitted(false);
      assert.equal(dispatched.length, 0, 'non-terminal boundary commit must not complete hydration');
    });
  });

  test('fires once the terminal boundary commits with no pending boundaries left', async () => {
    const lifecycle = await freshLifecycle();
    withPerformanceAndWindow((dispatched) => {
      lifecycle.beginStreamingGate();

      lifecycle.markBoundaryPending();
      lifecycle.hydrationStart();
      lifecycle.hydrationEnd();
      lifecycle.markBoundaryCommitted(false);
      assert.equal(dispatched.length, 0);

      lifecycle.markBoundaryPending();
      lifecycle.markBoundaryCommitted(true);
      assert.equal(dispatched.length, 1);
      assert.equal(dispatched[0].type, 'webui:hydration-complete');
    });
  });

  test('waits for a still-pending boundary even after the terminal boundary commits', async () => {
    const lifecycle = await freshLifecycle();
    withPerformanceAndWindow((dispatched) => {
      lifecycle.beginStreamingGate();

      lifecycle.markBoundaryPending(); // boundary 0: still being walked
      lifecycle.markBoundaryPending(); // boundary 1 (terminal): commits first
      lifecycle.markBoundaryCommitted(true);
      assert.equal(dispatched.length, 0, 'boundary 0 has not finished committing yet');

      lifecycle.markBoundaryCommitted(false);
      assert.equal(dispatched.length, 1);
    });
  });

  test('a streaming page with no components reaches terminal without a start mark', async () => {
    const lifecycle = await freshLifecycle();
    withPerformanceAndWindow((dispatched) => {
      lifecycle.beginStreamingGate();
      lifecycle.markBoundaryPending();
      // No hydrationStart()/hydrationEnd() calls — no components on this page.
      assert.doesNotThrow(() => lifecycle.markBoundaryCommitted(true));
      assert.equal(dispatched.length, 1);
    });
  });

  test('unmatched completion signals do not underflow or open the gate', async () => {
    const lifecycle = await freshLifecycle();
    withPerformanceAndWindow((dispatched) => {
      lifecycle.beginStreamingGate();
      lifecycle.markBoundaryCommitted(true);
      lifecycle.settleLateActivation();

      const state = lifecycle.__getLifecycleStateForTests();
      assert.equal(state.pendingBoundaries, 0);
      assert.equal(state.pendingLateActivations, 0);
      assert.equal(state.terminalReached, false);
      assert.equal(dispatched.length, 0);
    });
  });
});

describe('hydration lifecycle — late activation gate', () => {
  test('does not fire while a late activation is still pending after terminal', async () => {
    const lifecycle = await freshLifecycle();
    withPerformanceAndWindow((dispatched) => {
      lifecycle.beginStreamingGate();

      // A boundary committed but one of its roots is waiting on a
      // not-yet-defined class, so the coordinator opened a late-activation.
      lifecycle.markBoundaryPending();
      lifecycle.markLateActivationPending();
      lifecycle.markBoundaryCommitted(true);
      assert.equal(dispatched.length, 0, 'terminal alone must not complete while a late activation is pending');

      // Once the class defines and its roots activate, completion fires.
      lifecycle.settleLateActivation();
      assert.equal(dispatched.length, 1);
      assert.equal(dispatched[0].type, 'webui:hydration-complete');
    });
  });

  test('waits for every distinct late activation before firing', async () => {
    const lifecycle = await freshLifecycle();
    withPerformanceAndWindow((dispatched) => {
      lifecycle.beginStreamingGate();

      lifecycle.markBoundaryPending();
      lifecycle.markLateActivationPending(); // tag A undefined
      lifecycle.markLateActivationPending(); // tag B undefined
      lifecycle.markBoundaryCommitted(true);
      assert.equal(dispatched.length, 0);

      lifecycle.settleLateActivation(); // A defines
      assert.equal(dispatched.length, 0, 'still waiting on the second undefined tag');

      lifecycle.settleLateActivation(); // B defines
      assert.equal(dispatched.length, 1);
    });
  });

  test('a late activation settling before terminal does not fire early', async () => {
    const lifecycle = await freshLifecycle();
    withPerformanceAndWindow((dispatched) => {
      lifecycle.beginStreamingGate();

      lifecycle.markBoundaryPending();
      lifecycle.markLateActivationPending();
      lifecycle.markBoundaryCommitted(false);
      lifecycle.settleLateActivation();
      assert.equal(dispatched.length, 0, 'terminal boundary has not committed yet');

      lifecycle.markBoundaryPending();
      lifecycle.markBoundaryCommitted(true);
      assert.equal(dispatched.length, 1);
    });
  });
});

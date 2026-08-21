// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { strict as assert } from 'node:assert';
import { describe, test } from 'node:test';

import {
  ACTIVATION_ACTIVATED,
  ACTIVATION_ANCESTOR_BARRIER,
  ACTIVATION_MISSING_TEMPLATE,
  ACTIVATION_STATIC_HOST_OPT_OUT,
  isStreamingHydrationMode,
  resetStreamingModeForTests,
} from './streaming-mode.js';
import type { ActivationOutcome } from './streaming-mode.js';
import {
  ELEMENT_ACTIVATED_FROM_PENDING,
  ELEMENT_BARRIER_LIMIT_FAILURE,
  ELEMENT_DEFERRED,
  ELEMENT_IGNORED,
  ELEMENT_INVALID_OUTCOME,
  ELEMENT_LIMIT_FAILURE,
} from './streaming-deferred.js';

function withDocument<T>(meta: string | null, run: () => T): T {
  const previousDocument = Object.getOwnPropertyDescriptor(globalThis, 'document');
  let queries = 0;
  Object.defineProperty(globalThis, 'document', {
    value: {
      querySelector(selector: string) {
        queries++;
        assert.equal(selector, 'meta[name="webui-streaming"][content="1"]');
        return meta ? { getAttribute: () => meta } : null;
      },
    },
    configurable: true,
    writable: true,
  });
  try {
    return run();
  } finally {
    if (previousDocument) {
      Object.defineProperty(globalThis, 'document', previousDocument);
    } else {
      Reflect.deleteProperty(globalThis, 'document');
    }
    assert.ok(queries <= 1, `expected at most one document.querySelector call, got ${queries}`);
  }
}

describe('streaming-mode detection', () => {
  test('detects streaming mode from the meta tag', () => {
    resetStreamingModeForTests();
    withDocument('1', () => {
      assert.equal(isStreamingHydrationMode(), true);
    });
  });

  test('defaults to non-streaming when the meta tag is absent', () => {
    resetStreamingModeForTests();
    withDocument(null, () => {
      assert.equal(isStreamingHydrationMode(), false);
    });
  });

  test('caches the result across calls — queries the document at most once', () => {
    resetStreamingModeForTests();
    withDocument('1', () => {
      assert.equal(isStreamingHydrationMode(), true);
      assert.equal(isStreamingHydrationMode(), true);
      assert.equal(isStreamingHydrationMode(), true);
    });
  });

  test('resetStreamingModeForTests forces a fresh detection', () => {
    resetStreamingModeForTests();
    withDocument('1', () => {
      assert.equal(isStreamingHydrationMode(), true);
    });
    resetStreamingModeForTests();
    withDocument(null, () => {
      assert.equal(isStreamingHydrationMode(), false);
    });
  });
});

/**
 * The `STREAMING_BOUNDARY_ACTIVATE` outcome contract.
 *
 * These codes are the wire between the producer (`template-element.ts`) and the
 * consumer (`streaming-deferred.ts`). They used to be declared twice, so the
 * two copies could drift silently, and the activation space overlapped the
 * coordinator's own element-walk results — `ACTIVATION_ANCESTOR_BARRIER` and
 * `ELEMENT_DEFERRED` were both `4`. Both spaces still travel through one
 * `number`, so the disjointness below is what keeps them decodable.
 */
describe('activation outcome contract', () => {
  const ACTIVATION_OUTCOMES: ReadonlyArray<readonly [string, ActivationOutcome]> = [
    ['ACTIVATION_ACTIVATED', ACTIVATION_ACTIVATED],
    ['ACTIVATION_STATIC_HOST_OPT_OUT', ACTIVATION_STATIC_HOST_OPT_OUT],
    ['ACTIVATION_MISSING_TEMPLATE', ACTIVATION_MISSING_TEMPLATE],
    ['ACTIVATION_ANCESTOR_BARRIER', ACTIVATION_ANCESTOR_BARRIER],
  ];

  const ELEMENT_RESULTS: ReadonlyArray<readonly [string, number]> = [
    ['ELEMENT_IGNORED', ELEMENT_IGNORED],
    ['ELEMENT_DEFERRED', ELEMENT_DEFERRED],
    ['ELEMENT_LIMIT_FAILURE', ELEMENT_LIMIT_FAILURE],
    ['ELEMENT_ACTIVATED_FROM_PENDING', ELEMENT_ACTIVATED_FROM_PENDING],
    ['ELEMENT_BARRIER_LIMIT_FAILURE', ELEMENT_BARRIER_LIMIT_FAILURE],
    ['ELEMENT_INVALID_OUTCOME', ELEMENT_INVALID_OUTCOME],
  ];

  test('every activation outcome is a distinct non-zero integer', () => {
    const seen = new Map<number, string>();
    for (const [name, value] of ACTIVATION_OUTCOMES) {
      assert.ok(
        Number.isInteger(value) && value > 0,
        `${name} must be a non-zero integer so a hook returning nothing is rejected`,
      );
      const previous = seen.get(value);
      assert.equal(previous, undefined, `${name} duplicates ${previous} (${value})`);
      seen.set(value, name);
    }
    assert.equal(seen.size, ACTIVATION_OUTCOMES.length);
  });

  test('the activation space never collides with an element-walk result', () => {
    const activationValues = new Set<number>(
      ACTIVATION_OUTCOMES.map(([, value]) => value),
    );
    const collisions = ELEMENT_RESULTS
      .filter(([, value]) => activationValues.has(value))
      .map(([name]) => name);
    assert.deepEqual(
      collisions,
      [],
      'an element-walk result reusing an activation code makes the two indistinguishable',
    );
  });

  test('every element-walk result is itself distinct', () => {
    const seen = new Map<number, string>();
    for (const [name, value] of ELEMENT_RESULTS) {
      const previous = seen.get(value);
      assert.equal(previous, undefined, `${name} duplicates ${previous} (${value})`);
      seen.set(value, name);
    }
    assert.equal(seen.size, ELEMENT_RESULTS.length);
  });

  test('an ancestor barrier stays distinguishable from a deferred element', () => {
    assert.notEqual(
      ACTIVATION_ANCESTOR_BARRIER,
      ELEMENT_DEFERRED,
      'the coordinator branches on both from the same value; sharing 4 conflated them',
    );
  });
});

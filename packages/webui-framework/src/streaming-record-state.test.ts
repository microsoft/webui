// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { strict as assert } from 'node:assert';
import { beforeEach, describe, test } from 'node:test';

import type { BoundaryBootstrap } from './streaming-protocol.js';
import {
  clearRangeState,
  resolveRangeState,
} from './streaming-record-state.js';

function payload(
  value: Omit<BoundaryBootstrap, 'componentStyles' | 'declarationId'>,
): BoundaryBootstrap {
  return {
    declarationId: 0,
    componentStyles: {} as BoundaryBootstrap['componentStyles'],
    ...value,
  };
}

describe('streaming range state references', () => {
  beforeEach(clearRangeState);

  test('reuses an exact prior projection without aliasing component state', () => {
    const first = resolveRangeState(
      payload({ state: { todos: [{ id: 1 }] } }),
      0,
    );
    const second = resolveRangeState(payload({ stateRef: 0 }), 1);

    assert.deepEqual(second, first);
    assert.notStrictEqual(second, first);
    assert.notStrictEqual(second?.todos, first?.todos);
    const firstTodos = first?.todos as Array<{ id: number }>;
    firstTodos[0].id = 9;
    assert.deepEqual(second, { todos: [{ id: 1 }] });
  });

  test('applies a top-level delta without mutating the referenced state', () => {
    const first = resolveRangeState(
      payload({ state: { todos: [{ id: 1 }] } }),
      0,
    );
    const second = resolveRangeState(
      payload({ stateRef: 0, stateDelta: { toolbar: { active: true } } }),
      2,
    );

    assert.deepEqual(first, { todos: [{ id: 1 }] });
    assert.deepEqual(second, {
      todos: [{ id: 1 }],
      toolbar: { active: true },
    });
  });

  test('rejects missing, stale, forward, and malformed references', () => {
    assert.throws(
      () => resolveRangeState(payload({ stateRef: 0 }), 1),
      /prior range record none/,
    );

    clearRangeState();
    resolveRangeState(payload({ state: { a: 1 } }), 0);
    assert.throws(
      () => resolveRangeState(payload({ stateRef: 2 }), 1),
      /invalid streaming state reference/,
    );
    assert.throws(
      () => resolveRangeState(payload({ stateRef: 0, state: { a: 1 } }), 1),
      /must not include complete state/,
    );
    assert.throws(
      () =>
        resolveRangeState(
          payload({
            stateRef: 0,
            stateDelta: [] as unknown as Record<string, unknown>,
          }),
          1,
        ),
      /state delta must be an object/,
    );

    clearRangeState();
    resolveRangeState(payload({ state: { a: 1 } }), 0);
    resolveRangeState(payload({ stateRef: 0 }), 1);
    assert.throws(
      () => resolveRangeState(payload({ stateRef: 0 }), 2),
      /does not match prior range record 1/,
    );
  });

  test('releases the reference base on cancellation or reset', () => {
    resolveRangeState(payload({ state: { retained: true } }), 0);
    clearRangeState();

    assert.throws(
      () => resolveRangeState(payload({ stateRef: 0 }), 1),
      /prior range record none/,
    );
  });
});

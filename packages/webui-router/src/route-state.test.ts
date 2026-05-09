// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { strict as assert } from 'node:assert';
import { describe, test } from 'node:test';

import './browser-shim.js';
import {
  outOfChainStateKeys,
  stateForRouteEntry,
} from './route-state.js';
import type { RouteChainEntry, RouteStates } from './cache.js';

const chain: RouteChainEntry[] = [
  { component: 'app-shell', path: '/', params: {} },
  { component: 'mail-thread-page', path: 'email/:threadId', params: { threadId: '42' } },
];

describe('route-scoped state helpers', () => {
  test('resolves preferred index-aligned states', () => {
    const states: RouteStates = [
      { selectedFolder: 'inbox' },
      { threadId: '42', subject: 'Hello' },
    ];

    assert.deepEqual(stateForRouteEntry(states, chain, 0), { selectedFolder: 'inbox' });
    assert.deepEqual(stateForRouteEntry(states, chain, 1), { threadId: '42', subject: 'Hello' });
  });

  test('resolves object states by scoped chain key before component key', () => {
    const states: RouteStates = {
      '1:mail-thread-page': { threadId: '42' },
      'mail-thread-page': { threadId: 'wrong' },
    };

    assert.deepEqual(stateForRouteEntry(states, chain, 1), { threadId: '42' });
  });

  test('resolves object states by component and route path keys', () => {
    assert.deepEqual(
      stateForRouteEntry({ 'mail-thread-page': { threadId: '42' } }, chain, 1),
      { threadId: '42' },
    );
    assert.deepEqual(
      stateForRouteEntry({ 'email/:threadId': { threadId: '99' } }, chain, 1),
      { threadId: '99' },
    );
  });

  test('object form returns null/undefined to preserve current state', () => {
    // null and undefined must round-trip — components rely on this to skip setState
    assert.equal(stateForRouteEntry({ 'app-shell': null }, chain, 0), null);
    assert.equal(stateForRouteEntry({ 'app-shell': undefined }, chain, 0), undefined);
    assert.equal(stateForRouteEntry({ '0': null }, chain, 0), null);
    assert.equal(stateForRouteEntry({}, chain, 0), undefined);
  });

  test('reports non-null state entries outside the active chain', () => {
    assert.deepEqual(outOfChainStateKeys([{ ok: true }, null, { leak: true }], chain), ['2']);
    assert.deepEqual(
      outOfChainStateKeys({ 'app-shell': {}, settings: { theme: 'dark' }, unused: null }, chain),
      ['settings'],
    );
  });
});

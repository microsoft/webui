// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import './browser-shim.js';

import assert from 'node:assert/strict';
import { describe, test } from 'node:test';
import type { RouteChainEntry } from './cache.js';
import { findChangeLevel, sameRouteDeclaration } from './chain.js';

function routeEntry(
  path: string,
  params: Record<string, string> = {},
): RouteChainEntry {
  return {
    component: 'shared-page',
    path,
    params,
  };
}

describe('route chain identity', () => {
  test('declared path changes the chain level', () => {
    assert.equal(
      findChangeLevel([routeEntry('projects')], [routeEntry('')]),
      0,
    );
  });

  test('parameter changes preserve declaration identity but change the chain instance', () => {
    const oldEntry = routeEntry('items/:id', { id: '1' });
    const newEntry = routeEntry('items/:id', { id: '2' });

    assert.equal(sameRouteDeclaration(oldEntry, newEntry), true);
    assert.equal(findChangeLevel([oldEntry], [newEntry]), 0);
  });

  test('shared components on different paths are distinct declarations', () => {
    assert.equal(
      sameRouteDeclaration(routeEntry('projects'), routeEntry('')),
      false,
    );
  });
});

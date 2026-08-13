// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import './browser-shim.js';

import { strict as assert } from 'node:assert';
import { describe, test } from 'node:test';

import {
  matchBoundaryPath,
  splitBoundaryPath,
  type BoundaryPathMatch,
} from './route-boundary.js';

function match(
  routePath: string,
  requestPath: string,
  exact = true,
  base = 0,
): BoundaryPathMatch | null {
  const result = { consumed: 0, specificity: 0 };
  return matchBoundaryPath(
    routePath,
    exact,
    splitBoundaryPath(requestPath),
    base,
    result,
  )
    ? result
    : null;
}

describe('route boundary path matching', () => {
  test('matches literal and parameter segments', () => {
    assert.deepEqual(match('/contacts/:id', '/contacts/42'), {
      consumed: 2,
      specificity: 1,
    });
  });

  test('matches optional and splat segments', () => {
    assert.deepEqual(match('/profile/:id?', '/profile'), {
      consumed: 1,
      specificity: 1,
    });
    assert.deepEqual(match('/files/*rest', '/files/a/b'), {
      consumed: 3,
      specificity: 1,
    });
  });

  test('resolves relative paths from the consumed parent base', () => {
    assert.deepEqual(match('settings/:tab', '/users/42/settings/security', true, 2), {
      consumed: 4,
      specificity: 3,
    });
    assert.deepEqual(match('/admin', '/admin', true, 2), {
      consumed: 1,
      specificity: 1,
    });
  });

  test('honors exact matching', () => {
    assert.equal(match('/contacts', '/contacts/42'), null);
    assert.deepEqual(match('/contacts', '/contacts/42', false), {
      consumed: 1,
      specificity: 1,
    });
  });

  test('rejects unsafe or malformed parameter segments', () => {
    assert.equal(match('/files/:name', '/files/..'), null);
    assert.equal(match('/files/:name', '/files/%00'), null);
    assert.equal(match('/files/:name', '/files/%ZZ'), null);
  });

  test('strips query parameters before matching', () => {
    assert.deepEqual(match('/search', '/search?q=router'), {
      consumed: 1,
      specificity: 1,
    });
  });
});

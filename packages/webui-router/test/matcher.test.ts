// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { strict as assert } from 'node:assert';
import { describe, test } from 'node:test';

import {
  buildRouteBase,
  isRelativePath,
  matchPath,
  resolveRoutePath,
  specificity,
} from '../src/matcher.js';

describe('matcher helpers', () => {
  test('matches inline param segments', () => {
    const match = matchPath('/docs/topic-:topicId', '/docs/topic-intro', true);
    assert.ok(match);
    assert.deepEqual(match.params, { topicId: 'intro' });
    assert.equal(match.consumed, 2);
  });

  test('supports optional inline params', () => {
    const match = matchPath('/docs/topic-:topicId?', '/docs/topic-', true);
    assert.ok(match);
    assert.deepEqual(match.params, {});
    assert.equal(match.consumed, 2);
  });

  test('resolves relative paths against nested route bases', () => {
    assert.equal(
      resolveRoutePath('./topic-:topicId', '/docs/rust'),
      '/docs/rust/topic-:topicId',
    );
  });

  test('builds route bases from consumed segments', () => {
    assert.equal(
      buildRouteBase('/docs/rust/topic-intro', 2),
      '/docs/rust',
    );
  });

  test('detects relative paths', () => {
    assert.equal(isRelativePath('./topic-:topicId'), true);
    assert.equal(isRelativePath('/topic-:topicId'), false);
  });

  test('prefers fully literal paths over inline parameter segments', () => {
    assert.ok(specificity('/docs/topic-new') > specificity('/docs/topic-:topicId'));
    assert.ok(specificity('/docs/topic-:topicId') > specificity('/docs/:topicId'));
  });
});

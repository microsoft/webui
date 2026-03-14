import { strict as assert } from 'node:assert';
import { describe, test } from 'node:test';

import {
  buildNavigationTarget,
  prependBasePath,
  stripBaseFromPathname,
} from '../src/navigation-path.js';

describe('navigation-path helpers', () => {
  test('preserves the query string in request paths', () => {
    const target = buildNavigationTarget(
      new URL('https://example.test/store/search?q=shirt&sort=price-desc'),
      '/store',
    );

    assert.deepEqual(target, {
      pathname: '/search',
      requestPath: '/search?q=shirt&sort=price-desc',
    });
  });

  test('normalizes the base path root to slash', () => {
    assert.equal(stripBaseFromPathname('/store', '/store'), '/');
  });

  test('prepends the base path without dropping queries', () => {
    assert.equal(
      prependBasePath('/search?q=shirt&sort=price-desc', '/store'),
      '/store/search?q=shirt&sort=price-desc',
    );
  });
});

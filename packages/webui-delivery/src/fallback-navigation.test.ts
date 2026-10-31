// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

// Shim must be imported before any router code — sets up browser globals.
import './browser-shim.js';

import { strict as assert } from 'node:assert';
import { describe, test } from 'node:test';
import {
  canInterceptNavigations,
  resolveLinkNavigation,
  type ClickIntent,
  type LinkIntent,
} from './fallback-navigation.js';

const CURRENT = 'webui://app/contacts';

function click(overrides: Partial<ClickIntent> = {}): ClickIntent {
  return { defaultPrevented: false, button: 0, modified: false, ...overrides };
}

function link(href: string, overrides: Partial<LinkIntent> = {}): LinkIntent {
  return {
    href,
    resolved: new URL(href, CURRENT).href,
    download: false,
    target: '',
    rel: null,
    ...overrides,
  };
}

describe('resolveLinkNavigation', () => {
  test('claims a same-origin link on a custom scheme', () => {
    const url = resolveLinkNavigation(click(), link('/favorites'), CURRENT, []);
    assert.equal(url?.href, 'webui://app/favorites');
  });

  test('claims a link that only changes the query string', () => {
    const url = resolveLinkNavigation(click(), link('/contacts?sort=name'), CURRENT, []);
    assert.equal(url?.search, '?sort=name');
  });

  test('declines a modified click so the host can open a new tab', () => {
    assert.equal(
      resolveLinkNavigation(click({ modified: true }), link('/favorites'), CURRENT, []),
      null,
    );
  });

  test('declines a non-primary button', () => {
    assert.equal(
      resolveLinkNavigation(click({ button: 1 }), link('/favorites'), CURRENT, []),
      null,
    );
  });

  test('declines an already-handled click', () => {
    assert.equal(
      resolveLinkNavigation(click({ defaultPrevented: true }), link('/favorites'), CURRENT, []),
      null,
    );
  });

  test('declines a download link', () => {
    assert.equal(
      resolveLinkNavigation(click(), link('/report.csv', { download: true }), CURRENT, []),
      null,
    );
  });

  test('declines a link targeting another browsing context', () => {
    assert.equal(
      resolveLinkNavigation(click(), link('/favorites', { target: '_blank' }), CURRENT, []),
      null,
    );
  });

  test('claims a link that explicitly targets itself', () => {
    const url = resolveLinkNavigation(
      click(),
      link('/favorites', { target: '_self' }),
      CURRENT,
      [],
    );
    assert.equal(url?.pathname, '/favorites');
  });

  test('declines a rel=external link', () => {
    assert.equal(
      resolveLinkNavigation(
        click(),
        link('/favorites', { rel: 'noopener external' }),
        CURRENT,
        [],
      ),
      null,
    );
  });

  test('declines a cross-origin link', () => {
    assert.equal(
      resolveLinkNavigation(click(), link('https://example.com/docs'), CURRENT, []),
      null,
    );
  });

  test('declines a different custom-scheme authority', () => {
    assert.equal(
      resolveLinkNavigation(
        click(),
        {
          ...link('/dashboard'),
          resolved: 'webui://other/dashboard',
        },
        CURRENT,
        [],
      ),
      null,
    );
  });

  test('declines a missing href', () => {
    assert.equal(
      resolveLinkNavigation(click(), { ...link('/favorites'), href: null }, CURRENT, []),
      null,
    );
  });

  test('declines a pure fragment link so the browser can scroll', () => {
    assert.equal(resolveLinkNavigation(click(), link('#section'), CURRENT, []), null);
  });

  test('declines a fragment on the current path and query', () => {
    assert.equal(
      resolveLinkNavigation(click(), link('/contacts#section'), CURRENT, []),
      null,
    );
  });

  test('claims a fragment link that also changes the path', () => {
    const url = resolveLinkNavigation(click(), link('/favorites#section'), CURRENT, []);
    assert.equal(url?.pathname, '/favorites');
    assert.equal(url?.hash, '#section');
  });

  test('declines an excluded path', () => {
    assert.equal(
      resolveLinkNavigation(click(), link('/api/export'), CURRENT, ['/api']),
      null,
    );
  });

  test('claims a path that only shares a prefix with an excluded path', () => {
    const url = resolveLinkNavigation(click(), link('/apidocs'), CURRENT, ['/api/']);
    assert.equal(url?.pathname, '/apidocs');
  });

  test('works on an http origin so browsers without the Navigation API are covered', () => {
    const current = 'http://localhost:3000/contacts';
    const url = resolveLinkNavigation(
      click(),
      { ...link('/favorites'), resolved: 'http://localhost:3000/favorites' },
      current,
      [],
    );
    assert.equal(url?.href, 'http://localhost:3000/favorites');
  });
});

describe('canInterceptNavigations', () => {
  const originalProtocol = location.protocol;
  const originalNavigation = (globalThis as { navigation?: unknown }).navigation;

  function withEnvironment(protocol: string, navigation: unknown, run: () => void): void {
    Object.defineProperty(location, 'protocol', { value: protocol, configurable: true });
    (globalThis as { navigation?: unknown }).navigation = navigation;
    try {
      run();
    } finally {
      Object.defineProperty(location, 'protocol', {
        value: originalProtocol,
        configurable: true,
      });
      (globalThis as { navigation?: unknown }).navigation = originalNavigation;
    }
  }

  test('uses the Navigation API on https', () => {
    withEnvironment('https:', originalNavigation, () => {
      assert.equal(canInterceptNavigations(), true);
    });
  });

  test('uses the Navigation API on http for local development', () => {
    withEnvironment('http:', originalNavigation, () => {
      assert.equal(canInterceptNavigations(), true);
    });
  });

  test('falls back on a custom desktop scheme even though the API exists', () => {
    withEnvironment('webui:', originalNavigation, () => {
      assert.equal(canInterceptNavigations(), false);
    });
  });

  test('falls back on file:// documents', () => {
    withEnvironment('file:', originalNavigation, () => {
      assert.equal(canInterceptNavigations(), false);
    });
  });

  test('falls back when the browser has no Navigation API', () => {
    withEnvironment('https:', undefined, () => {
      assert.equal(canInterceptNavigations(), false);
    });
  });
});

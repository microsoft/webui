// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import './browser-shim.js';

import assert from 'node:assert/strict';
import { describe, test } from 'node:test';
import type { RouteChainEntry } from './cache.js';
import {
  findChangeLevel,
  findOrCreateRouteElement,
  sameRouteDeclaration,
} from './chain.js';

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

function routeElement(
  attributes: Record<string, string>,
): HTMLElement {
  return {
    tagName: 'WEBUI-ROUTE',
    style: {},
    getAttribute(name: string) {
      return Object.hasOwn(attributes, name) ? attributes[name] : null;
    },
    hasAttribute(name: string) {
      return Object.hasOwn(attributes, name);
    },
    setAttribute(name: string, value: string) {
      attributes[name] = value;
    },
  } as unknown as HTMLElement;
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

  test('missing path is not treated as the empty-path declaration', () => {
    const body = document.body as unknown as {
      children: HTMLElement[];
      appendChild(el: HTMLElement): void;
    };
    const originalChildren = body.children;
    const originalAppendChild = body.appendChild;
    const originalCreateElement = document.createElement;
    const unpathed = routeElement({ component: 'shared-page' });
    const created = routeElement({});

    body.children = [unpathed];
    body.appendChild = () => {};
    document.createElement = () => created;

    try {
      const result = findOrCreateRouteElement(null, routeEntry(''));

      assert.notEqual(result, unpathed);
      assert.equal(result, created);
      assert.equal(result.hasAttribute('path'), true);
      assert.equal(result.getAttribute('path'), '');
    } finally {
      body.children = originalChildren;
      body.appendChild = originalAppendChild;
      document.createElement = originalCreateElement;
    }
  });
});

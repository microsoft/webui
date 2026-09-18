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

function outletRoot(values: Array<string | { localName: string }>, routes: HTMLElement[] = []) {
  const inserted: { node: Node; before: Node }[] = [];
  const root = Object.assign(document.createElement('section'), {
    querySelectorAll: () => routes,
    querySelector: () => null,
    appendChild: () => { throw new Error('Route escaped the SSR outlet'); },
    insertBefore(node: Node, before: Node) {
      inserted.push({ node, before });
      return node;
    },
  });
  interface Marker {
    nodeValue: string | null;
    nodeType: number;
    localName?: string;
    parentNode: HTMLElement;
    nextSibling: Marker | null;
  }
  const nodes: Marker[] = values.map(value => ({
    ...(typeof value === 'string' ? { nodeValue: value, nodeType: 8 } : { ...value, nodeValue: null, nodeType: 1 }),
    parentNode: root, nextSibling: null,
  }));
  for (let i = 0; i < nodes.length; i++) nodes[i].nextSibling = nodes[i + 1] ?? null;
  const walker: { currentNode: object; nextNode(): Marker | null } = {
    currentNode: root,
    nextNode() {
      const next = nodes[nodes.findIndex(node => node === this.currentNode) + 1] ?? null;
      if (next) this.currentNode = next;
      return next;
    },
  };
  const parent = { ...routeEntry('/'), el: root, compEl: root };
  return { root, nodes, walker, parent, inserted };
}

describe('route chain identity', () => {
  test('ordinary digit-prefixed comments are not raw range openers', t => {
    for (const label of ['w1-note', 'w12x', 'w0 ', 'w2.3', 'w']) {
      const fixture = outletRoot([label, 'wo', '/wo']);
      const method = t.mock.method(document, 'createTreeWalker', () => fixture.walker);
      findOrCreateRouteElement(fixture.parent, routeEntry('added'));
      assert.equal(fixture.inserted[0].before, fixture.nodes[2]);
      method.mock.restore();
    }
  });

  test('new routes stay in empty and populated SSR ranges, outside nested ranges', t => {
    for (const routes of [[], [routeElement({ component: 'other-page', path: 'other' })]]) {
      const fixture = outletRoot(['wo', 'wo', '/wo', '/wo', 'wo', '/wo'], routes);
      const method = t.mock.method(document, 'createTreeWalker', () => fixture.walker);
      const result = findOrCreateRouteElement(fixture.parent, routeEntry('added'));
      assert.equal(fixture.inserted.length, 1);
      assert.equal(fixture.inserted[0].node, result);
      assert.equal(fixture.inserted[0].before, fixture.nodes[3]);
      method.mock.restore();
    }
  });

  test('an existing route does not scan SSR outlet comments', t => {
    const existing = routeElement({ component: 'shared-page', path: 'same' });
    const fixture = outletRoot([], [existing]);
    t.mock.method(document, 'createTreeWalker', () => {
      throw new Error('Existing routes must not scan comments');
    });
    assert.equal(findOrCreateRouteElement(fixture.parent, routeEntry('same')), existing);
  });

  test('raw HTML cannot supply the routing outlet markers', t => {
    const fixture = outletRoot(['w12', 'w12', 'wo', '/wo', '/w12', 'wo', '/wo', '/w12', 'wo', '/wo']);
    t.mock.method(document, 'createTreeWalker', () => fixture.walker);
    findOrCreateRouteElement(fixture.parent, routeEntry('added'));
    assert.equal(fixture.inserted[0].before, fixture.nodes[9]);
  });

  test('a recreated client outlet takes precedence over a later SSR outlet', t => {
    const fixture = outletRoot([{ localName: 'outlet' }, 'wo', '/wo']);
    t.mock.method(document, 'createTreeWalker', () => fixture.walker);
    findOrCreateRouteElement(fixture.parent, routeEntry('added'));
    assert.equal(fixture.inserted[0].before, fixture.nodes[1]);
  });

  test('unpaired SSR outlets fail before inserting a route', t => {
    for (const values of [['wo'], ['/wo'], ['wo', 'wo', '/wo'], ['w0', 'wo', '/wo']]) {
      const fixture = outletRoot(values);
      const method = t.mock.method(document, 'createTreeWalker', () => fixture.walker);
      assert.throws(
        () => findOrCreateRouteElement(fixture.parent, routeEntry('added')),
        /Unpaired SSR range markers.*Rebuild/,
      );
      assert.equal(fixture.inserted.length, 0);
      method.mock.restore();
    }
  });

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

// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { strict as assert } from 'node:assert';
import { test } from 'node:test';
import {
  getTemplate,
  registerTemplateData,
  templateHasFragments,
  templateNeedsRanges,
  type TemplateMeta,
} from './template.js';

// Node runs each test file in its own process, so this owns the first
// fragment-bearing registration without depending on other registry tests.
test('allocates the fragment cache only when a late template needs it', () => {
  const previousWindow = Object.getOwnPropertyDescriptor(globalThis, 'window');
  const PreviousWeakMap = globalThis.WeakMap;
  let allocations = 0;

  try {
    Object.defineProperty(globalThis, 'window', {
      value: { __webui: {} },
      configurable: true,
      writable: true,
    });
    globalThis.WeakMap = new Proxy(PreviousWeakMap, {
      construct(target, argumentsList, newTarget) {
        allocations++;
        return Reflect.construct(target, argumentsList, newTarget);
      },
    });

    const ordinary: TemplateMeta = {
      h: '<p>ordinary</p>',
      u: [],
      b: [{ h: '<span>ordinary block</span>', u: [] }],
    };
    assert.equal(templateHasFragments(ordinary), false);
    assert.equal(templateNeedsRanges(ordinary), false);
    registerTemplateData({ ordinary });
    assert.equal(getTemplate('ordinary'), ordinary);
    assert.equal(templateHasFragments(ordinary), false);
    assert.equal(allocations, 0);

    const nested: TemplateMeta = {
      h: '',
      r: [['items', 'item', 0, [0, 0]]],
      b: [
        { h: '', u: [[1, [0, 0], 'item', 'entry']] },
        { h: '<span>fragment body</span>' },
      ],
    };
    registerTemplateData({ nested });
    assert.equal(getTemplate('nested'), nested);
    assert.equal(templateHasFragments(nested), true);
    assert.equal(templateNeedsRanges(nested), true);
    assert.equal(allocations, 1, 'the first fragment template creates the cache');

    const root: TemplateMeta = {
      h: '',
      u: [[0, [0, 0]]],
      b: [{ h: '<b>root fragment</b>' }],
    };
    registerTemplateData({ root });
    registerTemplateData({ nested });
    assert.equal(templateHasFragments(root), true);
    assert.equal(templateHasFragments(nested), true);
    assert.equal(templateHasFragments(ordinary), false);
    assert.equal(allocations, 1, 'later registrations reuse the same weak cache');
  } finally {
    globalThis.WeakMap = PreviousWeakMap;
    if (previousWindow) {
      Object.defineProperty(globalThis, 'window', previousWindow);
    } else {
      Reflect.deleteProperty(globalThis, 'window');
    }
  }
});

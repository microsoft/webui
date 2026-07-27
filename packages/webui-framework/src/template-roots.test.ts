// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { strict as assert } from 'node:assert';
import { describe, test } from 'node:test';

import type { TemplateMeta } from './template.js';
import {
  templateHasRoot,
  templateNeedsStaticHost,
  templateRootForAttribute,
} from './template-roots.js';

describe('template root metadata helpers', () => {
  test('use compiler-emitted roots and attributes without scanning bindings', () => {
    const meta: TemplateMeta = {
      h: '<p></p>',
      tr: ['displayValue', 'readOnly'],
      ta: ['display-value', 'readonly'],
      tx: [[
        [[], 0],
        [['ignoredByHelpers']],
      ]      ],
    };

    assert.equal(templateHasRoot(meta, 'displayValue'), true);
    assert.equal(templateRootForAttribute(meta, 'readonly'), 'readOnly');
  });

  test('uses the compiler-owned dormant host flag directly', () => {
    assert.equal(templateNeedsStaticHost({ h: '<p></p>', th: 1 }), true);
    assert.equal(templateNeedsStaticHost({ h: '<p></p>' }), false);
  });
});

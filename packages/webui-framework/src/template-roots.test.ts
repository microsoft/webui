// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { strict as assert } from 'node:assert';
import { describe, test } from 'node:test';

import type { TemplateMeta } from './template.js';
import {
  collectTemplateRoots,
  templateAttributeForRoot,
  templateHasRoot,
  templateNeedsStaticHost,
  templateRootForAttribute,
} from './template-roots.js';

describe('template root metadata helpers', () => {
  test('use compiler-emitted roots and attributes without scanning bindings', () => {
    const meta: TemplateMeta = {
      h: '<p></p>',
      th: 1,
      tr: ['displayValue', 'readOnly'],
      ta: ['display-value', 'readonly'],
      tx: [[
        [[], 0],
        [['ignoredByHelpers']],
      ]],
    };

    assert.deepEqual(collectTemplateRoots(meta), ['displayValue', 'readOnly']);
    assert.equal(templateHasRoot(meta, 'displayValue'), true);
    assert.equal(templateRootForAttribute(meta, 'readonly'), 'readOnly');
    assert.equal(templateAttributeForRoot(meta, 'displayValue'), 'display-value');
    assert.equal(templateNeedsStaticHost(meta), true);
  });

  test('missing static host flag skips template host ownership', () => {
    const meta: TemplateMeta = {
      h: '<button></button>',
      tr: ['title'],
      ta: ['title'],
    };

    assert.equal(templateNeedsStaticHost(meta), false);
  });
});

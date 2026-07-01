// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { strict as assert } from 'node:assert';
import { describe, test } from 'node:test';

import type { TemplateMeta } from './template.js';
import {
  collectTemplateRoots,
  templateAttributeForRoot,
  templateHasEventHandlers,
  templateHasRoot,
  templateNeedsAutoElement,
  templateRootForAttribute,
} from './template-roots.js';

describe('template root metadata helpers', () => {
  test('use compiler-emitted roots and attributes without scanning bindings', () => {
    const meta: TemplateMeta = {
      h: '<p></p>',
      ae: 1,
      tr: ['displayValue', 'readOnly'],
      ta: ['display-value', 'displayValue', 'readonly', 'readOnly'],
      tx: [[
        [[], 0],
        [['ignoredByHelpers']],
      ]],
    };

    assert.deepEqual(collectTemplateRoots(meta), ['displayValue', 'readOnly']);
    assert.equal(templateHasRoot(meta, 'displayValue'), true);
    assert.equal(templateRootForAttribute(meta, 'readonly'), 'readOnly');
    assert.equal(templateAttributeForRoot(meta, 'displayValue'), 'display-value');
    assert.equal(templateNeedsAutoElement(meta), true);
  });

  test('feature flags mark interactive templates without walking nested blocks', () => {
    const meta: TemplateMeta = {
      h: '<button></button>',
      ae: 1,
      tr: ['title'],
      ta: ['title', 'title'],
      tf: 1,
    };

    assert.equal(templateHasEventHandlers(meta), true);
    assert.equal(templateNeedsAutoElement(meta), false);
  });
});

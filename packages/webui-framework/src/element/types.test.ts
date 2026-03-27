// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { strict as assert } from 'node:assert';
import { describe, test } from 'node:test';

import { toCamelCase } from './types.js';

describe('type helpers', () => {
  test('toCamelCase normalizes dash-separated names', () => {
    assert.equal(toCamelCase('data-active-item'), 'dataActiveItem');
    assert.equal(toCamelCase('aria-label'), 'ariaLabel');
    assert.equal(toCamelCase('alreadyCamel'), 'alreadyCamel');
  });
});

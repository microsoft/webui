// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { strict as assert } from 'node:assert';
import { describe, test } from 'node:test';

import { toKebabCase } from './decorators.js';

describe('toKebabCase', () => {
  test('converts multi-word ARIA properties to correct attribute names', () => {
    assert.equal(toKebabCase('ariaDescribedBy'), 'aria-describedby');
    assert.equal(toKebabCase('ariaLabelledBy'), 'aria-labelledby');
    assert.equal(toKebabCase('ariaActiveDescendant'), 'aria-activedescendant');
    assert.equal(toKebabCase('ariaAutoComplete'), 'aria-autocomplete');
    assert.equal(toKebabCase('ariaColCount'), 'aria-colcount');
    assert.equal(toKebabCase('ariaColIndex'), 'aria-colindex');
    assert.equal(toKebabCase('ariaColIndexText'), 'aria-colindextext');
    assert.equal(toKebabCase('ariaColSpan'), 'aria-colspan');
    assert.equal(toKebabCase('ariaDropEffect'), 'aria-dropeffect');
    assert.equal(toKebabCase('ariaErrorMessage'), 'aria-errormessage');
    assert.equal(toKebabCase('ariaFlowTo'), 'aria-flowto');
    assert.equal(toKebabCase('ariaHasPopup'), 'aria-haspopup');
    assert.equal(toKebabCase('ariaKeyShortcuts'), 'aria-keyshortcuts');
    assert.equal(toKebabCase('ariaMultiLine'), 'aria-multiline');
    assert.equal(toKebabCase('ariaMultiSelectable'), 'aria-multiselectable');
    assert.equal(toKebabCase('ariaPosInSet'), 'aria-posinset');
    assert.equal(toKebabCase('ariaReadOnly'), 'aria-readonly');
    assert.equal(toKebabCase('ariaRoleDescription'), 'aria-roledescription');
    assert.equal(toKebabCase('ariaRowCount'), 'aria-rowcount');
    assert.equal(toKebabCase('ariaRowIndex'), 'aria-rowindex');
    assert.equal(toKebabCase('ariaRowIndexText'), 'aria-rowindextext');
    assert.equal(toKebabCase('ariaRowSpan'), 'aria-rowspan');
    assert.equal(toKebabCase('ariaSetSize'), 'aria-setsize');
    assert.equal(toKebabCase('ariaValueMax'), 'aria-valuemax');
    assert.equal(toKebabCase('ariaValueMin'), 'aria-valuemin');
    assert.equal(toKebabCase('ariaValueNow'), 'aria-valuenow');
    assert.equal(toKebabCase('ariaValueText'), 'aria-valuetext');
    assert.equal(toKebabCase('ariaBrailleLabel'), 'aria-braillelabel');
    assert.equal(
      toKebabCase('ariaBrailleRoleDescription'),
      'aria-brailleroledescription',
    );
  });

  test('single-word ARIA properties use fallback conversion', () => {
    assert.equal(toKebabCase('ariaLabel'), 'aria-label');
    assert.equal(toKebabCase('ariaHidden'), 'aria-hidden');
    assert.equal(toKebabCase('ariaDisabled'), 'aria-disabled');
    assert.equal(toKebabCase('ariaChecked'), 'aria-checked');
  });

  test('non-ARIA properties use standard camelCase-to-kebab', () => {
    assert.equal(toKebabCase('myProp'), 'my-prop');
    assert.equal(toKebabCase('totalContacts'), 'total-contacts');
    assert.equal(toKebabCase('dataTitle'), 'data-title');
  });
});

// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { strict as assert } from 'node:assert';
import { describe, test } from 'node:test';

import {
  attributeNameForProperty,
  attr,
  getObservableNames,
  isAttributeProperty,
  observable,
  toKebabCase,
} from './decorators.js';

class FakeElement {
  $ready = true;
  isConnected = false;
  private readonly attrs = new Map<string, string>();

  getAttribute(name: string): string | null {
    return this.attrs.get(name) ?? null;
  }

  hasAttribute(name: string): boolean {
    return this.attrs.has(name);
  }

  setAttribute(name: string, value: string): void {
    const oldValue = this.getAttribute(name);
    const newValue = String(value);
    this.attrs.set(name, newValue);
    this.attributeChangedCallback?.(name, oldValue, newValue);
  }

  removeAttribute(name: string): void {
    const oldValue = this.getAttribute(name);
    this.attrs.delete(name);
    this.attributeChangedCallback?.(name, oldValue, null);
  }

  attributeChangedCallback?(
    name: string,
    oldValue: string | null,
    newValue: string | null,
  ): void;
}

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

  test('converts HTML global/element properties to correct attribute names', () => {
    assert.equal(toKebabCase('readOnly'), 'readonly');
    assert.equal(toKebabCase('tabIndex'), 'tabindex');
    assert.equal(toKebabCase('accessKey'), 'accesskey');
    assert.equal(toKebabCase('contentEditable'), 'contenteditable');
    assert.equal(toKebabCase('crossOrigin'), 'crossorigin');
    assert.equal(toKebabCase('inputMode'), 'inputmode');
    assert.equal(toKebabCase('maxLength'), 'maxlength');
    assert.equal(toKebabCase('minLength'), 'minlength');
    assert.equal(toKebabCase('noValidate'), 'novalidate');
    assert.equal(toKebabCase('formAction'), 'formaction');
    assert.equal(toKebabCase('formEnctype'), 'formenctype');
    assert.equal(toKebabCase('formMethod'), 'formmethod');
    assert.equal(toKebabCase('formNoValidate'), 'formnovalidate');
    assert.equal(toKebabCase('formTarget'), 'formtarget');
    assert.equal(toKebabCase('isMap'), 'ismap');
    assert.equal(toKebabCase('useMap'), 'usemap');
    assert.equal(toKebabCase('noModule'), 'nomodule');
    assert.equal(toKebabCase('autoCapitalize'), 'autocapitalize');
    assert.equal(toKebabCase('dirName'), 'dirname');
    assert.equal(toKebabCase('fetchPriority'), 'fetchpriority');
    assert.equal(toKebabCase('referrerPolicy'), 'referrerpolicy');
  });

  test('non-ARIA properties use standard camelCase-to-kebab', () => {
    assert.equal(toKebabCase('myProp'), 'my-prop');
    assert.equal(toKebabCase('totalContacts'), 'total-contacts');
    assert.equal(toKebabCase('dataTitle'), 'data-title');
  });
});

describe('observable decorators', () => {
  test('@observable registers reactive property names', () => {
    class TestElement {}
    observable(TestElement.prototype, 'count');

    assert.equal(getObservableNames(TestElement).has('count'), true);
  });

  test('@observable names inherit through subclasses', () => {
    class BaseElement {}
    observable(BaseElement.prototype, 'baseCount');
    class TestElement extends BaseElement {}
    observable(TestElement.prototype, 'count');

    const names = getObservableNames(TestElement);
    assert.equal(names.has('baseCount'), true);
    assert.equal(names.has('count'), true);
  });

  test('@attr registers reactive property names', () => {
    class TestElement {}
    attr(TestElement.prototype, 'label');

    assert.equal(getObservableNames(TestElement).has('label'), true);
    assert.equal(attributeNameForProperty(TestElement, 'label'), 'label');
  });

  test('@attr exposes custom reflected attribute names', () => {
    class TestElement {}
    attr({ attribute: 'display-value' })(
      TestElement.prototype,
      'displayValue',
    );

    assert.equal(
      attributeNameForProperty(TestElement, 'displayValue'),
      'display-value',
    );
    assert.equal(
      attributeNameForProperty(TestElement, 'missing'),
      undefined,
    );
  });

  test('@attr reflects property values without stringifying backing state', () => {
    class TestElement extends FakeElement {}
    attr(TestElement.prototype, 'count');

    const element = new TestElement() as TestElement & { count: number };
    element.count = 5;

    assert.equal(element.getAttribute('count'), '5');
    assert.equal(element.count, 5);
  });

  test('@attr reacts to string attribute changes', () => {
    class TestElement extends FakeElement {}
    attr({ attribute: 'display-value' })(TestElement.prototype, 'displayValue');

    const element = new TestElement() as TestElement & { displayValue: string | null };
    element.setAttribute('display-value', 'Ready');

    assert.equal(element.displayValue, 'Ready');
  });

  test('@attr boolean mode reflects presence and removal', () => {
    class TestElement extends FakeElement {}
    attr({ mode: 'boolean', attribute: 'is-active' })(TestElement.prototype, 'isActive');

    const element = new TestElement() as TestElement & { isActive: boolean };
    element.isActive = true;
    assert.equal(element.hasAttribute('is-active'), true);

    element.isActive = false;
    assert.equal(element.hasAttribute('is-active'), false);

    element.setAttribute('is-active', '');
    assert.equal(element.isActive, true);

    element.removeAttribute('is-active');
    assert.equal(element.isActive, false);
  });

  test('@attr metadata inherits through subclasses', () => {
    class BaseElement extends FakeElement {}
    attr(BaseElement.prototype, 'baseLabel');
    class TestElement extends BaseElement {}
    attr(TestElement.prototype, 'label');

    const names = getObservableNames(TestElement);
    assert.equal(names.has('baseLabel'), true);
    assert.equal(names.has('label'), true);
    assert.equal(isAttributeProperty(TestElement, 'baseLabel'), true);
    assert.equal(isAttributeProperty(TestElement, 'label'), true);
    assert.equal(isAttributeProperty(TestElement, 'missing'), false);

    const element = new TestElement() as TestElement & {
      baseLabel: string;
      label: string;
    };
    element.setAttribute('base-label', 'Base');
    element.setAttribute('label', 'Child');

    assert.equal(element.baseLabel, 'Base');
    assert.equal(element.label, 'Child');

    element.baseLabel = 'Updated Base';
    element.label = 'Updated Child';
    assert.equal(element.getAttribute('base-label'), 'Updated Base');
    assert.equal(element.getAttribute('label'), 'Updated Child');
  });
});

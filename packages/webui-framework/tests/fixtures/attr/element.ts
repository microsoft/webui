// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { WebUIElement, attr, observable } from '../../../src/index.js';

export class TestAttr extends WebUIElement {
  labelRef!: HTMLSpanElement;
  internals = this.attachInternals();
  changeLog: { first: boolean; keys: string[]; previousLabel: unknown; refReady: boolean; internalsReady: boolean }[] = [];
  @attr label = 'Status';
  @attr({ attribute: 'display-value' }) displayValue = 'Ready';
  @attr({ attribute: 'cta-href' }) ctaHref = '/checkout';
  @attr({ mode: 'boolean', attribute: 'is-active' }) isActive = false;
  @observable itemId = '42';
  @observable tag = 'demo';

  protected override propertiesChanged(changes: ReadonlyMap<string, unknown>, first: boolean): void {
    this.changeLog.push({
      first,
      keys: [...changes.keys()],
      previousLabel: changes.get('label'),
      refReady: this.labelRef instanceof HTMLSpanElement,
      internalsReady: this.internals instanceof ElementInternals,
    });
    if (changes.has('label')) this.setAttribute('role', 'status');
  }

  noop(): void {}
}

TestAttr.define('test-attr');

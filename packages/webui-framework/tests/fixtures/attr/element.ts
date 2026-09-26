// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { WebUIElement, attr, observable } from '../../../src/index.js';

export class TestAttr extends WebUIElement {
  labelRef!: HTMLSpanElement;
  internals = this.attachInternals();
  changeLog: { name: string; oldValue: unknown; value: unknown; refReady: boolean; internalsReady: boolean }[] = [];
  @attr label = 'Status';
  @attr({ attribute: 'display-value' }) displayValue = 'Ready';
  @attr({ attribute: 'cta-href' }) ctaHref = '/checkout';
  @attr({ mode: 'boolean', attribute: 'is-active' }) isActive = false;
  @observable itemId = '42';
  @observable tag = 'demo';

  private recordChange(name: string, oldValue: unknown, value: unknown): void {
    this.changeLog.push({
      name,
      oldValue,
      value,
      refReady: this.labelRef instanceof HTMLSpanElement,
      internalsReady: this.internals instanceof ElementInternals,
    });
  }

  labelChanged(oldValue: unknown, value: string): void {
    this.recordChange('label', oldValue, value);
    this.setAttribute('role', 'status');
  }

  displayValueChanged(oldValue: unknown, value: string): void {
    this.recordChange('displayValue', oldValue, value);
  }

  isActiveChanged(oldValue: unknown, value: boolean): void {
    this.recordChange('isActive', oldValue, value);
    this.internals.ariaChecked = String(value);
  }

  noop(): void {}
}

TestAttr.define('test-attr');

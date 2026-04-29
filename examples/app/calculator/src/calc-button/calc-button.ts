// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { FASTElement, attr } from '@microsoft/fast-element';
import { attributeMap } from '@microsoft/fast-element/attribute-map.js';
import { declarativeTemplate } from '@microsoft/fast-element/declarative.js';
import { observerMap } from '@microsoft/fast-element/observer-map.js';

export class CalcButton extends FASTElement {
  @attr label = '';
  @attr value = '';
  @attr({ attribute: 'btn-type' }) btnType = '';
  @attr({ attribute: 'btn-span' }) btnSpan = '';

  onClick(): void {
    this.dispatchEvent(
      new CustomEvent('button-press', {
        bubbles: true,
        composed: true,
        detail: { value: this.value },
      })
    );
  }
}

void CalcButton.define({
  name: 'calc-button',
  template: declarativeTemplate(),
}, [attributeMap(), observerMap()]);

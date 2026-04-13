// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { WebUIElement, attr } from '../../../src/index.js';

export class TestRef extends WebUIElement {
  @attr value = 'hello';
  inputEl!: HTMLInputElement;

  readInput(): void {
    this.value = this.inputEl.value;
  }

  focusInput(): void {
    this.inputEl.focus();
  }
}

TestRef.define('test-ref');


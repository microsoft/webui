// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { WebUIElement, attr, observable } from '../../../src/index.js';

export class TestRef extends WebUIElement {
  @attr value = 'hello';
  @observable expanded = false;
  inputEl!: HTMLInputElement;
  conditionalEl?: HTMLInputElement;

  readInput(): void {
    this.value = this.inputEl.value;
  }

  focusInput(): void {
    this.inputEl.focus();
  }

  toggle(): void {
    this.expanded = !this.expanded;
  }
}

TestRef.define('test-ref');

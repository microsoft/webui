// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { WebUIElement, attr, observable } from '../../../src/index.js';

export class TestBasics extends WebUIElement {
  @attr greeting = 'Hello';
  @observable name = 'World';
  @observable count = 0;
  @observable doubled = 0;
  @observable lastKey = '';
  @observable inputValue = '';

  increment(): void {
    this.count += 1;
    this.doubled = this.count * 2;
  }

  decrement(): void {
    this.count -= 1;
    this.doubled = this.count * 2;
  }

  onReset(): void {
    this.count = 0;
    this.doubled = 0;
  }

  onInput(e: Event): void {
    this.inputValue = (e.target as HTMLInputElement).value;
  }

  onKeydown(e: KeyboardEvent): void {
    this.lastKey = e.key;
  }
}

TestBasics.define('test-basics');

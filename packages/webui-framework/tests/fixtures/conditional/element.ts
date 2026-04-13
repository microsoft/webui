// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { WebUIElement, observable } from '../../../src/index.js';

export class TestConditional extends WebUIElement {
  @observable open = true;
  @observable busy = false;
  @observable details = 'Details';
  @observable count = 1;

  toggleOpen(): void {
    this.open = !this.open;
  }
}

TestConditional.define('test-conditional');

export class TestConditionalClient extends WebUIElement {
  @observable open = true;
  @observable busy = false;
  @observable details = 'Details';

  toggleOpen(): void {
    this.open = !this.open;
  }
}

TestConditionalClient.define('test-conditional-client');


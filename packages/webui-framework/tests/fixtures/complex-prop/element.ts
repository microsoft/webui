// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { WebUIElement, observable } from '../../../src/index.js';

export class TestItemList extends WebUIElement {
  @observable items: Array<{ name: string }> = [];
}

export class TestItemHost extends WebUIElement {
  @observable items: Array<{ name: string }> = [
    { name: 'Alpha' },
    { name: 'Beta' },
    { name: 'Gamma' },
  ];

  replaceItems(): void {
    this.items = [{ name: 'One' }, { name: 'Two' }];
  }

  clearItems(): void {
    this.items = [];
  }
}

TestItemList.define('test-item-list');
TestItemHost.define('test-item-host');

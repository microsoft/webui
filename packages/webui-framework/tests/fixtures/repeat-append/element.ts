// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { WebUIElement, attr, observable } from '../../../src/index.js';

interface RepeatItem {
  id: string;
  title: string;
}

export class TestRepeatChild extends WebUIElement {
  @attr label = '';
}

TestRepeatChild.define('test-repeat-child');

export class TestRepeatParent extends WebUIElement {
  @observable nextId = 6;
  @observable items: RepeatItem[] = [
    { id: '1', title: 'Item 1' },
    { id: '2', title: 'Item 2' },
    { id: '3', title: 'Item 3' },
    { id: '4', title: 'Item 4' },
    { id: '5', title: 'Item 5' },
  ];

  addItem(): void {
    const id = String(this.nextId);
    this.nextId += 1;
    this.items = [...this.items, { id, title: `Item ${id}` }];
  }

  prependItem(): void {
    const id = String(this.nextId);
    this.nextId += 1;
    this.items = [{ id, title: `Item ${id}` }, ...this.items];
  }

  removeItem(): void {
    this.items = this.items.slice(0, -1);
  }
}

TestRepeatParent.define('test-repeat-parent');

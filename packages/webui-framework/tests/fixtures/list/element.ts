// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { WebUIElement, attr, observable } from '../../../src/index.js';

export class TestListItem extends WebUIElement {
  @attr({ attribute: 'item-id' }) itemId = '';
  @attr title = '';
  @attr state = 'pending';

  onToggle(): void {
    this.$emit('toggle-item', { id: this.itemId });
  }
}

TestListItem.define('test-list-item');

interface ListItem {
  id: string;
  title: string;
  state: string;
  flagged?: boolean;
}

export class TestList extends WebUIElement {
  @observable items: ListItem[] = [
    { id: '1', title: 'Alpha', state: 'pending', flagged: false },
    { id: '2', title: 'Beta', state: 'done', flagged: true },
  ];
  nextId = 3;

  addItem(): void {
    const id = String(this.nextId);
    this.nextId += 1;
    this.items = [...this.items, {
      id,
      title: `Item ${id}`,
      state: id === '3' ? 'done' : 'pending',
    }];
  }

  prependItem(): void {
    const id = String(this.nextId);
    this.nextId += 1;
    this.items = [{
      id,
      title: `Item ${id}`,
      state: 'pending',
    }, ...this.items];
  }

  toggleItem(e: CustomEvent<{ id: string }>): void {
    const item = this.items.find(i => i.id === e.detail.id);
    if (item) {
      item.state = item.state === 'done' ? 'pending' : 'done';
      this.items = [...this.items];
    }
  }

  reverseItems(): void {
    this.items = [...this.items].reverse();
  }

  clearItems(): void {
    this.items = [];
  }
}

TestList.define('test-list');


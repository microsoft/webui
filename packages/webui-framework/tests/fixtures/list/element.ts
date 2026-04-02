// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { WebUIElement, attr, observable } from '../../../src/index.js';
import {
  attrTarget,
  bindAttr,
  bindEvent,
  bindText,
  dynamic,
  eq,
  nodePath,
  registerCompiledTemplate,
  repeat,
  slot,
  stringLiteral,
  when,
} from '@microsoft/webui-test-support';

registerCompiledTemplate('test-list-item', {
  h: '<article class="item"><button class="toggle">Toggle</button><span class="title"></span></article>',
  text: [
    bindText(slot({ parent: nodePath(0, 1), before: 0 }), dynamic('title')),
  ],
  conditionals: [when(eq('state', stringLiteral('done')), { blockIndex: 0, slot: { parent: nodePath(0), before: 2 } })],
  blocks: [{
    h: '<span class="done">Done</span>',
  }],
  events: [
    bindEvent('click', 'onToggle', false, nodePath(0, 0)),
  ],
});

registerCompiledTemplate('test-list', {
  h: '<div class="controls"><button class="add">Add</button><button class="prepend">Prepend</button><button class="reverse">Reverse</button><button class="clear">Clear</button></div><div class="items"></div>',
  repeats: [repeat('items', 'item', { blockIndex: 0, slot: { parent: nodePath(1), before: 0 } })],
  blocks: [{
    h: '<test-list-item></test-list-item>',
    attrs: [
      bindAttr('item-id', 'item.id'),
      bindAttr('title', 'item.title'),
      bindAttr('state', 'item.state'),
    ],
    attrGroups: [attrTarget(nodePath(0), { startIndex: 0, bindingCount: 3 })],
  }],
  events: [
    bindEvent('click', 'addItem', false, nodePath(0, 0)),
    bindEvent('click', 'prependItem', false, nodePath(0, 1)),
    bindEvent('click', 'reverseItems', false, nodePath(0, 2)),
    bindEvent('click', 'clearItems', false, nodePath(0, 3)),
  ],
  rootEvents: [
    bindEvent('toggle-item', 'toggleItem', true),
  ],
});

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
}

export class TestList extends WebUIElement {
  @observable items: ListItem[] = [
    { id: '1', title: 'Alpha', state: 'pending' },
    { id: '2', title: 'Beta', state: 'done' },
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


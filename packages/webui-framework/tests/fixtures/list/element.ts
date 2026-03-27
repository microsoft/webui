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
  h: '<article class="item"><span class="title"></span></article>',
  text: [
    bindText(slot({ parent: nodePath(0, 0), before: 0 }), dynamic('title')),
  ],
  conditionals: [when(eq('state', stringLiteral('done')), { blockIndex: 0 })],
  conditionSlots: [
    slot({ parent: nodePath(0), before: 1 }),
  ],
  blocks: [{
    h: '<span class="done">Done</span>',
  }],
});

registerCompiledTemplate('test-list', {
  h: '<div class="controls"><button class="add">Add</button><button class="reverse">Reverse</button><button class="clear">Clear</button></div><div class="items"></div>',
  repeats: [repeat('items', 'item', { blockIndex: 0 })],
  repeatSlots: [slot({ parent: nodePath(1), before: 0 })],
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
    bindEvent('click', 'addItem'),
    bindEvent('click', 'reverseItems'),
    bindEvent('click', 'clearItems'),
  ],
  eventTargets: [nodePath(0, 0), nodePath(0, 1), nodePath(0, 2)],
});

export class TestListItem extends WebUIElement {
  @attr({ attribute: 'item-id' }) itemId = '';
  @attr title = '';
  @attr state = 'pending';
}

TestListItem.define('test-list-item');

interface ListItem {
  id: string;
  title: string;
  state: string;
}

export class TestList extends WebUIElement {
  @observable items: ListItem[] = [];
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

  reverseItems(): void {
    this.items = [...this.items].reverse();
  }

  clearItems(): void {
    this.items = [];
  }
}

TestList.define('test-list');


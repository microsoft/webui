// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

/**
 * Regression fixture: complex property bindings (:prop) must propagate
 * changes from parent to child, causing the child's <for> loop to re-render.
 *
 * Scenario: parent has @observable items = [...]. Template binds
 * :items="{{items}}" on a child. When parent.items is replaced via
 * setInitialState (simulating SPA partial), the child's <for> loop
 * must re-render with the new array.
 */

import { WebUIElement, observable } from '../../../src/index.js';
import {
  bindProp,
  bindText,
  dynamic,
  nodePath,
  registerCompiledTemplate,
  repeat,
  slot,
  attrTarget,
} from '@microsoft/webui-test-support';

// Child: renders a <for> loop over its items
registerCompiledTemplate('test-item-list', {
  h: '<ul class="list"></ul>',
  sd: true,
  repeats: [repeat('items', 'item', { blockIndex: 0, slot: { parent: nodePath(0), before: 0 } })],
  blocks: [{
    h: '<li class="item"></li>',
    text: [bindText(slot({ parent: nodePath(0), before: 0 }), dynamic('item.name'))],
  }],
});

// Parent: owns the items array, passes via complex binding
registerCompiledTemplate('test-item-host', {
  h: '<div class="controls"><button class="replace">Replace</button><button class="clear">Clear</button></div><test-item-list></test-item-list>',
  sd: true,
  attrs: [
    bindProp('items', 'items'),
  ],
  attrGroups: [attrTarget(nodePath(1), { startIndex: 0, bindingCount: 1 })],
});

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

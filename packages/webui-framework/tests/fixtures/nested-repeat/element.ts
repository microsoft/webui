// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { WebUIElement, observable } from '../../../src/index.js';
import {
  attrTarget,
  bindAttr,
  bindBoolAttr,
  bindEvent,
  bindText,
  dynamic,
  identifier,
  nodePath,
  registerCompiledTemplate,
  repeat,
  slot,
} from '@microsoft/webui-test-support';

registerCompiledTemplate('test-nested-repeat', {
  h: '<div class="controls"><button class="load">Load</button></div><div class="groups"></div>',
  repeats: [repeat('groups', 'group', { blockIndex: 0 })],
  repeatSlots: [slot({ parent: nodePath(1), before: 0 })],
  blocks: [
    {
      h: '<section class="group"><h2></h2><div class="values"></div></section>',
      text: [
        bindText(slot({ parent: nodePath(0, 0), before: 0 }), dynamic('group.name')),
      ],
      repeats: [repeat('group.values', 'item', { blockIndex: 1 })],
      repeatSlots: [
        slot({ parent: nodePath(0, 1), before: 0 }),
      ],
    },
    {
      h: '<button class="value"></button>',
      text: [
        bindText(slot({ parent: nodePath(0), before: 0 }), dynamic('item.value')),
      ],
      attrs: [
        bindAttr('data-group', 'group.name'),
        bindAttr('data-value', 'item.value'),
        bindBoolAttr('disabled', identifier('item.disabled')),
      ],
      attrGroups: [attrTarget(nodePath(0), { startIndex: 0, bindingCount: 3 })],
    },
  ],
  events: [bindEvent('click', 'loadGroups')],
  eventTargets: [nodePath(0, 0)],
});

interface NestedRepeatValue {
  value: string;
  disabled: boolean;
}

interface NestedRepeatGroup {
  name: string;
  values: NestedRepeatValue[];
}

export class TestNestedRepeat extends WebUIElement {
  @observable groups: NestedRepeatGroup[] = [];

  loadGroups(): void {
    this.groups = [
      {
        name: 'Color',
        values: [
          { value: 'Black', disabled: false },
          { value: 'Blue', disabled: true },
        ],
      },
      {
        name: 'Size',
        values: [
          { value: 'S', disabled: false },
          { value: 'M', disabled: false },
        ],
      },
    ];
  }
}

TestNestedRepeat.define('test-nested-repeat');


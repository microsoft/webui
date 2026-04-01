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

  /** Re-set groups with new objects to trigger nested reconciliation. */
  updateGroups(): void {
    this.groups = this.groups.map((group) => ({
      name: group.name,
      values: group.values.map((v) => ({
        value: v.value,
        disabled: v.disabled,
      })),
    }));
  }

  /** Add a value to the first group to test growing inner lists. */
  growFirstGroup(): void {
    if (this.groups.length === 0) return;
    const first = this.groups[0];
    this.groups = [
      {
        name: first.name,
        values: [
          ...first.values.map((v) => ({ value: v.value, disabled: v.disabled })),
          { value: 'Red', disabled: false },
        ],
      },
      ...this.groups.slice(1).map((g) => ({
        name: g.name,
        values: g.values.map((v) => ({ value: v.value, disabled: v.disabled })),
      })),
    ];
  }

  /** Remove a value from the first group to test shrinking inner lists. */
  shrinkFirstGroup(): void {
    if (this.groups.length === 0 || this.groups[0].values.length === 0) return;
    const first = this.groups[0];
    this.groups = [
      {
        name: first.name,
        values: first.values.slice(1).map((v) => ({ value: v.value, disabled: v.disabled })),
      },
      ...this.groups.slice(1).map((g) => ({
        name: g.name,
        values: g.values.map((v) => ({ value: v.value, disabled: v.disabled })),
      })),
    ];
  }
}

TestNestedRepeat.define('test-nested-repeat');


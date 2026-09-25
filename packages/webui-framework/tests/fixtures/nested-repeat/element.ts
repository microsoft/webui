// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { WebUIElement, observable } from '../../../src/index.js';

interface NestedRepeatValue {
  value: string;
  disabled: boolean;
}

interface NestedRepeatGroup {
  name: string;
  values: NestedRepeatValue[];
}

interface KeyedChainGroup {
  id: string;
  sections: Array<{
    id: string;
    visible: boolean;
    items: Array<{ id: string; label: string }>;
  }>;
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

export class TestNestedRepeatKeyedChain extends WebUIElement {
  @observable keyedGroups: KeyedChainGroup[] = [];

  reverseItems(): void {
    this.keyedGroups = this.keyedGroups.map((group) => ({
      id: group.id,
      sections: group.sections.map((section) => ({
        id: section.id,
        visible: section.visible,
        items: section.items
          .map((item) => ({
            id: item.id,
            label: `${item.label} updated`,
          }))
          .reverse(),
      })),
    }));
  }
}

export class TestRepeatSiblings extends WebUIElement {
  @observable groups: NestedRepeatGroup[] = [];
  @observable others: string[] = [];
  @observable selected = 'none';

  select(value: string): void {
    this.selected = value;
  }

  replaceOthers(): void {
    this.others = ['Three', 'Four'];
  }
}

export class TestRepeatInterleaved extends WebUIElement {
  @observable headItems: string[] = [];
  @observable innerItems: string[] = [];
  @observable tailItems: string[] = [];

  replaceTail(): void {
    this.tailItems = ['T3', 'T4', 'T5'];
  }
}

export class TestRepeatAfterConditional extends WebUIElement {
  @observable showInner = true;
  @observable innerRows: string[] = [];
  @observable tailRows: string[] = [];

  replaceTailRows(): void {
    this.tailRows = ['Y3', 'Y4', 'Y5'];
  }
}

TestNestedRepeat.define('test-nested-repeat');
TestRepeatSiblings.define('test-repeat-siblings');
TestNestedRepeatKeyedChain.define('test-nested-repeat-keyed-chain');
TestRepeatInterleaved.define('test-repeat-interleaved');
TestRepeatAfterConditional.define('test-repeat-after-conditional');

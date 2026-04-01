// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { WebUIElement, observable } from '../../../src/index.js';
import {
  bindEvent,
  bindText,
  dynamic,
  nodePath,
  registerCompiledTemplate,
  repeat,
  slot,
} from '@microsoft/webui-test-support';

registerCompiledTemplate('test-split-repeat', {
  h: '<div class="controls"><button class="load">Load</button></div><ul class="primary"></ul><ul class="secondary"></ul>',
  repeats: [
    repeat('primaryItems', 'item', { blockIndex: 0 }),
    repeat('secondaryItems', 'item', { blockIndex: 1 }),
  ],
  repeatSlots: [
    slot({ parent: nodePath(1), before: 0 }),
    slot({ parent: nodePath(2), before: 0 }),
  ],
  blocks: [
    {
      h: '<li class="primary-item"></li>',
      text: [bindText(slot({ parent: nodePath(0), before: 0 }), dynamic('item.label'))],
    },
    {
      h: '<li class="secondary-item"></li>',
      text: [bindText(slot({ parent: nodePath(0), before: 0 }), dynamic('item.label'))],
    },
  ],
  events: [bindEvent('click', 'loadItems')],
  eventTargets: [nodePath(0, 0)],
});

interface SplitRepeatItem {
  label: string;
}

export class TestSplitRepeat extends WebUIElement {
  @observable primaryItems: SplitRepeatItem[] = [{ label: 'Seed Alpha' }, { label: 'Seed Beta' }];
  @observable secondaryItems: SplitRepeatItem[] = [{ label: 'Seed One' }, { label: 'Seed Two' }];

  loadItems(): void {
    this.primaryItems = [{ label: 'Alpha' }, { label: 'Beta' }];
    this.secondaryItems = [{ label: 'One' }, { label: 'Two' }, { label: 'Three' }];
  }
}

TestSplitRepeat.define('test-split-repeat');


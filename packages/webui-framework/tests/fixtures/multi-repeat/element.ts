// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { WebUIElement, observable } from '../../../src/index.js';
import {
  attrTarget,
  bindAttr,
  bindText,
  dynamic,
  eq,
  nodePath,
  neq,
  registerCompiledTemplate,
  repeat,
  slot,
  when,
  stringLiteral,
} from '@microsoft/webui-test-support';

// Component with TWO <for> loops that both contain <if> conditionals.
// SSR emits global marker IDs (for-1, for-2, if-3...) but the runtime
// maps them to local block indices. This test verifies that the second
// loop's conditionals are correctly hydrated despite non-local IDs.
registerCompiledTemplate('test-multi-repeat', {
  h: '<ul class="list-a"></ul><ul class="list-b"></ul>',
  repeats: [
    repeat('items', 'item', { blockIndex: 0 }),
    repeat('items', 'item', { blockIndex: 0 }),
  ],
  repeatSlots: [
    slot({ parent: nodePath(0), before: 0 }),
    slot({ parent: nodePath(1), before: 0 }),
  ],
  blocks: [
    {
      h: '<li></li>',
      conditionals: [
        when(eq('item.active', stringLiteral('true')), { blockIndex: 1 }),
        when(neq('item.active', stringLiteral('true')), { blockIndex: 2 }),
      ],
      conditionSlots: [
        slot({ parent: nodePath(0), before: 0, order: 0 }),
        slot({ parent: nodePath(0), before: 0, order: 1 }),
      ],
    },
    {
      h: '<p class="current"></p>',
      text: [
        bindText(slot({ parent: nodePath(0), before: 0 }), dynamic('item.title')),
      ],
    },
    {
      h: '<a class="link"></a>',
      text: [
        bindText(slot({ parent: nodePath(0), before: 0 }), dynamic('item.title')),
      ],
      attrs: [bindAttr('href', 'item.href')],
      attrGroups: [attrTarget(nodePath(0), { startIndex: 0, bindingCount: 1 })],
    },
  ],
});

interface MultiRepeatItem {
  title: string;
  href: string;
  active: string;
}

export class TestMultiRepeat extends WebUIElement {
  @observable items: MultiRepeatItem[] = [];
}

TestMultiRepeat.define('test-multi-repeat');

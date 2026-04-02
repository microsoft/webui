// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { WebUIElement, observable } from '../../../src/index.js';
import {
  bindEvent,
  bindText,
  dynamic,
  identifier,
  nodePath,
  registerCompiledTemplate,
  repeat,
  slot,
  when,
} from '@microsoft/webui-test-support';

// Reproduces the sort-dropdown pattern:
//   <span class="label">
//     <for each="option in options">
//       <if condition="option.active">{{option.title}}</if>
//     </for>
//   </span>
//   <button class="update">Update</button>
//
// The <for> block body has NO root element — it's just an <if> with a text node.
// This tests that hydration correctly tracks text-only repeat items.

registerCompiledTemplate('test-text-only-repeat', {
  h: '<span class="label"></span><button class="update">Update</button>',
  repeats: [repeat('options', 'option', { blockIndex: 0, slot: { parent: nodePath(0), before: 0 } })],
  blocks: [
    // Block 0: for-body — just a conditional, no wrapper element
    {
      h: '',
      conditionals: [
        when(identifier('option.active'), { blockIndex: 1, slot: { parent: [], before: 0, order: 0 } }),
      ],
    },
    // Block 1: if-body — the text binding
    {
      h: '<span class="active-label"></span>',
      text: [
        bindText(slot({ parent: nodePath(0), before: 0 }), dynamic('option.title')),
      ],
    },
  ],
  events: [bindEvent('click', 'onUpdate', false, nodePath(1))],
});

interface Option {
  title: string;
  active: boolean;
}

export class TestTextOnlyRepeat extends WebUIElement {
  @observable options: Option[] = [];

  onUpdate(): void {
    // Shift active from first to second option
    this.options = this.options.map((opt, i) => ({
      ...opt,
      active: i === 1,
    }));
  }
}

TestTextOnlyRepeat.define('test-text-only-repeat');

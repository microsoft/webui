// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { WebUIElement, attr } from '../../../src/index.js';
import {
  bindEvent,
  bindText,
  dynamic,
  nodePath,
  registerCompiledTemplate,
  slot,
} from '@microsoft/webui-test-support';

registerCompiledTemplate('test-ref', {
  h: '<input class="input" w-ref="{inputEl}"><span class="display"></span><button class="read">Read</button><button class="focus">Focus</button>',
  text: [
    bindText(slot({ parent: nodePath(1), before: 0 }), dynamic('value')),
  ],
  events: [
    bindEvent('click', 'readInput', false, nodePath(2)),
    bindEvent('click', 'focusInput', false, nodePath(3)),
  ],
});

export class TestRef extends WebUIElement {
  @attr value = 'hello';
  inputEl!: HTMLInputElement;

  readInput(): void {
    this.value = this.inputEl.value;
  }

  focusInput(): void {
    this.inputEl.focus();
  }
}

TestRef.define('test-ref');


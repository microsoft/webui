// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { WebUIElement, attr, observable } from '../../../src/index.js';
import {
  bindEvent,
  bindText,
  dynamic,
  nodePath,
  registerCompiledTemplate,
  slot,
} from '@microsoft/webui-test-support';

registerCompiledTemplate('test-counter', {
  h: '<span class="label"></span>: <span class="count"></span> (<span class="doubled"></span>)<button class="inc">+</button><button class="dec">-</button>',
  text: [
    bindText(slot({ parent: nodePath(0), before: 0 }), dynamic('label')),
    bindText(slot({ parent: nodePath(2), before: 0 }), dynamic('count')),
    bindText(slot({ parent: nodePath(4), before: 0 }), dynamic('doubled')),
  ],
  events: [
    bindEvent('click', 'increment', false, nodePath(6)),
    bindEvent('click', 'decrement', false, nodePath(7)),
  ],
});

export class TestCounter extends WebUIElement {
  @attr label = 'Clicks';
  @observable count = 0;
  @observable doubled = 0;

  increment(): void {
    this.count += 1;
    this.doubled = this.count * 2;
  }

  decrement(): void {
    this.count -= 1;
    this.doubled = this.count * 2;
  }
}

TestCounter.define('test-counter');


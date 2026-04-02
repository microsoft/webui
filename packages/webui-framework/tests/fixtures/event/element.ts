// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { WebUIElement, observable } from '../../../src/index.js';
import {
  bindEvent,
  bindText,
  dynamic,
  nodePath,
  registerCompiledTemplate,
  slot,
} from '@microsoft/webui-test-support';

registerCompiledTemplate('test-event', {
  h: '<span class="count"></span><button class="inc">+</button><button class="dec">-</button><button class="reset">Reset</button>',
  text: [
    bindText(slot({ parent: nodePath(0), before: 0 }), dynamic('count')),
  ],
  events: [
    bindEvent('click', 'onIncrement', false, nodePath(1)),
    bindEvent('click', 'onDecrement', false, nodePath(2)),
    bindEvent('click', 'onReset', false, nodePath(3)),
  ],
});

export class TestEvent extends WebUIElement {
  @observable count = 0;

  onIncrement(): void {
    this.count += 1;
  }

  onDecrement(): void {
    this.count -= 1;
  }

  onReset(): void {
    this.count = 0;
  }
}

TestEvent.define('test-event');


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

registerCompiledTemplate('test-root-event', {
  h: '<span class="total"></span><button class="action" data-action="ping">Ping</button><button class="other" data-action="pong">Pong</button>',
  text: [
    bindText(slot({ parent: nodePath(0), before: 0 }), dynamic('totalClicks')),
  ],
  rootEvents: [
    bindEvent('click', 'onRootClick', true),
  ],
});

export class TestRootEvent extends WebUIElement {
  @observable totalClicks = 0;

  lastAction = '';

  onRootClick(e: MouseEvent): void {
    this.totalClicks += 1;
    const target = e.composedPath()[0] as HTMLElement;
    const actionEl = target?.closest?.('[data-action]') as HTMLElement | null;
    if (actionEl) {
      this.lastAction = actionEl.dataset.action ?? '';
    }
  }
}

TestRootEvent.define('test-root-event');

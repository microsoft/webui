// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { WebUIElement, observable } from '../../../src/index.js';

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

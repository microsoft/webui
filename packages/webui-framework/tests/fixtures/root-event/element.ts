// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { WebUIElement, observable } from '../../../src/index.js';

/**
 * Root-level `@event` coverage. The binding lives on the host, so `e.target` is
 * retargeted to the host and `e.composedPath()[0]` recovers the real origin.
 */
export class TestRootEvent extends WebUIElement {
  @observable totalClicks = 0;

  lastAction = '';
  lastClickTarget = '';
  lastClickOrigin = '';

  onRootClick(e: MouseEvent): void {
    this.totalClicks += 1;
    this.lastClickTarget = (e.target as HTMLElement)?.tagName ?? '';
    const origin = e.composedPath()[0] as HTMLElement;
    this.lastClickOrigin = origin?.tagName ?? '';
    const actionEl = origin?.closest?.('[data-action]') as HTMLElement | null;
    if (actionEl) {
      this.lastAction = actionEl.dataset.action ?? '';
    }
  }
}

TestRootEvent.define('test-root-event');

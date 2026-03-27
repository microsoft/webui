// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { WebUIElement } from '../../../src/index.js';
import {
  bindEvent,
  nodePath,
  registerCompiledTemplate,
} from '@microsoft/webui-test-support';

registerCompiledTemplate('test-no-css-host', {
  h: '<button class="spawn">Spawn</button><div class="slot"></div>',
  events: [bindEvent('click', 'spawnChild')],
  eventTargets: [nodePath(0)],
});

registerCompiledTemplate('test-no-css-child', {
  h: '<p class="child-label">Ready</p>',
});

export class TestNoCssHost extends WebUIElement {
  spawnChild(): void {
    const slot = this.shadowRoot?.querySelector('.slot');
    if (!(slot instanceof HTMLDivElement)) {
      throw new Error('Missing .slot container');
    }

    if (!slot.querySelector('test-no-css-child')) {
      slot.appendChild(document.createElement('test-no-css-child'));
    }
  }
}

export class TestNoCssChild extends WebUIElement {}

TestNoCssHost.define('test-no-css-host');
TestNoCssChild.define('test-no-css-child');


// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { WebUIElement } from '../../../src/index.js';
import {
  bindEvent,
  nodePath,
  registerCompiledTemplate,
} from '@microsoft/webui-test-support';

registerCompiledTemplate('test-link-host', {
  h: '<link rel="stylesheet" href="/css-link/host.css"><button class="spawn">Spawn</button><p class="host-label">Host</p><div class="slot"></div>',
  events: [bindEvent('click', 'spawnChild', false, nodePath(1))],
});

registerCompiledTemplate('test-link-child', {
  h: '<link rel="stylesheet" href="/css-link/child.css"><p class="child-label">Child</p>',
});

export class TestLinkHost extends WebUIElement {
  spawnChild(): void {
    const root = this.shadowRoot ?? this;
    const slot = root.querySelector('.slot');
    if (!(slot instanceof HTMLDivElement)) {
      throw new Error('Missing .slot container');
    }

    if (!slot.querySelector('test-link-child')) {
      slot.appendChild(document.createElement('test-link-child'));
    }
  }
}

export class TestLinkChild extends WebUIElement {}

TestLinkHost.define('test-link-host');
TestLinkChild.define('test-link-child');


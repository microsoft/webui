// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { WebUIElement } from '../../../src/index.js';
import {
  bindEvent,
  nodePath,
  registerCompiledTemplate,
} from '@microsoft/webui-test-support';

registerCompiledTemplate('test-style-host', {
  h: '<style>.host-label{color:rgb(12, 34, 56);}</style><button class="spawn">Spawn</button><p class="host-label">Host</p><div class="slot"></div>',
  events: [bindEvent('click', 'spawnChild', false, nodePath(1))],
});

registerCompiledTemplate('test-style-child', {
  h: '<style>.child-label{color:rgb(210, 105, 30);}</style><p class="child-label">Child</p>',
});

export class TestStyleHost extends WebUIElement {
  spawnChild(): void {
    const slot = (this.shadowRoot ?? this).querySelector('.slot');
    if (!(slot instanceof HTMLDivElement)) {
      throw new Error('Missing .slot container');
    }

    if (!slot.querySelector('test-style-child')) {
      slot.appendChild(document.createElement('test-style-child'));
    }
  }
}

export class TestStyleChild extends WebUIElement {}

TestStyleHost.define('test-style-host');
TestStyleChild.define('test-style-child');


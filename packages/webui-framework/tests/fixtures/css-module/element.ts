// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { WebUIElement } from '../../../src/index.js';
import {
  bindEvent,
  nodePath,
  registerCompiledTemplate,
} from '@microsoft/webui-test-support';

registerCompiledTemplate('test-module-host', {
  h: '<button class="spawn">Spawn</button><p class="host-label">Host</p><div class="slot"></div>',
  adoptedStylesheet: 'test-module-host',
  events: [bindEvent('click', 'spawnChild')],
  eventTargets: [nodePath(0)],
});

registerCompiledTemplate('test-module-child', {
  h: '<p class="child-label">Child</p>',
  adoptedStylesheet: 'test-module-child',
});

export class TestModuleHost extends WebUIElement {
  spawnChild(): void {
    const slot = this.shadowRoot?.querySelector('.slot');
    if (!(slot instanceof HTMLDivElement)) {
      throw new Error('Missing .slot container');
    }

    if (!slot.querySelector('test-module-child')) {
      slot.appendChild(document.createElement('test-module-child'));
    }
  }
}

export class TestModuleChild extends WebUIElement {}

TestModuleHost.define('test-module-host');
TestModuleChild.define('test-module-child');


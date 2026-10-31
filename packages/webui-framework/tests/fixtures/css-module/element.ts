// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { startRuntime } from '../../../src/runtime.js';
import '../../../src/styles-entry.js';
import { WebUIElement } from '../../../src/index.js';

startRuntime();

export class TestModuleHost extends WebUIElement {
  spawnChild(): void {
    const slot = (this.shadowRoot ?? this).querySelector('.slot');
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

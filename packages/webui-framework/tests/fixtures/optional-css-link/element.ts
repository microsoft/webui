// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { startRuntime } from '../../../src/runtime.js';
import '../../../src/styles-entry.js';
import { WebUIElement } from '../../../src/index.js';

startRuntime();

export class TestNoCssHost extends WebUIElement {
  spawnChild(): void {
    const slot = (this.shadowRoot ?? this).querySelector('.slot');
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

// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { WebUIElement } from '../../../src/index.js';

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

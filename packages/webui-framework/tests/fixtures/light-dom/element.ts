// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { startRuntime } from '../../../src/runtime.js';
import '../../../src/styles-entry.js';
import { WebUIElement, observable } from '../../../src/index.js';

startRuntime();

export class TestLightDom extends WebUIElement {
  @observable greeting = 'Hello';
  @observable name = 'World';

  spawnChild(): void {
    const container = this.querySelector('.client-children');
    if (container) {
      container.appendChild(document.createElement('test-light-child'));
    }
  }
}

export class TestLightChild extends WebUIElement {}

export class TestShadowOptIn extends WebUIElement {}

export class TestShadowLightChild extends WebUIElement {}

TestLightDom.define('test-light-dom');
TestLightChild.define('test-light-child');
TestShadowOptIn.define('test-shadow-opt-in');
TestShadowLightChild.define('test-shadow-light-child');

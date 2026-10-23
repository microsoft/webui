// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { startRuntime } from '../../../src/runtime.js';
import '../../../src/styles-entry.js';
import '../../../src/lazy-hydration-entry.js';
import { attr, WebUIElement } from '../../../src/index.js';

startRuntime();

export class TestOrdinaryEagerItem extends WebUIElement {
  @attr label = 'Client default';

  protected override hydratedCallback(): void {
    this.setAttribute('data-hydrated', '');
  }
}
TestOrdinaryEagerItem.define('test-ordinary-eager-item');

export class TestOrdinaryLazyItem extends WebUIElement {
  @attr label = 'Client default';

  protected override hydratedCallback(): void {
    this.setAttribute('data-hydrated', '');
  }
}
TestOrdinaryLazyItem.define('test-ordinary-lazy-item');

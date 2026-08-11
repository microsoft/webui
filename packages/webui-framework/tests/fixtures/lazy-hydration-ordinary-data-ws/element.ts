// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import '../../../src/lazy-hydration-entry.js';
import { attr, WebUIElement } from '../../../src/index.js';

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

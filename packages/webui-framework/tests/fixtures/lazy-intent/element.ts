// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { startRuntime } from '../../../src/runtime.js';
import '../../../src/lazy-entry.js';
import { observable, WebUIElement } from '../../../src/index.js';

startRuntime();

/** Renders a tile beside an intent owner in its shadow tree. */
export class IntentApp extends WebUIElement {}
IntentApp.define('test-intent-app');

/** Owns a lazy region that prefetches on intent. */
export class IntentOwner extends WebUIElement {
  @observable open = false;

  show(): void { this.open = true; }
}
IntentOwner.define('test-intent-owner');

/** Reveals a block holding a lazy intent region only after it has mounted. */
export class IntentLater extends WebUIElement {
  @observable shown = false;
  @observable open = false;
}
IntentLater.define('test-intent-later');

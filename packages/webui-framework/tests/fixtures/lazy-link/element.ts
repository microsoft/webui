// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { startRuntime } from '../../../src/runtime.js';
import '../../../src/styles-entry.js';
import '../../../src/lazy-entry.js';
import { observable, WebUIElement } from '../../../src/index.js';

startRuntime();

/** Owns a lazy region whose body introduces a scriptless, link-styled component. */
export class LinkOwner extends WebUIElement {
  @observable open = false;

  show(): void { this.open = true; }
}
LinkOwner.define('test-link-owner');

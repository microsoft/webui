// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { startRuntime } from '../../../src/runtime.js';
import '../../../src/styles-entry.js';
import { defineLazyScripts } from '../../../src/lazy-entry.js';
import { observable, WebUIElement } from '../../../src/index.js';

// Stands in for the generated `lazy.webui.js`. The region's script is a chunk
// the server sends on its own, so a test can hold it like a slow bundler chunk.
defineLazyScripts({
  'test-link-owner::lazy:@0': {
    'test-link-script': () => import(new URL('chip.js', document.baseURI).href),
  },
});

startRuntime();

/** Owns a lazy region whose body introduces a scriptless, link-styled component. */
export class LinkOwner extends WebUIElement {
  @observable open = false;

  show(): void { this.open = true; }
}
LinkOwner.define('test-link-owner');

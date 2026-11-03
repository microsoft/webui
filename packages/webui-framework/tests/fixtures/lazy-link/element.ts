// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { startRuntime } from '@microsoft/webui-framework/runtime.js';
import '@microsoft/webui-framework/styles.js';
import { registerGeneratedLazyScripts } from '@microsoft/webui-framework/lazy.js';
import { observable, WebUIElement } from '@microsoft/webui-framework';

// Stands in for the generated `lazy.webui.js`. The region's script is a chunk
// the server sends on its own, so a test can hold it like a slow bundler chunk.
registerGeneratedLazyScripts({
  'test-link-owner::lazy:@0': ['test-link-script'],
}, {
  'test-link-script': () => import(new URL('chip.js', document.baseURI).href),
});

startRuntime();

/** Owns a lazy region whose body introduces a scriptless, link-styled component. */
export class LinkOwner extends WebUIElement {
  @observable open = false;

  show(): void { this.open = true; }
}
LinkOwner.define('test-link-owner');

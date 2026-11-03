// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import '@microsoft/webui-framework/styles.js';
import { startRuntime } from '@microsoft/webui-framework/runtime.js';
import { observable, WebUIElement } from '@microsoft/webui-framework';

class PagesShell extends WebUIElement {
  @observable pageA = false;
  @observable pageB = false;
  async first(): Promise<void> {
    await import('./src/preview-page.ts');
    this.pageA = true;
    this.pageB = false;
  }
  async second(): Promise<void> {
    await import('./src/unrelated-page.ts');
    this.pageB = true;
    this.pageA = false;
  }
}
PagesShell.define('pages-shell');
startRuntime();

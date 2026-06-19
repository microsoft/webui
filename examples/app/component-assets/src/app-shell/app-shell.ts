// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { WebUIElement, attr } from '@microsoft/webui-framework';
import { defineComponentAssets } from '@microsoft/webui-framework/component-asset.js';

const assets = defineComponentAssets({
  'lazy-panel': {
    asset: './lazy-panel.webui.json',
    module: () => import('../lazy-panel/lazy-panel.js'),
    data: async () => await (await fetch('./lazy-panel-data.json')).json(),
  },
});

export class AppShell extends WebUIElement {
  @attr title = '';

  panelSlot!: HTMLDivElement;

  async openPanel(): Promise<void> {
    this.panelSlot.replaceChildren(await assets.create('lazy-panel'));
  }
}

AppShell.define('app-shell');

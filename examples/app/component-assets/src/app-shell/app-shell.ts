// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { WebUIElement, attr } from '@microsoft/webui-framework';
import { defineComponentAssets } from '@microsoft/webui-framework/component-asset.js';

const assets = defineComponentAssets({
  'lazy-panel': {
    asset: './lazy-panel.webui.js',
    modulepreload: ['./chunk-shared-detail.webui.js'],
    data: async () => await (await fetch('./lazy-panel-data.json')).json(),
  },
  'secondary-panel': {
    asset: './secondary-panel.webui.js',
    modulepreload: ['./chunk-shared-detail.webui.js'],
  },
});

export class AppShell extends WebUIElement {
  @attr title = '';

  panelSlot!: HTMLDivElement;
  secondaryPanelSlot!: HTMLDivElement;

  async openPanel(): Promise<void> {
    this.panelSlot.replaceChildren(await assets.create('lazy-panel'));
  }

  async openSecondaryPanel(): Promise<void> {
    this.secondaryPanelSlot.replaceChildren(await assets.create('secondary-panel'));
  }
}

AppShell.define('app-shell');

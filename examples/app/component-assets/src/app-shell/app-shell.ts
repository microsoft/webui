// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { WebUIElement, attr } from '@microsoft/webui-framework';
import {
  defineComponentAsset,
  preloadComponentAssetStyles,
  type GeneratedComponentAsset,
} from '@microsoft/webui-framework/component-asset-runtime.js';
import { createRetryableLoader } from './asset-loader.js';

const loadLazyPanel = createRetryableLoader<GeneratedComponentAsset>(() =>
  import('../../.webui/lazy-panel.webui.js')
    .then(module => defineComponentAsset(module.default)),
);

const loadSecondaryPanel = createRetryableLoader<GeneratedComponentAsset>(() =>
  import('../../.webui/secondary-panel.webui.js')
    .then(module => defineComponentAsset(module.default)),
);

export class AppShell extends WebUIElement {
  @attr title = '';

  panelSlot!: HTMLDivElement;
  secondaryPanelSlot!: HTMLDivElement;

  preloadPanel(): void {
    preloadComponentAssetStyles('lazy-panel');
    void loadLazyPanel()
      .then(asset => asset.preload())
      .catch(() => undefined);
  }

  preloadSecondaryPanel(): void {
    preloadComponentAssetStyles('secondary-panel');
    void loadSecondaryPanel()
      .then(asset => asset.preload())
      .catch(() => undefined);
  }

  async openPanel(): Promise<void> {
    preloadComponentAssetStyles('lazy-panel');
    const [asset, state] = await Promise.all([
      loadLazyPanel(),
      fetch('./lazy-panel-data.json').then(response => response.json()),
    ]);
    const panel = await asset.create();
    (panel as HTMLElement & { setState(value: unknown): void }).setState(state);
    this.panelSlot.replaceChildren(panel);
  }

  async openSecondaryPanel(): Promise<void> {
    preloadComponentAssetStyles('secondary-panel');
    const asset = await loadSecondaryPanel();
    this.secondaryPanelSlot.replaceChildren(await asset.create());
  }
}

AppShell.define('app-shell');

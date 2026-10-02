// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { WebUIElement, attr } from '@microsoft/webui-framework';
import {
  defineComponentAsset,
  preloadComponentAssetStyles,
  type GeneratedComponentAsset,
} from '@microsoft/webui-framework/component-asset-runtime.js';

let lazyPanelAsset: Promise<GeneratedComponentAsset> | undefined;
let secondaryPanelAsset: Promise<GeneratedComponentAsset> | undefined;

const loadLazyPanel = (): Promise<GeneratedComponentAsset> => {
  lazyPanelAsset ??= import('../../.webui/lazy-panel.webui.js')
    .then(module => defineComponentAsset(module.default));
  return lazyPanelAsset;
};

const loadSecondaryPanel = (): Promise<GeneratedComponentAsset> => {
  secondaryPanelAsset ??= import('../../.webui/secondary-panel.webui.js')
    .then(module => defineComponentAsset(module.default));
  return secondaryPanelAsset;
};

export class AppShell extends WebUIElement {
  @attr title = '';

  panelSlot!: HTMLDivElement;
  secondaryPanelSlot!: HTMLDivElement;

  preloadPanel(): void {
    preloadComponentAssetStyles('lazy-panel');
    void loadLazyPanel().then(asset => asset.preload());
  }

  preloadSecondaryPanel(): void {
    preloadComponentAssetStyles('secondary-panel');
    void loadSecondaryPanel().then(asset => asset.preload());
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

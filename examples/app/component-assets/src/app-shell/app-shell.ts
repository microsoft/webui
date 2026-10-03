// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { WebUIElement, attr, observable } from '@microsoft/webui-framework';
import {
  defineComponentAsset,
  preloadComponentAssetStyles,
} from '@microsoft/webui-framework/component-asset-runtime.js';

const loadLazyPanel = () =>
  import('../../.webui/lazy-panel.webui.js')
    .then(module => defineComponentAsset(module.default));

const loadSecondaryPanel = () =>
  import('../../.webui/secondary-panel.webui.js')
    .then(module => defineComponentAsset(module.default));

export class AppShell extends WebUIElement {
  @attr title = '';
  @observable loadError = '';

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
    try {
      preloadComponentAssetStyles('lazy-panel');
      const [asset, state] = await Promise.all([
        loadLazyPanel(),
        fetch('./lazy-panel-data.json').then(response => {
          if (!response.ok) throw new Error(`Panel data request failed: HTTP ${response.status}`);
          return response.json();
        }),
      ]);
      const panel = await asset.create();
      (panel as HTMLElement & { setState(value: unknown): void }).setState(state);
      this.panelSlot.replaceChildren(panel);
      this.loadError = '';
    } catch (error) {
      this.reportLoadError(error);
    }
  }

  async openSecondaryPanel(): Promise<void> {
    try {
      preloadComponentAssetStyles('secondary-panel');
      const asset = await loadSecondaryPanel();
      this.secondaryPanelSlot.replaceChildren(await asset.create());
      this.loadError = '';
    } catch (error) {
      this.reportLoadError(error);
    }
  }

  reloadPage(): void {
    location.reload();
  }

  private reportLoadError(error: unknown): void {
    console.error('Panel loading failed:', error);
    this.loadError = 'The panel could not load. Reload the page to try again.';
  }
}

AppShell.define('app-shell');

// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { WebUIElement, attr } from '@microsoft/webui-framework';

const loadLazyPanel = () => import('../../.webui/lazy-panel.webui.js');
const loadSecondaryPanel = () => import('../../.webui/secondary-panel.webui.js');

export class AppShell extends WebUIElement {
  @attr title = '';

  panelSlot!: HTMLDivElement;
  secondaryPanelSlot!: HTMLDivElement;

  preloadPanel(): void {
    void loadLazyPanel().then(asset => asset.preload());
  }

  preloadSecondaryPanel(): void {
    void loadSecondaryPanel().then(asset => asset.preload());
  }

  async openPanel(): Promise<void> {
    const [asset, state] = await Promise.all([
      loadLazyPanel(),
      fetch('./lazy-panel-data.json').then(response => response.json()),
    ]);
    const panel = await asset.create();
    (panel as HTMLElement & { setState(value: unknown): void }).setState(state);
    this.panelSlot.replaceChildren(panel);
  }

  async openSecondaryPanel(): Promise<void> {
    const asset = await loadSecondaryPanel();
    this.secondaryPanelSlot.replaceChildren(await asset.create());
  }
}

AppShell.define('app-shell');

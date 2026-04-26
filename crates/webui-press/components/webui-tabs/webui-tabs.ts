// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { WebUIElement, observable } from "@microsoft/webui-framework";

export class WebUITabs extends WebUIElement {
  @observable activeIndex = 0;

  connectedCallback(): void {
    super.connectedCallback();

    // Listen for tab-select events from child webui-tab components
    this.addEventListener("tab-select", ((e: CustomEvent) => {
      this.onTabSelect(e);
    }) as EventListener);

    // Find initially active tab
    const tabs = [...this.querySelectorAll(":scope > webui-tab")];
    const activeIdx = tabs.findIndex((t) => t.hasAttribute("active"));
    this.activeIndex = activeIdx >= 0 ? activeIdx : 0;
    this.syncActive();
  }

  onTabSelect(e: CustomEvent): void {
    const tab = e.detail.tab as Element;
    const tabs = [...this.querySelectorAll(":scope > webui-tab")];
    const idx = tabs.indexOf(tab);
    if (idx >= 0) {
      this.activeIndex = idx;
      this.syncActive();
    }
  }

  private syncActive(): void {
    const tabs = this.querySelectorAll(":scope > webui-tab");
    const panels = this.querySelectorAll(":scope > webui-tab-panel");
    tabs.forEach((tab, i) => {
      if (i === this.activeIndex) tab.setAttribute("active", "");
      else tab.removeAttribute("active");
    });
    panels.forEach((panel, i) => {
      if (i === this.activeIndex) panel.setAttribute("active", "");
      else panel.removeAttribute("active");
    });
  }
}

WebUITabs.define("webui-tabs");

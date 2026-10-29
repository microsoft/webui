// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { WebUIElement, attr } from "@microsoft/webui-framework";

interface TabSelection {
  value: string;
  members: Set<WebUIPressTabs>;
  chosen?: boolean;
  storageKey?: string;
  storage?: Storage;
}

const pageSelections = new WeakMap<Document, Map<string, TabSelection>>();

export class WebUIPressTabs extends WebUIElement {
  @attr sync = "";
  @attr storage = "";

  private activeIndex = 0;
  private selection?: TabSelection;

  connectedCallback(): void {
    super.connectedCallback();

    const tabs = this.querySelectorAll(":scope > webui-press-tab");
    this.activeIndex = 0;
    for (let index = 0; index < tabs.length; index++) {
      if (tabs[index].hasAttribute("active")) {
        this.activeIndex = index;
        break;
      }
    }
    this.syncActive();
    this.joinSelection();
  }

  disconnectedCallback(): void {
    this.selection?.members.delete(this);
    this.selection = undefined;
    super.disconnectedCallback();
  }

  onTabSelect(e: CustomEvent<{ tab: Element }>): void {
    const tab = e.detail.tab;
    if (tab.parentElement !== this) return;
    const tabs = this.querySelectorAll(":scope > webui-press-tab");
    for (let idx = 0; idx < tabs.length; idx++) {
      if (tabs[idx] !== tab) continue;
      this.activeIndex = idx;
      this.syncActive();
      if (this.selection) {
        const value = tab.getAttribute("value");
        if (!value) {
          console.error("[WebUI Press] Synchronized tabs require a nonempty value attribute.");
          return;
        }
        this.selection.value = value;
        this.selection.chosen = true;
        this.saveSelection(this.selection);
        for (const member of this.selection.members) {
          if (member !== this) member.selectValue(value);
        }
      }
      return;
    }
  }

  private joinSelection(): void {
    if (!this.sync) {
      if (this.storage) {
        console.error("[WebUI Press] Tab storage requires a sync category.");
      }
      return;
    }
    if (this.storage && this.storage !== "local") {
      console.error('[WebUI Press] Unsupported tab storage; use storage="local" or omit it.');
    }
    const value = this.querySelector(":scope > webui-press-tab[active]")?.getAttribute("value");
    if (!value) {
      console.error("[WebUI Press] Synchronized tabs require a nonempty value attribute.");
      return;
    }
    let groups = pageSelections.get(this.ownerDocument);
    if (!groups) {
      groups = new Map();
      pageSelections.set(this.ownerDocument, groups);
    }
    let selection = groups.get(this.sync);
    if (!selection) {
      selection = { value, members: new Set() };
      groups.set(this.sync, selection);
    }
    this.selection = selection;
    this.restoreSelection(selection);
    selection.members.add(this);
    this.selectValue(selection.value);
  }

  private restoreSelection(selection: TabSelection): void {
    if (this.storage !== "local" || selection.storageKey) return;
    selection.storageKey = `webui-press:tabs:${this.sync}`;
    let stored: string | null;
    try {
      const storage = this.ownerDocument.defaultView?.localStorage;
      if (!storage) {
        console.warn("[WebUI Press] Tab preferences cannot be restored without localStorage; selection remains page-local.");
        return;
      }
      stored = storage.getItem(selection.storageKey);
      selection.storage = storage;
    } catch (error) {
      console.warn("[WebUI Press] Cannot restore tab preferences; selection remains page-local.", error);
      return;
    }
    if (stored !== null && !selection.chosen) {
      selection.value = stored;
      for (const member of selection.members) member.selectValue(stored);
    }
    if (selection.chosen) this.saveSelection(selection);
  }

  private saveSelection(selection: TabSelection): void {
    if (!selection.storage || !selection.storageKey) return;
    try {
      selection.storage.setItem(selection.storageKey, selection.value);
    } catch (error) {
      selection.storage = undefined;
      console.warn("[WebUI Press] Cannot save tab preferences; selection remains page-local.", error);
    }
  }

  private selectValue(value: string): void {
    const tabs = this.querySelectorAll(":scope > webui-press-tab");
    for (let index = 0; index < tabs.length; index++) {
      if (tabs[index].getAttribute("value") === value) {
        this.activeIndex = index;
        this.syncActive();
        return;
      }
    }
  }

  private syncActive(): void {
    const tabs = this.querySelectorAll(":scope > webui-press-tab");
    const panels = this.querySelectorAll(":scope > webui-press-tab-panel");
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

WebUIPressTabs.define("webui-press-tabs");

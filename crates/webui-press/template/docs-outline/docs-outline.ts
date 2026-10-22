// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { WebUIElement } from "@microsoft/webui-framework";

interface OutlineHeading {
  text: string;
  anchor: string;
  level: number;
}

export class DocsOutline extends WebUIElement {
  declare headings: OutlineHeading[];
  declare path: string;
  outline!: HTMLOListElement;
  private listeners?: AbortController;
  private current: HTMLAnchorElement | null = null;
  private frame = 0;
  private dirty = false;
  private recheck = true;

  override connectedCallback(): void {
    super.connectedCallback();
    if (this.outline) this.startFollowing();
  }

  protected override hydratedCallback(): void {
    this.startFollowing();
  }

  override disconnectedCallback(): void {
    this.listeners?.abort();
    this.listeners = undefined;
    if (this.frame) cancelAnimationFrame(this.frame);
    this.frame = 0;
    this.dirty = false;
    this.recheck = true;
    this.current?.removeAttribute("aria-current");
    this.current = null;
    super.disconnectedCallback();
  }

  private startFollowing(): void {
    if (this.listeners || !CSS.supports("scroll-target-group: auto") ||
      !CSS.supports("selector(a:target-current)")) return;
    const main = this.closest<HTMLElement>(".main-content");
    if (!main) {
      console.warn("[WebUI Press] Place docs-outline inside .main-content to enable section following.");
      return;
    }
    this.listeners = new AbortController();
    const { signal } = this.listeners;
    main.addEventListener("scroll", this.schedule, { passive: true, signal });
    main.addEventListener("scrollend", this.schedule, { signal });
    this.outline.addEventListener("focusout", this.remeasure, { signal });
    window.addEventListener("resize", this.remeasure, { passive: true, signal });
    window.addEventListener("pageshow", this.remeasure, { signal });
    window.addEventListener("hashchange", this.remeasure, { signal });
    this.schedule();
  }

  private schedule = (): void => {
    if (!this.isConnected || !this.outline.checkVisibility()) return;
    this.dirty = true;
    if (!this.frame) this.frame = requestAnimationFrame(this.afterScroll);
  };

  private afterScroll = (): void => {
    this.dirty = false;
    this.frame = requestAnimationFrame(this.reveal);
  };

  private reveal = (): void => {
    this.frame = 0;
    // Scrolls arriving between the two frames still need a settled native marker.
    if (this.dirty) this.schedule();
    if (!this.isConnected || !this.outline.checkVisibility()) return;
    const next = this.outline.querySelector<HTMLAnchorElement>("a:target-current");
    const changed = next !== this.current;
    if (changed) {
      this.current?.removeAttribute("aria-current");
      this.current = next;
      this.current?.setAttribute("aria-current", "location");
    }
    const shouldReveal = changed || this.recheck;
    this.recheck = false;
    if (!shouldReveal || !this.current || this.outline.matches(":focus-within")) return;
    const bounds = this.outline.getBoundingClientRect();
    const link = this.current.getBoundingClientRect();
    const delta = link.top < bounds.top + 8
      ? link.top - bounds.top - 8
      : Math.max(0, link.bottom - bounds.bottom + 8);
    if (delta) this.outline.scrollBy({ top: delta, behavior: "auto" });
  };

  private remeasure = (): void => {
    this.recheck = true;
    this.schedule();
  };
}

DocsOutline.define("docs-outline");

// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { WebUIElement, observable } from "@microsoft/webui-framework";

export class DocsAppearance extends WebUIElement {
  @observable ready = false;
  @observable mobile = false;
  panel!: HTMLDivElement;
  trigger!: HTMLButtonElement;
  private media?: MediaQueryList;
  private resize = (): void => {
    const supported = typeof this.panel.showPopover === "function";
    if (supported && this.panel.matches(":popover-open")) this.panel.hidePopover();
    this.mobile = !supported || this.media?.matches === true;
  };

  override connectedCallback(): void {
    super.connectedCallback();
    if (this.ready) this.follow();
  }

  protected override hydratedCallback(): void {
    this.follow();
    this.ready = true;
  }

  private follow(): void {
    this.media?.removeEventListener("change", this.resize);
    this.media = matchMedia("(max-width: 1024px)");
    this.resize();
    this.media.addEventListener("change", this.resize);
  }

  override disconnectedCallback(): void {
    this.media?.removeEventListener("change", this.resize);
    super.disconnectedCallback();
  }

  onToggle(event: ToggleEvent): void {
    if (event.newState !== "closed" || this.mobile) return;
    let active = this.ownerDocument.activeElement;
    while (active) {
      // Native restoration does not cover focused controls in slotted shadow roots.
      if (this.contains(active) || this.shadowRoot?.contains(active)) {
        this.trigger.focus({ preventScroll: true });
        return;
      }
      active = active.shadowRoot?.activeElement ?? null;
    }
  }
}

DocsAppearance.define("docs-appearance");

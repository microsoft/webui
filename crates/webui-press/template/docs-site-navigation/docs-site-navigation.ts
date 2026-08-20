// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { WebUIElement, attr, observable } from "@microsoft/webui-framework";

import "../docs-search/docs-search.js";
import "../docs-theme-toggle/docs-theme-toggle.js";

interface NavigationLink {
  text: string;
  link: string;
  section: string;
  fullLayout: boolean;
}

interface SidebarItem {
  text: string;
  link: string;
  active: boolean;
  expanded: boolean;
  hasChildren: boolean;
  children: SidebarItem[];
}

interface SidebarSection {
  title: string;
  items: SidebarItem[];
}

export class DocsSiteNavigation extends WebUIElement {
  @attr section = "";
  @attr({ mode: "boolean" }) home = false;
  @attr({ attribute: "full-layout", mode: "boolean" }) fullLayout = false;
  @observable navigation: NavigationLink[] = [];
  @observable open = false;

  declare sections: SidebarSection[];
  dialog!: HTMLDialogElement;
  private pageSwapAttached = false;
  private skipNextTransition = false;

  private resizeHandler = (): void => {
    if (window.innerWidth > 768) {
      this.closeMenu();
    }
  };
  private pageSwapHandler = (event: Event): void => {
    if (!this.skipNextTransition) return;
    const pageSwap = event as Event & {
      viewTransition?: {
        ready: Promise<void>;
        skipTransition(): void;
      } | null;
    };
    const transition = pageSwap.viewTransition;
    if (transition) {
      void transition.ready.catch((error: unknown) => {
        if (error instanceof DOMException && error.name === "AbortError") {
          return;
        }
        console.error("[WebUI Press] View transition failed", error);
      });
      transition.skipTransition();
    }
    this.skipNextTransition = false;
  };

  connectedCallback(): void {
    super.connectedCallback();
    window.addEventListener("resize", this.resizeHandler);
    if (!this.pageSwapAttached) {
      window.addEventListener("pageswap", this.pageSwapHandler);
      this.pageSwapAttached = true;
    }
  }

  disconnectedCallback(): void {
    super.disconnectedCallback();
    window.removeEventListener("resize", this.resizeHandler);
    // Chromium dispatches `pageswap` after custom elements disconnect. Keep
    // this listener for the old window's remaining lifetime so it can cancel
    // transitions into or out of full-layout pages.
  }

  openMenu(): void {
    if (this.dialog.open) return;
    this.dialog.showModal();
    this.open = true;
  }

  closeMenu(): void {
    if (this.dialog.open) {
      this.dialog.close();
    }
  }

  onDialogClick(event: Event): void {
    if (event.target === this.dialog) {
      this.closeMenu();
    }
  }

  onDialogClose(): void {
    this.open = false;
  }

  onNavigate(targetFullLayout: boolean, event: MouseEvent): void {
    const primaryNavigation =
      event.button === 0 &&
      !event.altKey &&
      !event.ctrlKey &&
      !event.metaKey &&
      !event.shiftKey;
    this.skipNextTransition =
      primaryNavigation && Boolean(this.fullLayout || targetFullLayout);
  }
}

DocsSiteNavigation.define("docs-site-navigation");

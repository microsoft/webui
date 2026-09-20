// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { WebUIElement, attr, observable } from '@microsoft/webui-framework';

import '#molecules/mp-search-bar/mp-search-bar.js';

export class MpMobileMenu extends WebUIElement {
  @attr({ attribute: 'search-query' }) searchQuery = '';
  @observable open = false;
  panelEl!: HTMLElement;

  private resizeHandler = (): void => {
    if (window.innerWidth >= 768) {
      this.closeMenu();
    }
  };

  connectedCallback(): void {
    super.connectedCallback();
    window.addEventListener('resize', this.resizeHandler);
  }

  disconnectedCallback(): void {
    super.disconnectedCallback();
    window.removeEventListener('resize', this.resizeHandler);
  }

  onCloseClick(): void {
    this.closeMenu();
  }

  onBackdropClick(): void {
    this.closeMenu();
  }

  openMenu(): void {
    this.open = true;
    this.panelEl.showPopover();
  }

  closeMenu(): void {
    this.open = false;
    this.panelEl.hidePopover();
  }
}

MpMobileMenu.define('mp-mobile-menu');

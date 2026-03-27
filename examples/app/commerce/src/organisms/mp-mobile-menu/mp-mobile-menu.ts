// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { WebUIElement, attr, observable } from '@microsoft/webui-framework';
import { Router } from '@microsoft/webui-router';

import '#molecules/mp-search-bar/mp-search-bar.js';

interface Category {
  handle: string;
  title: string;
}

export class MpMobileMenu extends WebUIElement {
  @attr({ attribute: 'search-query' }) searchQuery = '';
  @observable navCategories: Category[] = [];
  panelEl!: HTMLElement;
  backdropEl!: HTMLElement;

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

  onLinkClick(e: MouseEvent): void {
    const target = e.currentTarget;
    if (!(target instanceof HTMLAnchorElement)) {
      return;
    }

    e.preventDefault();
    Router.navigate(target.href);
    this.closeMenu();
  }

  onCloseClick(): void {
    this.closeMenu();
  }

  onBackdropClick(): void {
    this.closeMenu();
  }

  openMenu(): void {
    this.panelEl.showPopover();
    this.backdropEl.classList.add('open');
  }

  closeMenu(): void {
    this.panelEl.hidePopover();
    this.backdropEl.classList.remove('open');
  }
}

MpMobileMenu.define('mp-mobile-menu');

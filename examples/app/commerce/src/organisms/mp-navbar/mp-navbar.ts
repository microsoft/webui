// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { WebUIElement, attr, observable } from '@microsoft/webui-framework';
import { Router } from '@microsoft/webui-router';

import '#molecules/mp-search-bar/mp-search-bar.js';

export class MpNavbar extends WebUIElement {
  @attr({ attribute: 'store-name' }) storeName = 'Acme Store';
  @attr({ attribute: 'search-query' }) searchQuery = '';
  @attr({ attribute: 'cart-href' }) cartHref = '/?cart=open';
  @observable cartItems!: unknown[];
  @observable navCategories: { handle: string; title: string }[] = [];

  onCartClick(e: MouseEvent): void {
    e.preventDefault();
    this.openCart();
  }

  onMenuClick(): void {
    this.$emit('toggle-mobile-menu');
  }

  onNavigateClick(e: MouseEvent): void {
    const target = e.currentTarget;
    if (!(target instanceof HTMLAnchorElement)) {
      return;
    }

    e.preventDefault();
    Router.navigate(target.href);
  }

  openCart(): void {
    this.$emit('toggle-cart');
  }
}

MpNavbar.define('mp-navbar');

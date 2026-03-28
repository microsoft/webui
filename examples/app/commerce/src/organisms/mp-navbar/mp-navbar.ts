// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { WebUIElement, attr, observable } from '@microsoft/webui-framework';

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

  openCart(): void {
    this.$emit('toggle-cart');
  }
}

MpNavbar.define('mp-navbar');

// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { WebUIElement, observable } from '@microsoft/webui-framework';

import '#organisms/mp-navbar/mp-navbar.js';
import '#organisms/mp-mobile-menu/mp-mobile-menu.js';
import '#organisms/mp-cart-panel/mp-cart-panel.js';
import '#organisms/mp-footer/mp-footer.js';

interface NavCategory {
  handle: string;
  title: string;
}

interface MobileMenuController extends HTMLElement {
  openMenu(): void;
}

interface CartStateDetail {
  cartItems?: any[];
  subtotal?: string;
  taxes?: string;
  cartOpen?: string;
  cartHref?: string;
  cartCloseHref?: string;
  currentPath?: string;
}

export class MpApp extends WebUIElement {
  @observable storeName!: string;
  @observable searchQuery!: string;
  @observable currentPath!: string;
  @observable cartOpen!: string;
  @observable cartHref!: string;
  @observable cartCloseHref!: string;
  @observable subtotal!: string;
  @observable taxes!: string;
  @observable navCategories!: NavCategory[];
  @observable cartItems!: any[];
  mobileMenu!: MobileMenuController;

  onCommerceCartState(event: Event): void {
    const detail = (event as CustomEvent<CartStateDetail>).detail;
    if (Array.isArray(detail.cartItems)) {
      this.cartItems = detail.cartItems;
    }
    if (typeof detail.subtotal === 'string') {
      this.subtotal = detail.subtotal;
    }
    if (typeof detail.taxes === 'string') {
      this.taxes = detail.taxes;
    }
    if (typeof detail.cartOpen === 'string') {
      this.cartOpen = detail.cartOpen;
    }
    if (typeof detail.cartHref === 'string') {
      this.cartHref = detail.cartHref;
    }
    if (typeof detail.cartCloseHref === 'string') {
      this.cartCloseHref = detail.cartCloseHref;
    }
    if (typeof detail.currentPath === 'string') {
      this.currentPath = detail.currentPath;
    }
  }

  onToggleCart(): void {
    this.cartOpen = 'true';
  }

  onToggleMobileMenu(): void {
    this.mobileMenu?.openMenu();
  }

  onCartClosed(): void {
    this.cartOpen = '';
  }
}

MpApp.define('mp-app');

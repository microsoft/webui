// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { WebUIElement, attr, observable } from '@microsoft/webui-framework';

import '#atoms/mp-price/mp-price.js';
import '#atoms/mp-product-image/mp-product-image.js';

interface CartItem {
  handle: string;
  title: string;
  color: string;
  size: string;
  variantLabel: string;
  price: string;
  quantity: number;
  gradient: string;
  imageUrl: string;
  increaseTo: number;
  decreaseTo: number;
  redirectTo: string;
}

export class MpCartPanel extends WebUIElement {
  @observable cartItems!: CartItem[];
  @attr subtotal!: string;
  @attr taxes!: string;
  @attr({ attribute: 'cart-open' }) cartOpen!: string;
  @attr({ attribute: 'cart-close-href' }) cartCloseHref!: string;
  @attr({ attribute: 'current-path' }) currentPath!: string;

  onCloseClick(e: MouseEvent): void {
    e.preventDefault();
    this.closeCart();
  }

  onBackdropClick(e: MouseEvent): void {
    e.preventDefault();
    this.closeCart();
  }

  onQuantityClick(e: MouseEvent): void {
    e.preventDefault();
    const target = e.currentTarget;
    if (!(target instanceof HTMLElement)) {
      return;
    }

    void this.handleQuantity(target);
  }

  async handleQuantity(btn: HTMLElement): Promise<void> {
    const handle = btn.getAttribute('data-handle') || '';
    const color = btn.getAttribute('data-color') || '';
    const size = btn.getAttribute('data-size') || '';
    const quantity = parseInt(btn.getAttribute('data-quantity') || '0', 10);
    if (!handle || Number.isNaN(quantity)) {
      return;
    }

    await this.submitCartMutation('./cart/update', {
      handle,
      color,
      size,
      quantity,
      redirectTo: this.currentPath,
      openCart: true,
    });
  }

  closeCart(): void {
    this.cartOpen = '';
    this.$emit('cart-closed');
  }

  private async submitCartMutation(url: string, payload: Record<string, unknown>): Promise<void> {
    const response = await fetch(url, {
      method: 'POST',
      headers: {
        Accept: 'application/json',
        'Content-Type': 'application/json',
      },
      body: JSON.stringify(payload),
      credentials: 'same-origin',
    });
    if (!response.ok) {
      return;
    }
    const state = await response.json() as Record<string, unknown>;
    this.$emit('commerce-cart-state', state);
  }
}

MpCartPanel.define('mp-cart-panel');

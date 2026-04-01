// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { WebUIElement, attr } from '@microsoft/webui-framework';

export class MpAddToCart extends WebUIElement {
  @attr handle = '';
  @attr({ attribute: 'product-title' }) productTitle = '';
  @attr price = '';
  @attr gradient = '';
  @attr({ attribute: 'image-url' }) imageUrl = '';
  @attr({ attribute: 'default-color' }) defaultColor = '';
  @attr({ attribute: 'default-size' }) defaultSize = '';
  @attr({ attribute: 'selected-color' }) selectedColor = '';
  @attr({ attribute: 'selected-size' }) selectedSize = '';
  @attr({ attribute: 'current-path' }) currentPath = '/';

  connectedCallback(): void {
    super.connectedCallback();
    if (!this.selectedColor) this.selectedColor = this.defaultColor;
    if (!this.selectedSize) this.selectedSize = this.defaultSize;
  }

  async onSubmit(e: SubmitEvent): Promise<void> {
    e.preventDefault();
    await this.submitCart();
  }

  private async submitCart(): Promise<void> {
    const response = await fetch('/cart/add', {
      method: 'POST',
      headers: {
        Accept: 'application/json',
        'Content-Type': 'application/json',
      },
      body: JSON.stringify({
        handle: this.handle,
        color: this.selectedColor,
        size: this.selectedSize,
        redirectTo: this.currentPath,
        openCart: true,
      }),
      credentials: 'same-origin',
    });
    if (!response.ok) {
      return;
    }

    const state = await response.json() as Record<string, unknown>;
    this.$emit('commerce-cart-state', state);
  }
}

MpAddToCart.define('mp-add-to-cart');

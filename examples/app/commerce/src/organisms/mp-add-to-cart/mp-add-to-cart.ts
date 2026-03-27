// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { WebUIElement, attr, observable } from '@microsoft/webui-framework';

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
  @observable canSubmit = true;

  connectedCallback(): void {
    super.connectedCallback();
    this.syncSelectionState();
  }

  selectedColorChanged(): void {
    this.syncSelectionState();
  }

  selectedSizeChanged(): void {
    this.syncSelectionState();
  }

  defaultColorChanged(): void {
    this.syncSelectionState();
  }

  defaultSizeChanged(): void {
    this.syncSelectionState();
  }

  async onSubmit(e: SubmitEvent): Promise<void> {
    e.preventDefault();
    await this.submitCart();
  }

  private async submitCart(): Promise<void> {
    this.syncSelectionState();
    if (!this.canSubmit) {
      return;
    }

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

  private syncSelectionState(): void {
    if (!this.selectedColor) {
      this.selectedColor = this.defaultColor;
    }
    if (!this.selectedSize) {
      this.selectedSize = this.defaultSize;
    }

    this.canSubmit = (!this.defaultColor || this.selectedColor !== '')
      && (!this.defaultSize || this.selectedSize !== '');
  }
}

MpAddToCart.define('mp-add-to-cart');

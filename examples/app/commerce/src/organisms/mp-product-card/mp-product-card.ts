// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { WebUIElement, attr } from '@microsoft/webui-framework';
import { Router } from '@microsoft/webui-router';

import '#atoms/mp-product-image/mp-product-image.js';
import '#molecules/mp-product-label/mp-product-label.js';

export class MpProductCard extends WebUIElement {
  @attr handle = '';
  @attr title = '';
  @attr price = '';
  @attr gradient = '';
  @attr({ attribute: 'image-url' }) imageUrl = '';
  @attr variant = 'grid';
  @attr({ attribute: 'image-loading' }) imageLoading = 'lazy';
  @attr({ attribute: 'image-fetch-priority' }) imageFetchPriority = 'auto';

  handleChanged(): void {
    this.applyViewTransitionName();
  }

  private applyViewTransitionName(): void {
    if (!this.handle) return;
    this.style.viewTransitionName = `product-image-${this.handle}`;
  }

  onClick(event: MouseEvent): void {
    const href = (event.currentTarget as HTMLAnchorElement | null)?.getAttribute('href');
    if (!href) {
      return;
    }

    event.preventDefault();
    Router.navigate(href);
  }
}

MpProductCard.define('mp-product-card');

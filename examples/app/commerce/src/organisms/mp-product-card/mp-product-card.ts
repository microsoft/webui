// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { WebUIElement, attr } from '@microsoft/webui-framework';

export class MpProductCard extends WebUIElement {
  @attr handle = '';
  @attr title = '';
  @attr price = '';
  @attr gradient = '';
  @attr({ attribute: 'image-url' }) imageUrl = '';
  @attr variant = 'grid';
  @attr({ attribute: 'image-loading' }) imageLoading = 'lazy';
  @attr({ attribute: 'image-fetch-priority' }) imageFetchPriority = 'auto';
  @attr({ attribute: 'image-width' }) imageWidth = '640';
  @attr({ attribute: 'image-height' }) imageHeight = '640';

  protected override propertiesChanged(changes: ReadonlyMap<string, unknown>): void {
    if (changes.has('handle')) this.applyViewTransitionName();
  }

  private applyViewTransitionName(): void {
    if (!this.handle) return;
    this.style.viewTransitionName = `product-image-${this.handle}`;
  }
}

MpProductCard.define('mp-product-card');

// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { FASTElement, attr } from '@microsoft/fast-element';
import { RenderableFASTElement } from '@microsoft/fast-html';

import '#atoms/mp-product-image/mp-product-image.js';
import '#molecules/mp-product-label/mp-product-label.js';

export class MpProductCard extends RenderableFASTElement(FASTElement) {
  @attr handle!: string;
  @attr title!: string;
  @attr price!: string;
  @attr gradient!: string;
  @attr({ attribute: 'image-url' }) imageUrl!: string;
  @attr variant!: string;
  @attr({ attribute: 'image-loading' }) imageLoading!: string;
  @attr({ attribute: 'image-fetch-priority' }) imageFetchPriority!: string;

  async prepare(): Promise<void> {
    this.handle = this.getAttribute('handle') || '';
    this.title = this.getAttribute('title') || '';
    this.price = this.getAttribute('price') || '';
    this.gradient = this.getAttribute('gradient') || '';
    this.imageUrl = this.getAttribute('image-url') || '';
    this.variant = this.getAttribute('variant') || 'grid';
    this.imageLoading = this.getAttribute('image-loading') || 'lazy';
    this.imageFetchPriority = this.getAttribute('image-fetch-priority') || 'auto';
    this.applyViewTransitionName();
  }

  handleChanged(): void {
    this.applyViewTransitionName();
  }

  private applyViewTransitionName(): void {
    if (!this.handle) return;
    this.style.viewTransitionName = `product-image-${this.handle}`;
  }
}

MpProductCard.defineAsync({
  name: 'mp-product-card',
  templateOptions: 'defer-and-hydrate',
});

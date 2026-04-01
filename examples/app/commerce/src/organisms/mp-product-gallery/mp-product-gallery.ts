// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { WebUIElement, attr, observable } from '@microsoft/webui-framework';

import '#atoms/mp-product-image/mp-product-image.js';

interface GalleryImage {
  index: number;
  gradient: string;
  imageUrl: string;
}

export class MpProductGallery extends WebUIElement {
  @attr({ attribute: 'active-gradient' }) activeGradient = '';
  @attr({ attribute: 'active-image-url' }) activeImageUrl = '';
  @attr handle = '';
  @observable images!: GalleryImage[];
  @observable activeIndex = 0;

  handleChanged(): void {
    this.applyViewTransitionName();
  }

  activeIndexChanged(): void {
    const images = this.galleryImages();
    const active = images[this.activeIndex];
    if (active) {
      this.activeGradient = active.gradient;
      this.activeImageUrl = active.imageUrl;
    }
  }

  onPreviousClick(): void {
    const images = this.galleryImages();
    if (images.length === 0) {
      return;
    }

    this.activeIndex = (this.activeIndex - 1 + images.length) % images.length;
  }

  onNextClick(): void {
    const images = this.galleryImages();
    if (images.length === 0) {
      return;
    }

    this.activeIndex = (this.activeIndex + 1) % images.length;
  }

  onThumbnailClick(e: MouseEvent): void {
    const images = this.galleryImages();
    if (images.length === 0) {
      return;
    }

    const target = e.currentTarget;
    if (!(target instanceof HTMLElement)) {
      return;
    }

    const indexStr = target.getAttribute('data-index');
    if (indexStr == null) {
      return;
    }

    const index = Number.parseInt(indexStr, 10);
    if (!Number.isNaN(index) && index >= 0 && index < images.length) {
      this.activeIndex = index;
    }
  }

  private galleryImages(): GalleryImage[] {
    return Array.isArray(this.images) ? this.images : [];
  }

  private applyViewTransitionName(): void {
    if (this.handle) {
      this.style.viewTransitionName = `product-image-${this.handle}`;
    } else {
      this.style.removeProperty('view-transition-name');
    }
  }
}
 
MpProductGallery.define('mp-product-gallery');

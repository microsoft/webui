// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { FASTElement, attr, observable } from '@microsoft/fast-element';
import { RenderableFASTElement } from '@microsoft/fast-html';

import '#atoms/mp-icon/mp-icon.js';
import '#atoms/mp-product-image/mp-product-image.js';

interface GalleryImage {
  index: number;
  gradient: string;
  imageUrl: string;
  activeClass: string;
}

export class MpProductGallery extends RenderableFASTElement(FASTElement) {
  @attr({ attribute: 'active-gradient' }) activeGradient!: string;
  @attr({ attribute: 'active-image-url' }) activeImageUrl!: string;
  @attr handle = '';
  @observable images!: GalleryImage[];

  private activeIndex = 0;

  private clickHandler = (e: Event): void => { this.onClick(e as MouseEvent); };

  connectedCallback(): void {
    super.connectedCallback();
    this.addEventListener('click', this.clickHandler);
  }

  disconnectedCallback(): void {
    super.disconnectedCallback();
    this.removeEventListener('click', this.clickHandler);
  }

  async prepare(): Promise<void> {
    if (Array.isArray(this.images) && this.images.length > 0) return;

    this.handle = this.getAttribute('handle') || '';
    const thumbs = this.shadowRoot!.querySelectorAll('.thumb');
    const images: GalleryImage[] = [];
    thumbs.forEach((el, i) => {
      const media = el.querySelector('mp-product-image');
      images.push({
        index: i,
        gradient: media?.getAttribute('gradient') || '',
        imageUrl: media?.getAttribute('image-url') || '',
        activeClass: i === 0 ? 'active' : '',
      });
    });
    this.images = images;
    if (images.length > 0) {
      this.activeGradient = images[0].gradient;
      this.activeImageUrl = images[0].imageUrl;
    }
    this.applyViewTransitionName();
  }

  setInitialState(state: Record<string, unknown>): void {
    if (Array.isArray(state.images)) {
      this.images = (state.images as any[]).map((img, i) => ({
        index: i,
        gradient: String(img.gradient || ''),
        imageUrl: String(img.imageUrl || ''),
        activeClass: i === 0 ? 'active' : '',
      }));
    }
    if (typeof state.gradient === 'string') this.activeGradient = state.gradient;
    if (typeof state.imageUrl === 'string') this.activeImageUrl = state.imageUrl;
    if (typeof state.handle === 'string') this.handle = state.handle;
    this.applyViewTransitionName();
    const view = this.$fastController?.view;
    if (view) {
      view.unbind();
      view.bind(this, view.context);
    }
  }

  handleChanged(): void {
    this.applyViewTransitionName();
  }

  private applyViewTransitionName(): void {
    if (!this.handle) return;
    this.style.viewTransitionName = `product-image-${this.handle}`;
  }

  onClick(e: MouseEvent): void {
    const actionTarget = this.findPathElement(e, '[data-action]');
    const action = actionTarget?.getAttribute('data-action');
    if (!action) return;

    if (action === 'prev') {
      this.activeIndex = (this.activeIndex - 1 + this.images.length) % this.images.length;
    } else if (action === 'next') {
      this.activeIndex = (this.activeIndex + 1) % this.images.length;
    } else if (action === 'select') {
      const indexStr = this.findPathElement(e, '[data-index]')?.getAttribute('data-index');
      if (indexStr != null) {
        this.activeIndex = parseInt(indexStr, 10);
      }
    }

    this.updateActive();
  }

  private updateActive(): void {
    const active = this.images[this.activeIndex];
    this.activeGradient = active?.gradient || '';
    this.activeImageUrl = active?.imageUrl || '';
    this.images = this.images.map((img, i) => ({
      index: img.index,
      gradient: img.gradient,
      imageUrl: img.imageUrl,
      activeClass: i === this.activeIndex ? 'active' : '',
    }));
  }

  private findPathElement(event: Event, selector: string): Element | null {
    for (const target of event.composedPath()) {
      if (target instanceof Element && target.matches(selector)) {
        return target;
      }
    }

    return null;
  }
}

MpProductGallery.defineAsync({
  name: 'mp-product-gallery',
  templateOptions: 'defer-and-hydrate',
});

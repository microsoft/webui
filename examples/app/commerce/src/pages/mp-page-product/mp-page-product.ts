// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { FASTElement, attr, observable } from '@microsoft/fast-element';
import { RenderableFASTElement } from '@microsoft/fast-html';

import '#atoms/mp-price/mp-price.js';
import '#organisms/mp-product-gallery/mp-product-gallery.js';
import '#organisms/mp-variant-selector/mp-variant-selector.js';
import '#organisms/mp-add-to-cart/mp-add-to-cart.js';
import '#organisms/mp-product-card/mp-product-card.js';

export class MpPageProduct extends RenderableFASTElement(FASTElement) {
  // Product fields as individual @attr
  @attr handle = '';
  @attr({ attribute: 'product-title' }) productTitle = '';
  @attr price = '';
  @attr gradient = '';
  @attr({ attribute: 'gradient-alt' }) gradientAlt = '';
  @attr({ attribute: 'image-url' }) imageUrl = '';
  @attr({ attribute: 'image-alt-url' }) imageAltUrl = '';
  @attr({ attribute: 'compare-at' }) compareAt = '';
  @attr({ attribute: 'has-compare-at' }) hasCompareAt = '';
  @attr({ attribute: 'description-html' }) descriptionHtml = '';
  @attr({ attribute: 'default-color' }) defaultColor = '';
  @attr({ attribute: 'default-size' }) defaultSize = '';
  @attr({ attribute: 'current-path' }) currentPath = '';

  // Arrays as optional @observable
  @observable images?: any[];
  @observable optionGroups?: any[];
  @observable relatedProducts?: any[];
  @observable categories?: any[];
  @attr({ attribute: 'all-active-class' }) allActiveClass = '';

  async prepare(): Promise<void> {
    // Read state from data-state attribute (set by SSR handler)
    const raw = this.getAttribute('data-state');
    if (raw) {
      try {
        const state = JSON.parse(raw);
        this.handle = String(state.handle ?? '');
        this.productTitle = String(state.productTitle ?? state.title ?? '');
        this.price = String(state.price ?? '');
        this.gradient = String(state.gradient ?? '');
        this.gradientAlt = String(state.gradientAlt ?? '');
        this.imageUrl = String(state.imageUrl ?? '');
        this.imageAltUrl = String(state.imageAltUrl ?? '');
        this.compareAt = String(state.compareAt ?? '');
        this.hasCompareAt = state.compareAt ? 'true' : '';
        this.descriptionHtml = String(state.descriptionHtml ?? '');
        this.defaultColor = String(state.defaultColor ?? '');
        this.defaultSize = String(state.defaultSize ?? '');
        this.currentPath = String(state.currentPath ?? '');
        if (Array.isArray(state.images)) this.images = state.images;
        if (Array.isArray(state.optionGroups)) this.optionGroups = state.optionGroups;
        if (Array.isArray(state.relatedProducts)) this.relatedProducts = state.relatedProducts;
        if (Array.isArray(state.categories)) this.categories = state.categories;
        if (state.allActiveClass !== undefined) this.allActiveClass = String(state.allActiveClass);
        this.emitCatalogNavState();
        await this.syncChildren();
        return;
      } catch { /* fall through to DOM extraction */ }
    }

    const sr = this.shadowRoot;
    if (!sr) return;

    const titleEl = sr.querySelector('.product-title');
    if (!titleEl) return;

    this.productTitle = titleEl.textContent?.trim() || '';
    this.price = sr.querySelector('mp-price')?.getAttribute('value') || '';
    const compareEl = sr.querySelector('.product-price-compare');
    this.compareAt = compareEl?.textContent?.trim() || '';
    this.hasCompareAt = compareEl ? 'true' : '';
    const descEl = sr.querySelector('.product-description');
    this.descriptionHtml = descEl?.innerHTML || '';

    const gallery = sr.querySelector('mp-product-gallery');
    this.gradient = gallery?.getAttribute('active-gradient') || '';
    this.imageUrl = gallery?.getAttribute('active-image-url') || '';

    const addToCart = sr.querySelector('mp-add-to-cart');
    this.handle = addToCart?.getAttribute('handle') || '';

    // Read related products from SSR'd cards
    const relatedCards = sr.querySelectorAll('.related-scroll mp-product-card');
    if (relatedCards.length > 0) {
      const items: any[] = [];
      relatedCards.forEach((card) => {
        const el = card as HTMLElement;
        items.push({
          handle: el.getAttribute('handle') || '',
          title: el.getAttribute('title') || '',
          price: el.getAttribute('price') || '',
          gradient: el.getAttribute('gradient') || '',
          imageUrl: el.getAttribute('image-url') || '',
        });
      });
      this.relatedProducts = items;
    }
  }

  setInitialState(state: Record<string, unknown>): void {
    this.handle = String(state.handle ?? '');
    this.productTitle = String(state.productTitle ?? '');
    this.price = String(state.price ?? '');
    this.gradient = String(state.gradient ?? '');
    this.gradientAlt = String(state.gradientAlt ?? '');
    this.imageUrl = String(state.imageUrl ?? '');
    this.imageAltUrl = String(state.imageAltUrl ?? '');
    this.compareAt = String(state.compareAt ?? '');
    this.hasCompareAt = state.hasCompareAt ? 'true' : '';
    this.descriptionHtml = String(state.descriptionHtml ?? '');
    this.defaultColor = String(state.defaultColor ?? '');
    this.defaultSize = String(state.defaultSize ?? '');
    this.currentPath = String(state.currentPath ?? '');

    if (Array.isArray(state.images)) {
      this.images = state.images as any[];
    }
    if (Array.isArray(state.optionGroups)) {
      this.optionGroups = state.optionGroups as any[];
    }
    if (Array.isArray(state.relatedProducts)) {
      this.relatedProducts = state.relatedProducts as any[];
    }
    if (Array.isArray(state.categories)) {
      this.categories = state.categories as any[];
    }
    if (state.allActiveClass !== undefined) {
      this.allActiveClass = String(state.allActiveClass);
    }

    this.emitCatalogNavState();
    this.syncChildren();
  }

  private emitCatalogNavState(): void {
    document.dispatchEvent(new CustomEvent('commerce:catalog-nav-state', {
      detail: {
        categories: this.categories,
        allActiveClass: this.allActiveClass,
      },
    }));
  }

  private async syncChildren(): Promise<void> {
    const sr = this.shadowRoot;
    if (!sr) return;

    await Promise.all([
      customElements.whenDefined('mp-product-gallery'),
      customElements.whenDefined('mp-variant-selector'),
    ]);
    await new Promise<void>(r => requestAnimationFrame(() => r()));

    await this.pushAndRebind(sr, 'mp-product-gallery', {
      images: this.images,
      activeGradient: this.gradient,
      activeImageUrl: this.imageUrl,
    });
    await this.pushAndRebind(sr, 'mp-variant-selector', {
      optionGroups: this.optionGroups,
    });
    await this.pushAndRebind(sr, 'mp-add-to-cart', {
      handle: this.handle,
      productTitle: this.productTitle,
      price: this.price,
      gradient: this.gradient,
      imageUrl: this.imageUrl,
      defaultColor: this.defaultColor,
      defaultSize: this.defaultSize,
      currentPath: this.currentPath,
    });
    const addToCart = sr.querySelector('mp-add-to-cart') as { updateSelectionState?: () => void } | null;
    addToCart?.updateSelectionState?.();

    const descEl = sr.querySelector('.product-description');
    if (descEl && this.descriptionHtml) descEl.innerHTML = this.descriptionHtml;
  }

  private async pushAndRebind(
    sr: ShadowRoot,
    tag: string,
    data: Record<string, unknown>,
  ): Promise<void> {
    const el = sr.querySelector(tag) as any;
    if (!el) return;
    for (const [key, value] of Object.entries(data)) {
      if (value !== undefined) {
        delete el[key];
        el[key] = value;
      }
    }

    for (let attempt = 0; attempt < 5; attempt += 1) {
      const view = el.$fastController?.view;
      if (view) {
        view.unbind();
        view.bind(el, view.context);
        return;
      }

      await new Promise<void>(resolve => requestAnimationFrame(() => resolve()));
    }
  }
}

MpPageProduct.defineAsync({
  name: 'mp-page-product',
  templateOptions: 'defer-and-hydrate',
});

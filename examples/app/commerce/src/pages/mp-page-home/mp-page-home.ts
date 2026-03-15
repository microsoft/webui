import { FASTElement, observable } from '@microsoft/fast-element';
import { RenderableFASTElement } from '@microsoft/fast-html';

import '#organisms/mp-hero-grid/mp-hero-grid.js';
import '#organisms/mp-carousel/mp-carousel.js';

export class MpPageHome extends RenderableFASTElement(FASTElement) {
  @observable featuredProducts?: any[];
  @observable carouselProducts?: any[];

  async prepare(): Promise<void> {
    const raw = this.getAttribute('data-state');
    if (raw) {
      try {
        const state = JSON.parse(raw);
        if (Array.isArray(state.featuredProducts)) this.featuredProducts = state.featuredProducts;
        if (Array.isArray(state.carouselProducts)) this.carouselProducts = state.carouselProducts;
      } catch { /* ignore */ }
    }

    if (!this.featuredProducts || !this.carouselProducts) {
      const sr = this.shadowRoot;
      if (!sr) return;

      const heroGrid = sr.querySelector('mp-hero-grid');
      if (heroGrid?.shadowRoot && !this.featuredProducts) {
        const featured: any[] = [];
        heroGrid.shadowRoot.querySelectorAll('mp-product-card').forEach((card) => {
          const el = card as HTMLElement;
          featured.push({
            handle: el.getAttribute('handle') || '',
            title: el.getAttribute('title') || '',
            price: el.getAttribute('price') || '',
            gradient: el.getAttribute('gradient') || '',
            imageUrl: el.getAttribute('image-url') || '',
          });
        });
        if (featured.length > 0) this.featuredProducts = featured;
      }

      const carousel = sr.querySelector('mp-carousel');
      if (carousel?.shadowRoot && !this.carouselProducts) {
        const items: any[] = [];
        carousel.shadowRoot.querySelectorAll('mp-product-card').forEach((card) => {
          const el = card as HTMLElement;
          items.push({
            handle: el.getAttribute('handle') || '',
            title: el.getAttribute('title') || '',
            price: el.getAttribute('price') || '',
            gradient: el.getAttribute('gradient') || '',
            imageUrl: el.getAttribute('image-url') || '',
          });
        });
        if (items.length > 0) this.carouselProducts = items;
      }
    }
    // SSR hydration: children hydrate from their own SSR content
  }

  setInitialState(state: Record<string, unknown>): void {
    if (Array.isArray(state.featuredProducts)) {
      this.featuredProducts = state.featuredProducts as any[];
    }
    if (Array.isArray(state.carouselProducts)) {
      this.carouselProducts = state.carouselProducts as any[];
    }
    this.syncChildren();
  }

  private async syncChildren(): Promise<void> {
    const sr = this.shadowRoot;
    if (!sr) return;

    await Promise.all([
      customElements.whenDefined('mp-hero-grid'),
      customElements.whenDefined('mp-carousel'),
    ]);
    await new Promise<void>(r => requestAnimationFrame(() => r()));

    this.pushAndRebind(sr, 'mp-hero-grid', { products: this.featuredProducts });
    this.pushAndRebind(sr, 'mp-carousel', { products: this.carouselProducts });
  }

  private pushAndRebind(
    sr: ShadowRoot,
    tag: string,
    data: Record<string, unknown>,
  ): void {
    const el = sr.querySelector(tag) as any;
    if (!el) return;
    for (const [key, value] of Object.entries(data)) {
      if (value !== undefined) {
        delete el[key];
        el[key] = value;
      }
    }
    const view = el.$fastController?.view;
    if (view) {
      view.unbind();
      view.bind(el, view.context);
    }
  }
}

MpPageHome.defineAsync({
  name: 'mp-page-home',
  templateOptions: 'defer-and-hydrate',
});

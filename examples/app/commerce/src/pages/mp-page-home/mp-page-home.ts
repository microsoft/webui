// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { FASTElement, observable } from '@microsoft/fast-element';
import { RenderableFASTElement } from '@microsoft/fast-html';

import '#organisms/mp-hero-grid/mp-hero-grid.js';
import '#organisms/mp-carousel/mp-carousel.js';

function waitForView(el: any, maxFrames = 10): Promise<void> {
  return new Promise<void>((resolve) => {
    let frame = 0;
    const check = (): void => {
      if (el.$fastController?.view) { resolve(); return; }
      if (++frame >= maxFrames) { resolve(); return; }
      requestAnimationFrame(check);
    };
    check();
  });
}

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
        await this.initChildren(state);
        return;
      } catch { /* ignore */ }
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
    void this.initChildren(state);
  }

  private async initChildren(state: Record<string, unknown>): Promise<void> {
    const sr = this.shadowRoot;
    if (!sr) return;

    await Promise.all([
      customElements.whenDefined('mp-hero-grid'),
      customElements.whenDefined('mp-carousel'),
    ]);

    const heroGrid = sr.querySelector('mp-hero-grid') as any;
    if (heroGrid) {
      await waitForView(heroGrid);
      heroGrid.setInitialState?.(state);
    }

    const carousel = sr.querySelector('mp-carousel') as any;
    if (carousel) {
      await waitForView(carousel);
      carousel.setInitialState?.(state);
    }
  }
}

MpPageHome.defineAsync({
  name: 'mp-page-home',
  templateOptions: 'defer-and-hydrate',
});

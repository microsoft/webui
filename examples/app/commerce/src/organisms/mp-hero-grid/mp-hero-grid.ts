// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { FASTElement, observable } from '@microsoft/fast-element';
import { RenderableFASTElement } from '@microsoft/fast-html';

import '#organisms/mp-product-card/mp-product-card.js';

export class MpHeroGrid extends RenderableFASTElement(FASTElement) {
  @observable products?: any[];

  async prepare(): Promise<void> {
    if (Array.isArray(this.products) && this.products.length > 0) return;

    const sr = this.shadowRoot;
    if (!sr) return;
    const cards = sr.querySelectorAll('mp-product-card');
    if (cards.length === 0) return;
    const items: any[] = [];
    cards.forEach((card) => {
      const a = card as HTMLElement;
      items.push({
        handle: a.getAttribute('handle') || '',
        title: a.getAttribute('title') || '',
        price: a.getAttribute('price') || '',
        gradient: a.getAttribute('gradient') || '',
        imageUrl: a.getAttribute('image-url') || '',
      });
    });
    this.products = items;
  }

  setInitialState(state: Record<string, unknown>): void {
    if (Array.isArray(state.featuredProducts)) {
      this.products = state.featuredProducts as any[];
    }
    const view = this.$fastController?.view;
    if (view) {
      view.unbind();
      view.bind(this, view.context);
    }
  }
}

MpHeroGrid.defineAsync({
  name: 'mp-hero-grid',
  templateOptions: 'defer-and-hydrate',
});

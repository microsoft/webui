// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { FASTElement, attr, observable } from '@microsoft/fast-element';
import { RenderableFASTElement } from '@microsoft/fast-html';

import '#organisms/mp-product-card/mp-product-card.js';

export class MpProductGrid extends RenderableFASTElement(FASTElement) {
  @observable products?: any[];
  @attr query = '';

  async prepare(): Promise<void> {
    this.query = this.getAttribute('query') || '';
    const sr = this.shadowRoot;
    if (!sr) return;
    const cards = sr.querySelectorAll('mp-product-card');
    if (cards.length === 0) return;
    const items: any[] = [];
    cards.forEach((card) => {
      const el = card as HTMLElement;
      const handle = el.getAttribute('handle') || '';
      items.push({
        handle,
        title: el.getAttribute('title') || '',
        price: el.getAttribute('price') || '',
        gradient: el.getAttribute('gradient') || '',
        imageUrl: el.getAttribute('image-url') || '',
      });
    });
    this.products = items;
  }

  setInitialState(state: Record<string, unknown>): void {
    if (Array.isArray(state.products)) {
      this.products = state.products;
    }
    if (typeof state.query === 'string') {
      this.query = state.query;
    }
    const view = this.$fastController?.view;
    if (view) {
      view.unbind();
      view.bind(this, view.context);
    }
  }
}

MpProductGrid.defineAsync({
  name: 'mp-product-grid',
  templateOptions: 'defer-and-hydrate',
});

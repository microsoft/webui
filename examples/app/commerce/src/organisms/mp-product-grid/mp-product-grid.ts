// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { FASTElement, observable } from '@microsoft/fast-element';
import { RenderableFASTElement } from '@microsoft/fast-html';

import '#organisms/mp-product-card/mp-product-card.js';

export class MpProductGrid extends RenderableFASTElement(FASTElement) {
  @observable products?: any[];

  async prepare(): Promise<void> {
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
}

MpProductGrid.defineAsync({
  name: 'mp-product-grid',
  templateOptions: 'defer-and-hydrate',
});

import { FASTElement, observable } from '@microsoft/fast-element';
import { RenderableFASTElement } from '@microsoft/fast-html';

import '#organisms/mp-product-card/mp-product-card.js';

export class MpHeroGrid extends RenderableFASTElement(FASTElement) {
  @observable products?: any[];

  async prepare(): Promise<void> {
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
}

MpHeroGrid.defineAsync({
  name: 'mp-hero-grid',
  templateOptions: 'defer-and-hydrate',
});

import { FASTElement, observable } from '@microsoft/fast-element';
import { RenderableFASTElement } from '@microsoft/fast-html';

import '#organisms/mp-product-card/mp-product-card.js';

export class MpCarousel extends RenderableFASTElement(FASTElement) {
  @observable products!: {
    handle: string;
    title: string;
    price: string;
    gradient: string;
    imageUrl?: string;
  }[];

  async prepare(): Promise<void> {
    const items: {
      handle: string;
      title: string;
      price: string;
      gradient: string;
      imageUrl?: string;
    }[] = [];
    this.shadowRoot?.querySelectorAll('mp-product-card').forEach(card => {
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

MpCarousel.defineAsync({
  name: 'mp-carousel',
  templateOptions: 'defer-and-hydrate',
});

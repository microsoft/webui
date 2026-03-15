import { FASTElement, attr } from '@microsoft/fast-element';
import { RenderableFASTElement } from '@microsoft/fast-html';

import '#atoms/mp-icon/mp-icon.js';

export class MpSearchBar extends RenderableFASTElement(FASTElement) {
  @attr action = '/search';
  @attr query = '';
  @attr placeholder = 'Search for products...';
  @attr variant = 'desktop';
  @attr label = 'Search for products';

  async prepare(): Promise<void> {
    this.action = this.getAttribute('action') || '/search';
    this.query = this.getAttribute('query') || '';
    this.placeholder = this.getAttribute('placeholder') || 'Search for products...';
    this.variant = this.getAttribute('variant') || 'desktop';
    this.label = this.getAttribute('label') || 'Search for products';
  }
}

MpSearchBar.defineAsync({
  name: 'mp-search-bar',
  templateOptions: 'defer-and-hydrate',
});

// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { FASTElement, attr } from '@microsoft/fast-element';
import { RenderableFASTElement } from '@microsoft/fast-html';

import '#atoms/mp-price/mp-price.js';

export class MpProductLabel extends RenderableFASTElement(FASTElement) {
  @attr title = '';
  @attr price = '';
  @attr({ attribute: 'price-size' }) priceSize = 'sm';

  async prepare(): Promise<void> {
    this.title = this.getAttribute('title') || '';
    this.price = this.getAttribute('price') || '';
    this.priceSize = this.getAttribute('price-size') || 'sm';
  }
}

MpProductLabel.defineAsync({
  name: 'mp-product-label',
  templateOptions: 'defer-and-hydrate',
});

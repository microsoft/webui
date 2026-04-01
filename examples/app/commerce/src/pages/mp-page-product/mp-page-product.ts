// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { WebUIElement, observable } from '@microsoft/webui-framework';

import '#atoms/mp-price/mp-price.js';
import '#organisms/mp-product-gallery/mp-product-gallery.js';
import '#organisms/mp-variant-selector/mp-variant-selector.js';
import '#organisms/mp-add-to-cart/mp-add-to-cart.js';
import '#organisms/mp-product-card/mp-product-card.js';

export class MpPageProduct extends WebUIElement {
  @observable handle!: string;
  @observable productTitle!: string;
  @observable price!: string;
  @observable gradient!: string;
  @observable gradientAlt!: string;
  @observable imageUrl!: string;
  @observable imageAltUrl!: string;
  @observable compareAt!: string;
  @observable hasCompareAt!: boolean;
  @observable descriptionHtml!: string;
  @observable defaultColor!: string;
  @observable defaultSize!: string;
  @observable selectedColor!: string;
  @observable selectedSize!: string;
  @observable currentPath!: string;

  @observable images!: any[];
  @observable optionGroups!: any[];
  @observable relatedProducts!: any[];

  onVariantSelect(event: Event): void {
    const { group, value } = (event as CustomEvent).detail;
    const name = (group as string).trim().toLowerCase();
    if (name.includes('color')) this.selectedColor = value;
    else if (name.includes('size')) this.selectedSize = value;
  }
}

MpPageProduct.define('mp-page-product');

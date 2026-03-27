// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { WebUIElement, observable } from '@microsoft/webui-framework';

import '#organisms/mp-product-card/mp-product-card.js';

export class MpCarousel extends WebUIElement {
  @observable products: {
    handle: string;
    title: string;
    price: string;
    gradient: string;
    imageUrl?: string;
  }[] = [];
}

MpCarousel.define('mp-carousel');

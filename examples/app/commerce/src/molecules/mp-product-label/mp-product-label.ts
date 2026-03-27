// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { WebUIElement, attr } from '@microsoft/webui-framework';

import '#atoms/mp-price/mp-price.js';

export class MpProductLabel extends WebUIElement {
  @attr title = '';
  @attr price = '';
  @attr({ attribute: 'price-size' }) priceSize = 'sm';
}

MpProductLabel.define('mp-product-label');

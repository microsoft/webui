// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { WebUIElement, observable } from '@microsoft/webui-framework';

import '#organisms/mp-product-card/mp-product-card.js';

export class MpProductGrid extends WebUIElement {
  @observable products: any[] = [];
  @observable query = '';
}

MpProductGrid.define('mp-product-grid');

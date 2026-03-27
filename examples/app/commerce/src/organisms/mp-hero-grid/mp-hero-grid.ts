// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { WebUIElement, observable } from '@microsoft/webui-framework';

import '#organisms/mp-product-card/mp-product-card.js';

export class MpHeroGrid extends WebUIElement {
  @observable products: any[] = [];
}

MpHeroGrid.define('mp-hero-grid');

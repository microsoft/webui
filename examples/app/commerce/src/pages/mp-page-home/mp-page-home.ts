// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { WebUIElement, observable } from '@microsoft/webui-framework';

import '#organisms/mp-hero-grid/mp-hero-grid.js';
import '#organisms/mp-carousel/mp-carousel.js';

export class MpPageHome extends WebUIElement {
  @observable featuredProducts!: any[];
  @observable carouselProducts!: any[];
}

MpPageHome.define('mp-page-home');

// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { WebUIElement, observable } from '@microsoft/webui-framework';

import '#organisms/mp-category-nav/mp-category-nav.js';
import '#organisms/mp-filter-list/mp-filter-list.js';

export class MpPageSearch extends WebUIElement {
  @observable categories!: any[];
  @observable sortOptions!: any[];
  @observable allActiveClass!: string;
  @observable currentCategoryLabel!: string;
}

MpPageSearch.define('mp-page-search');

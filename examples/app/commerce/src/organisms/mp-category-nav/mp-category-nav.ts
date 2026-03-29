// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { WebUIElement, attr, observable } from '@microsoft/webui-framework';

export class MpCategoryNav extends WebUIElement {
  @attr({ attribute: 'all-active' }) allActive = '';
  @attr({ attribute: 'current-label' }) currentCategoryLabel = 'All';
  @observable categories: any[] = [];
  mobileDropdown!: HTMLDetailsElement;

  closeMobileDropdown(): void {
    if (this.mobileDropdown) {
      this.mobileDropdown.open = false;
    }
  }
}

MpCategoryNav.define('mp-category-nav');

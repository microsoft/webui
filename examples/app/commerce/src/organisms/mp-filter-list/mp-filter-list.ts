// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { WebUIElement, observable } from '@microsoft/webui-framework';

export class MpFilterList extends WebUIElement {
  @observable sortOptions: any[] = [];
  mobileDropdown!: HTMLDetailsElement;

  closeMobileDropdown(): void {
    if (this.mobileDropdown) {
      this.mobileDropdown.open = false;
    }
  }
}

MpFilterList.define('mp-filter-list');

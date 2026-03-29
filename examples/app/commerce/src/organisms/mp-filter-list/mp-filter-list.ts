// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { WebUIElement, observable } from '@microsoft/webui-framework';
import { Router } from '@microsoft/webui-router';

export class MpFilterList extends WebUIElement {
  @observable sortOptions: any[] = [];
  mobileDropdown!: HTMLDetailsElement;

  closeMobileDropdown(): void {
    this.mobileDropdown.open = false;
  }
}

MpFilterList.define('mp-filter-list');

// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { WebUIElement } from '@microsoft/webui-framework';

export class MpFilterList extends WebUIElement {
  mobileDropdown!: HTMLDetailsElement;

  closeMobileDropdown(): void {
    this.mobileDropdown.open = false;
  }
}

MpFilterList.define('mp-filter-list');

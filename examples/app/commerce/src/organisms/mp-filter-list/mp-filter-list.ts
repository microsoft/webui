// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { WebUIElement, observable } from '@microsoft/webui-framework';
import { Router } from '@microsoft/webui-router';

export class MpFilterList extends WebUIElement {
  @observable sortOptions: any[] = [];
  mobileDropdown!: HTMLDetailsElement;

  onClick(event: MouseEvent): void {
    const target = event.target;
    if (!(target instanceof Element)) {
      return;
    }

    const link = target.closest('a[href]');
    if (link) {
      const href = link.getAttribute('href');
      if (href) {
        event.preventDefault();
        Router.navigate(href);
      }
    }

    if (target.closest('.mobile-link, .mobile-current-item')) {
      this.closeMobileDropdown();
    }
  }

  private closeMobileDropdown(): void {
    if (this.mobileDropdown) {
      this.mobileDropdown.open = false;
    }
  }
}

MpFilterList.define('mp-filter-list');

// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { WebUIElement, attr, observable } from '@microsoft/webui-framework';
import { Router } from '@microsoft/webui-router';

export class MpCategoryNav extends WebUIElement {
  @attr({ attribute: 'all-active-class' }) allActiveClass = '';
  @attr({ attribute: 'current-label' }) currentCategoryLabel = 'All';
  @observable categories: any[] = [];
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
      if (link.classList.contains('mobile-link')) {
        this.closeMobileDropdown();
      }
      return;
    }

    if (target.closest('.mobile-link')) {
      this.closeMobileDropdown();
    }
  }

  private closeMobileDropdown(): void {
    if (this.mobileDropdown) {
      this.mobileDropdown.open = false;
    }
  }
}

MpCategoryNav.define('mp-category-nav');

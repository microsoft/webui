// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { WebUIElement, attr } from '@microsoft/webui-framework';
import { Router } from '@microsoft/webui-router';

export class MpSearchBar extends WebUIElement {
  @attr action = '/search';
  @attr query = '';
  @attr placeholder = 'Search for products...';
  @attr variant = 'desktop';
  @attr label = 'Search for products';

  onInput(event: Event): void {
    const target = event.target;
    if (target instanceof HTMLInputElement) {
      this.query = target.value;
    }
  }

  onSubmit(event: SubmitEvent): void {
    event.preventDefault();

    const url = new URL(this.action, window.location.origin);
    const query = this.query.trim();
    if (query) {
      url.searchParams.set('q', query);
    }

    Router.navigate(`${url.pathname}${url.search}`);
  }
}

MpSearchBar.define('mp-search-bar');

// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { WebUIElement, attr } from '@microsoft/webui-framework';

export class CbHeader extends WebUIElement {
  @attr mode: 'desktop' | 'web' = 'web';
  @attr searchQuery = '';

  onInput(e: Event): void {
    const input = e.currentTarget;
    if (!(input instanceof HTMLInputElement)) return;

    this.searchQuery = input.value;
    this.$emit('search-change', { value: input.value });
  }
}

CbHeader.define('cb-header');

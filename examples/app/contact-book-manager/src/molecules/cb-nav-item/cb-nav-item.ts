// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { WebUIElement, attr } from '@microsoft/webui-framework';

export class CbNavItem extends WebUIElement {
  @attr icon = '';
  @attr label = '';
  @attr count = '';
  @attr active = '';

  onClick(): void {
    this.$emit('nav-select', { label: this.label });
  }
}

CbNavItem.define('cb-nav-item');

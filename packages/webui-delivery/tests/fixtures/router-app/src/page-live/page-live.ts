// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { WebUIElement, observable } from '@microsoft/webui-framework';

export class PageLive extends WebUIElement {
  @observable view_model = { label: 'Connecting' };

  connectedCallback(): void {
    super.connectedCallback();
    this.view_model = { ...this.view_model, label: 'Live' };
  }
}

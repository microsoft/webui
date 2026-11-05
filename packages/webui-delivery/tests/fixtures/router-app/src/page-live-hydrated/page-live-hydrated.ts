// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { WebUIElement, observable } from '@microsoft/webui-framework';

export class PageLiveHydrated extends WebUIElement {
  @observable view_model = { label: 'Connecting' };

  hydratedCallback(): void {
    this.view_model = { ...this.view_model, label: 'Live' };
  }
}

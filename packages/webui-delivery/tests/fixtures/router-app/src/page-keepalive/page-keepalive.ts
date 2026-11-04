// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { WebUIElement, observable } from '@microsoft/webui-framework';

// Has local state (clickCount) that should survive keep-alive reactivation.
export class PageKeepAlive extends WebUIElement {
  @observable clickCount = 0;

  onIncrement = (): void => {
    this.clickCount++;
  };
}

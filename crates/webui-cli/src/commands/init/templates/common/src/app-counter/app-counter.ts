// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { WebUIElement, observable } from '@microsoft/webui-framework';

export class AppCounter extends WebUIElement {
  @observable count = 0;
  @observable ready = false;

  override hydratedCallback(): void {
    this.ready = true;
  }

  increment(): void {
    if (this.ready) this.count += 1;
  }
}

AppCounter.define('app-counter');

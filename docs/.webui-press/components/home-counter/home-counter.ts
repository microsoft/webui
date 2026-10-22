// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { WebUIElement, observable } from "@microsoft/webui-framework";

export class HomeCounter extends WebUIElement {
  @observable count = 0;
  @observable ready = false;

  protected hydratedCallback(): void {
    this.ready = true;
  }

  increment(): void {
    this.count += 1;
  }

  reset(): void {
    this.count = 0;
  }
}

HomeCounter.define("home-counter");

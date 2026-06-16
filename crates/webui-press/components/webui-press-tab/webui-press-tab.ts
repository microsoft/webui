// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { WebUIElement } from "@microsoft/webui-framework";

export class WebUIPressTab extends WebUIElement {
  select(): void {
    this.$emit("tab-select", { tab: this });
  }
}

WebUIPressTab.define("webui-press-tab");

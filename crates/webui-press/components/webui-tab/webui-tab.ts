// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { WebUIElement } from "@microsoft/webui-framework";

export class WebUITab extends WebUIElement {
  select(): void {
    this.$emit("tab-select", { tab: this });
  }
}

WebUITab.define("webui-tab");

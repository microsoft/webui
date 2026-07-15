// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import {
  WebUIElement,
  attr,
} from "@microsoft/webui-framework";

export class ExternalPanel extends WebUIElement {
  @attr label = "External panel";
}

ExternalPanel.define("external-panel");

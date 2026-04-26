// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { WebUIElement, attr } from "@microsoft/webui-framework";

export class WebUIBlockquote extends WebUIElement {
  @attr appearance: string = "info";
  @attr title: string = "";
  @attr icon: string = "ℹ️";
}

WebUIBlockquote.define("webui-blockquote");

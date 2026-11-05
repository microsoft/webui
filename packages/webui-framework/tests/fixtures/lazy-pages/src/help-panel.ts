// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { observable, WebUIElement } from '@microsoft/webui-framework';

class HelpPanel extends WebUIElement {
  @observable message = '';
  @observable showDeferred = false;
  textNode!: HTMLParagraphElement;

  override hydratedCallback(): void {
    if (new URLSearchParams(location.search).has('wait-style')) this.showDeferred = true;
  }
}
HelpPanel.define('help-panel');

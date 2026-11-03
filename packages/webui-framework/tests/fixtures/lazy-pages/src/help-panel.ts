// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { observable, WebUIElement } from '@microsoft/webui-framework';

class HelpPanel extends WebUIElement {
  @observable message = '';
  textNode!: HTMLParagraphElement;
}
HelpPanel.define('help-panel');

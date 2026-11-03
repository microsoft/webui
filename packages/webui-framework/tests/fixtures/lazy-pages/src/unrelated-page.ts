// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { observable, WebUIElement } from '@microsoft/webui-framework';

class UnrelatedPage extends WebUIElement {
  @observable open = false;
  show(): void { this.open = true; }
}
UnrelatedPage.define('unrelated-page');

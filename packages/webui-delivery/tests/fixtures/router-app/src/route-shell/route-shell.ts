// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { WebUIElement, observable } from '@microsoft/webui-framework';

export class RouteShell extends WebUIElement {
  @observable isHome = false;
  @observable isAlpha = false;
  @observable isBeta = false;
  @observable isItem1 = false;
  @observable isItem2 = false;
}

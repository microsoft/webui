// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { WebUIElement, attr } from '@microsoft/webui-framework';

export class PageCompose extends WebUIElement {
  @attr action = '';
  @attr to = '';
  @attr subject = '';
}

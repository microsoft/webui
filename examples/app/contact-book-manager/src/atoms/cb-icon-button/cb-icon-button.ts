// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { WebUIElement, attr } from '@microsoft/webui-framework';

export class CbIconButton extends WebUIElement {
  @attr icon = '';
  @attr title = '';
  @attr variant = 'default';
}

CbIconButton.define('cb-icon-button');

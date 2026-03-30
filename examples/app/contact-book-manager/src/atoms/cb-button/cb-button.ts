// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { WebUIElement, attr } from '@microsoft/webui-framework';

export class CbButton extends WebUIElement {
  @attr label = '';
  @attr variant = 'primary';
  @attr size = 'md';
}

CbButton.define('cb-button');

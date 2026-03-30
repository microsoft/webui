// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { WebUIElement, attr } from '@microsoft/webui-framework';

export class CbInput extends WebUIElement {
  @attr placeholder = '';
  @attr value = '';
  @attr type = 'text';
  @attr name = '';
}

CbInput.define('cb-input');

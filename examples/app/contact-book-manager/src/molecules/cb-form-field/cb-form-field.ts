// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { WebUIElement, attr } from '@microsoft/webui-framework';

export class CbFormField extends WebUIElement {
  @attr label = '';
  @attr name = '';
  @attr value = '';
  @attr placeholder = '';
  @attr type = 'text';
  @attr error = '';
}

CbFormField.define('cb-form-field');

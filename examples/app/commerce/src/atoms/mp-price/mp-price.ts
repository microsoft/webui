// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { WebUIElement, attr } from '@microsoft/webui-framework';

export class MpPrice extends WebUIElement {
  @attr value = '';
  @attr size = 'md';
  @attr variant = 'pill';
  @attr({ attribute: 'currency-code' }) currencyCode = 'USD';
}

MpPrice.define('mp-price');

// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { WebUIElement, attr } from '@microsoft/webui-framework';

export class CalcDisplay extends WebUIElement {
  @attr expression = '';
  @attr value = '';
  @attr error = '';
}

CalcDisplay.define('calc-display');

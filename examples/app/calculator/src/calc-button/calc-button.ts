// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { WebUIElement, attr } from '@microsoft/webui-framework';

export class CalcButton extends WebUIElement {
  @attr label = '';
  @attr value = '';
  @attr btnType = '';
  @attr btnSpan = '';

  onClick(): void {
    this.$emit('button-press', { value: this.value });
  }
}

CalcButton.define('calc-button');

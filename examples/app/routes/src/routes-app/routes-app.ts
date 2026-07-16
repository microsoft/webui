// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { WebUIElement } from '@microsoft/webui-framework';

export class RoutesApp extends WebUIElement {
  counterLabel!: HTMLSpanElement;
  onCounterClick(): void {
    this.counterLabel.textContent = String(Number(this.counterLabel.textContent) + 1);
  }
}
RoutesApp.define('routes-app');

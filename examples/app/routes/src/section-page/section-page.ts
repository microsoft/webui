// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { WebUIElement } from '@microsoft/webui-framework';

export class SectionPage extends WebUIElement {
  counterLabel!: HTMLSpanElement;
  onCounterClick(): void {
    this.counterLabel.textContent = String(Number(this.counterLabel.textContent) + 1);
  }
}
SectionPage.define('section-page');

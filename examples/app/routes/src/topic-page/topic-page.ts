// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { WebUIElement } from '@microsoft/webui-framework';

export class TopicPage extends WebUIElement {
  counterLabel!: HTMLSpanElement;
  onCounterClick(): void {
    this.counterLabel.textContent = String(Number(this.counterLabel.textContent) + 1);
  }
}
TopicPage.define('topic-page');

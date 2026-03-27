// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { WebUIElement, observable } from '@microsoft/webui-framework';

interface TopicLink {
  id: string;
  name: string;
}

export class SectionPage extends WebUIElement {
  @observable sectionId = '';
  @observable sectionName = '';
  @observable sectionIcon = '';
  @observable topicId = '';
  @observable topics: TopicLink[] = [];

  counterLabel!: HTMLSpanElement;
  onCounterClick(): void {
    this.counterLabel.textContent = String(Number(this.counterLabel.textContent) + 1);
  }
}
SectionPage.define('section-page');

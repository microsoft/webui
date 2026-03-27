// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { WebUIElement, observable } from '@microsoft/webui-framework';

interface LessonLink {
  id: string;
  name: string;
}

export class TopicPage extends WebUIElement {
  @observable sectionId = '';
  @observable sectionName = '';
  @observable topicId = '';
  @observable topicName = '';
  @observable lessonId = '';
  @observable lessons: LessonLink[] = [];

  counterLabel!: HTMLSpanElement;
  onCounterClick(): void {
    this.counterLabel.textContent = String(Number(this.counterLabel.textContent) + 1);
  }
}
TopicPage.define('topic-page');

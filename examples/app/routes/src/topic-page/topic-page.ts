// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { FASTElement, attr, observable } from '@microsoft/fast-element';
import { RenderableFASTElement } from '@microsoft/fast-html';

interface LessonLink {
  id: string;
  name: string;
}

export class TopicPage extends RenderableFASTElement(FASTElement) {
  @attr id = '';
  @attr({ attribute: 'section-name' }) sectionName = '';
  @attr({ attribute: 'topic-id' }) topicId = '';
  @attr({ attribute: 'topic-name' }) topicName = '';
  @observable lessons!: LessonLink[];

  async prepare(): Promise<void> {
    this.id = this.getAttribute('id') || '';
    this.sectionName = this.getAttribute('section-name') || '';
    this.topicId = this.getAttribute('topic-id') || '';
    this.topicName = this.getAttribute('topic-name') || '';

    const state = this.getAttribute('data-state');
    if (state) {
      const parsed = JSON.parse(state);
      if (Array.isArray(parsed.lessons)) {
        this.lessons = parsed.lessons;
      }
    }
  }

  setInitialState(state: Record<string, unknown>): void {
    if (state.id) this.id = state.id as string;
    if (state.sectionName) this.sectionName = state.sectionName as string;
    if (state.topicId) this.topicId = state.topicId as string;
    if (state.topicName) this.topicName = state.topicName as string;
    if (Array.isArray(state.lessons)) this.lessons = state.lessons as LessonLink[];
  }
}

TopicPage.defineAsync({
  name: 'topic-page',
  templateOptions: 'defer-and-hydrate',
});

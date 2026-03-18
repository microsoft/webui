// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { FASTElement, attr } from '@microsoft/fast-element';
import { RenderableFASTElement } from '@microsoft/fast-html';

export class LessonPage extends RenderableFASTElement(FASTElement) {
  @attr id = '';
  @attr({ attribute: 'section-name' }) sectionName = '';
  @attr({ attribute: 'topic-id' }) topicId = '';
  @attr({ attribute: 'topic-name' }) topicName = '';
  @attr({ attribute: 'lesson-id' }) lessonId = '';
  @attr({ attribute: 'lesson-name' }) lessonName = '';
  @attr({ attribute: 'lesson-content' }) lessonContent = '';

  async prepare(): Promise<void> {
    this.id = this.getAttribute('id') || '';
    this.sectionName = this.getAttribute('section-name') || '';
    this.topicId = this.getAttribute('topic-id') || '';
    this.topicName = this.getAttribute('topic-name') || '';
    this.lessonId = this.getAttribute('lesson-id') || '';
    this.lessonName = this.getAttribute('lesson-name') || '';
    this.lessonContent = this.getAttribute('lesson-content') || '';
  }

  setInitialState(state: Record<string, unknown>): void {
    if (state.id) this.id = state.id as string;
    if (state.sectionName) this.sectionName = state.sectionName as string;
    if (state.topicId) this.topicId = state.topicId as string;
    if (state.topicName) this.topicName = state.topicName as string;
    if (state.lessonId) this.lessonId = state.lessonId as string;
    if (state.lessonName) this.lessonName = state.lessonName as string;
    if (state.lessonContent) this.lessonContent = state.lessonContent as string;
  }
}

LessonPage.defineAsync({
  name: 'lesson-page',
  templateOptions: 'defer-and-hydrate',
});

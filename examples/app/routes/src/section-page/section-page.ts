// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { FASTElement, attr, observable } from '@microsoft/fast-element';
import { RenderableFASTElement } from '@microsoft/fast-html';

interface TopicLink {
  id: string;
  name: string;
}

export class SectionPage extends RenderableFASTElement(FASTElement) {
  @attr id = '';
  @attr({ attribute: 'section-name' }) sectionName = '';
  @attr({ attribute: 'section-icon' }) sectionIcon = '';
  @observable topics!: TopicLink[];

  async prepare(): Promise<void> {
    this.id = this.getAttribute('id') || '';
    this.sectionName = this.getAttribute('section-name') || '';
    this.sectionIcon = this.getAttribute('section-icon') || '';

    const state = this.getAttribute('data-state');
    if (state) {
      const parsed = JSON.parse(state);
      if (Array.isArray(parsed.topics)) {
        this.topics = parsed.topics;
      }
    }
  }

  setInitialState(state: Record<string, unknown>): void {
    if (state.id) this.id = state.id as string;
    if (state.sectionName) this.sectionName = state.sectionName as string;
    if (state.sectionIcon) this.sectionIcon = state.sectionIcon as string;
    if (Array.isArray(state.topics)) this.topics = state.topics as TopicLink[];
  }
}

SectionPage.defineAsync({
  name: 'section-page',
  templateOptions: 'defer-and-hydrate',
});

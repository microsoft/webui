// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { FASTElement, attr, observable } from '@microsoft/fast-element';
import { RenderableFASTElement } from '@microsoft/fast-html';

interface SectionLink {
  id: string;
  name: string;
  icon: string;
}

export class RoutesApp extends RenderableFASTElement(FASTElement) {
  @attr({ attribute: 'app-title' }) appTitle = 'Learning Platform';
  @observable sections!: SectionLink[];

  async prepare(): Promise<void> {
    this.appTitle = this.getAttribute('app-title') || this.appTitle;

    const state = this.getAttribute('data-state');
    if (state) {
      const parsed = JSON.parse(state);
      if (Array.isArray(parsed.sections)) {
        this.sections = parsed.sections;
      }
    }
  }

  setInitialState(state: Record<string, unknown>): void {
    if (state.title) this.appTitle = state.title as string;
    if (Array.isArray(state.sections)) this.sections = state.sections as SectionLink[];
  }
}

RoutesApp.defineAsync({
  name: 'routes-app',
  templateOptions: 'defer-and-hydrate',
});

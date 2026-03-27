// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { WebUIElement, observable } from '@microsoft/webui-framework';

interface SectionLink {
  id: string;
  name: string;
  icon: string;
}

export class RoutesApp extends WebUIElement {
  @observable appTitle = 'Learning Platform';
  @observable sectionId = '';
  @observable sections: SectionLink[] = [];

  counterLabel!: HTMLSpanElement;
  onCounterClick(): void {
    this.counterLabel.textContent = String(Number(this.counterLabel.textContent) + 1);
  }

}
RoutesApp.define('routes-app');

// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { observable, WebUIElement } from '@microsoft/webui-framework';
import { collectTemplates } from '@microsoft/webui-framework/template-cache.js';
import type { ViewPreferences } from '../lazy-dialog/lazy-dialog.js';

export class LazyWorkspace extends WebUIElement {
  @observable compactMode = false;
  @observable highlightPinned = true;
  @observable slowData = false;
  @observable announcement = '';

  changeDelay(event: Event): void {
    const input = event.currentTarget;
    if (input instanceof HTMLInputElement) this.slowData = input.checked;
  }

  applyPreferences(event: CustomEvent<ViewPreferences>): void {
    this.compactMode = event.detail.compactMode;
    this.highlightPinned = event.detail.highlightPinned;
    this.announcement = 'View preferences applied to this page.';
  }

  clearTemplateCache(): void {
    collectTemplates();
    this.announcement = 'Template cache cleared. Live views keep the resources they still need.';
  }
}

LazyWorkspace.define('lazy-workspace');

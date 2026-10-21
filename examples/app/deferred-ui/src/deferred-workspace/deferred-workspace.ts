// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { observable, WebUIElement } from '@microsoft/webui-framework';
import { collectTemplates } from '@microsoft/webui-framework/template-cache.js';
import type { ViewPreferences } from '../deferred-dialog/deferred-dialog.js';

export class DeferredWorkspace extends WebUIElement {
  @observable compactMode = false;
  @observable highlightPinned = true;
  @observable slowData = false;
  @observable announcement = '';

  onOverlayNotice(event: CustomEvent<string>): void {
    this.announcement = event.detail;
  }

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

DeferredWorkspace.define('deferred-workspace');

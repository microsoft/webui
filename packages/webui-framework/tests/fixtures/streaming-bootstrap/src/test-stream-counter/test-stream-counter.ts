// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { attr, observable, WebUIElement } from '@microsoft/webui-framework';

export class TestStreamCounter extends WebUIElement {
  @attr label = '';
  @observable count = 0;
  hydratedCalls = 0;

  increment(): void {
    this.count++;
  }

  hydratedCallback(): void {
    this.hydratedCalls++;
    (window.__streamingActivationOrder ??= []).push(`${this.localName}:${this.label}`);
  }
}

TestStreamCounter.define('test-stream-counter');

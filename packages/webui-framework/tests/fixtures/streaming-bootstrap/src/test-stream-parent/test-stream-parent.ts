// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { observable, WebUIElement } from '@microsoft/webui-framework';

export class TestStreamParent extends WebUIElement {
  @observable count = 0;
  hydratedCalls = 0;

  hydratedCallback(): void {
    this.hydratedCalls++;
    (window.__streamingActivationOrder ??= []).push(this.localName);
  }
}

TestStreamParent.define('test-stream-parent');

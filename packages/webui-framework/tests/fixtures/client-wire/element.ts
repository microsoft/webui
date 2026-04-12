// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

/**
 * Regression fixture: client-created components (no SSR) must render
 * their initial observable values immediately after connectedCallback.
 *
 * Without the $updateInstance call after $wire, client-created
 * components would show empty/default values until the first
 * reactive change triggers an update.
 */

import { WebUIElement, observable } from '../../../src/index.js';
import {
  bindText,
  dynamic,
  nodePath,
  registerCompiledTemplate,
  slot,
} from '@microsoft/webui-test-support';

registerCompiledTemplate('test-client-wire', {
  h: '<span class="greeting"></span><span class="count"></span>',
  sd: true,
  text: [
    bindText(slot({ parent: nodePath(0), before: 0 }), dynamic('greeting')),
    bindText(slot({ parent: nodePath(1), before: 0 }), dynamic('count')),
  ],
});

export class TestClientWire extends WebUIElement {
  @observable greeting = 'Hello';
  @observable count = 42;
}

TestClientWire.define('test-client-wire');

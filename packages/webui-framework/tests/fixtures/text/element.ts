// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { WebUIElement, attr, observable } from '../../../src/index.js';
import {
  bindText,
  dynamic,
  nodePath,
  registerCompiledTemplate,
  slot,
} from '@microsoft/webui-test-support';

registerCompiledTemplate('test-text', {
  h: '<span class="greeting"></span>, <span class="name"></span>!',
  text: [
    bindText(slot({ parent: nodePath(0), before: 0 }), dynamic('greeting')),
    bindText(slot({ parent: nodePath(2), before: 0 }), dynamic('name')),
  ],
});

export class TestText extends WebUIElement {
  @attr greeting = 'Hello';
  @observable name = 'World';
}

TestText.define('test-text');


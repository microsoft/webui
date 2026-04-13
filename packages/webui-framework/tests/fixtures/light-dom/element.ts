// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

/**
 * Dedicated light-DOM hydration fixture.
 *
 * The pipeline always produces shadow DOM, so this fixture uses manual
 * template registration and hand-written SSR HTML to keep the light-DOM
 * hydration code path tested.
 */

import { WebUIElement, observable } from '../../../src/index.js';
import { registerCompiledTemplate } from '@microsoft/webui-test-support';

registerCompiledTemplate('test-light-dom', {
  h: '<span class="greeting"></span> <span class="name"></span>!',
  tx: [
    [[[0], 0], [['greeting']]],
    [[[2], 0], [['name']]],
  ],
});

export class TestLightDom extends WebUIElement {
  @observable greeting = 'Hello';
  @observable name = 'World';
}

TestLightDom.define('test-light-dom');

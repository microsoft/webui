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
    // Pre-order element indices: 1 is .greeting, 2 is .name.
    [[1, 0], [['greeting']]],
    [[2, 0], [['name']]],
  ],
});

registerCompiledTemplate('test-light-dom-comment', {
  h: '<!--authored--><span class="tail">tail</span>',
  tx: [
    [[0, 0], [['commentText']]],
  ],
});

export class TestLightDom extends WebUIElement {
  @observable greeting = 'Hello';
  @observable name = 'World';
}

export class TestLightDomComment extends WebUIElement {
  @observable commentText = 'server';
}

TestLightDom.define('test-light-dom');
TestLightDomComment.define('test-light-dom-comment');

// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { WebUIElement, attr, observable } from '../../../src/index.js';

/**
 * Child component used inside a <for> loop.
 * Receives `htmlContent` via @attr — tests whether {{{htmlContent}}}
 * renders raw HTML after a reactive update (not just SSR).
 */
export class TestRawItem extends WebUIElement {
  @attr name = '';
  @attr htmlContent = '';
}
TestRawItem.define('test-raw-item');

/**
 * Parent component with a <for> loop of test-raw-item elements.
 * Reactively updating `items` should re-render child @attr values
 * and {{{htmlContent}}} should render as raw HTML, not escaped.
 */
export class TestRawFor extends WebUIElement {
  @observable items: Array<{ name: string; htmlContent: string }> = [];
}
TestRawFor.define('test-raw-for');

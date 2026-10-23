// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { startRuntime } from '../../../src/runtime.js';
import '../../../src/styles-entry.js';
import { WebUIElement, observable } from '../../../src/index.js';

startRuntime();

export class TestRawHtmlLight extends WebUIElement {
  @observable rawHtml = '';
  @observable rowsHtml = '';
  @observable showConditional = true;
}

TestRawHtmlLight.define('test-raw-html-light');

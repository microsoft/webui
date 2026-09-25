// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { WebUIElement, observable } from '../../../src/index.js';

export class TestRawHtmlLight extends WebUIElement {
  @observable rawHtml = '';
  @observable rowsHtml = '';
  @observable showConditional = true;
}

TestRawHtmlLight.define('test-raw-html-light');

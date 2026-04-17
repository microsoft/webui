// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { WebUIElement, observable } from '../../../src/index.js';

export class TestRawHtml extends WebUIElement {
  @observable expanded = true;
  @observable name = '';
  @observable rawHtml = '';
}

TestRawHtml.define('test-raw-html');

// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { WebUIElement, observable } from '../../../src/index.js';

export class TestLightDomStructural extends WebUIElement {
  @observable show = true;
  @observable conditionalText = 'client';
  @observable items = ['X'];
}

TestLightDomStructural.define('test-light-dom-structural');

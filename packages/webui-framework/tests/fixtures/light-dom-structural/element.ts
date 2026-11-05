// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { startRuntime } from '../../../src/runtime.js';
import '../../../src/styles-entry.js';
import { WebUIElement, observable } from '../../../src/index.js';

startRuntime();

export class TestLightDomStructural extends WebUIElement {
  @observable show = true;
  @observable conditionalText = 'client';
  @observable items = ['X'];
}

TestLightDomStructural.define('test-light-dom-structural');

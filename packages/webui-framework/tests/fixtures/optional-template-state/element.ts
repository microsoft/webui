// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { WebUIElement, observable } from '../../../src/index.js';

export class TestOptionalState extends WebUIElement {
  @observable selected = 'off';

  toggle(): void {
    this.selected = this.selected === 'off' ? 'on' : 'off';
  }
}

TestOptionalState.define('test-optional-state');

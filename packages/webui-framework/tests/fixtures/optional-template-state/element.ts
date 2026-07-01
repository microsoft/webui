// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { WebUIElement, observable } from '../../../src/index.js';

/**
 * Test component for optional template state.
 *
 * `selected` is JS-owned interactive state because the click handler mutates it.
 * Other template values in the fixture are intentionally omitted from the class
 * to prove `@observable` / `@attr` are not required for template-only bindings.
 */
export class TestOptionalState extends WebUIElement {
  @observable selected = 'off';

  toggle(): void {
    this.selected = this.selected === 'off' ? 'on' : 'off';
  }
}

TestOptionalState.define('test-optional-state');

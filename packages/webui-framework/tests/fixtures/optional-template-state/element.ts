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

/**
 * Child component with an authored setter but no decorator.
 *
 * The parent binds a complex property into this setter. That must fall back to
 * direct assignment when the child template does not read the property itself,
 * otherwise authored non-observable APIs silently miss parent data.
 */
export class TestNonobservableChild extends WebUIElement {
  set payload(value: { label?: string }) {
    this.setAttribute('data-payload-label', value.label ?? '');
  }
}

TestNonobservableChild.define('test-nonobservable-child');

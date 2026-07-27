// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

/**
 * Fixtures for issue #379: hydration must surface (not silently swallow) an
 * observable whose value, set at or before `super.connectedCallback()`,
 * disagrees with the server-rendered DOM.
 *
 * Each component renders a conditional (`<if condition="show">`) and a
 * template attribute (`data-value="{{value}}"`). The SSR state omits `show`
 * and `value`, so the server renders the conditional empty and the attribute
 * blank. The components differ only in WHEN they assign the observables.
 */

import { WebUIElement, observable } from '../../../src/index.js';

/**
 * `@observable` field default differs from SSR state. The initializer runs
 * during construction (before hydration), so the value is dropped and the DOM
 * disagrees. Expect a hydration-mismatch warning.
 */
export class MismatchFieldDefault extends WebUIElement {
  @observable show = true;
  @observable value = '3';
}
MismatchFieldDefault.define('mismatch-field-default');

/** Assigns in the constructor — before hydration. Expect a warning. */
export class MismatchConstructor extends WebUIElement {
  @observable show = false;
  @observable value = '';

  constructor() {
    super();
    this.show = true;
    this.value = '3';
  }
}
MismatchConstructor.define('mismatch-constructor');

/** Assigns before `super.connectedCallback()`. Expect a warning. */
export class MismatchBeforeSuper extends WebUIElement {
  @observable show = false;
  @observable value = '';

  connectedCallback(): void {
    this.show = true;
    this.value = '3';
    super.connectedCallback();
  }
}
MismatchBeforeSuper.define('mismatch-before-super');

/**
 * Assigns after `super.connectedCallback()`. No warning; the DOM updates.
 * Also covers synchronous hydration during parsing for issue #393.
 */
export class MismatchAfterSuper extends WebUIElement {
  @observable show = false;
  @observable value = '';
  box!: HTMLDivElement;
  readyStateAtConnect = '';
  referencesReadyAfterSuper = false;

  connectedCallback(): void {
    this.readyStateAtConnect = document.readyState;
    super.connectedCallback();
    this.referencesReadyAfterSuper = this.box instanceof HTMLDivElement;
    this.show = true;
    this.value = '3';
  }
}
MismatchAfterSuper.define('mismatch-after-super');

/** Assigns in a deferred task. No warning; the DOM updates. */
export class MismatchDeferred extends WebUIElement {
  @observable show = false;
  @observable value = '';

  connectedCallback(): void {
    super.connectedCallback();
    setTimeout(() => {
      this.show = true;
      this.value = '3';
    }, 0);
  }
}
MismatchDeferred.define('mismatch-deferred');

/**
 * `@observable` field defaults that ARE seeded into SSR state (see state.json),
 * so the server already rendered them. A pre-ready write that matches the
 * server must NOT warn.
 */
export class MismatchSeeded extends WebUIElement {
  @observable seededShow = true;
  @observable seededValue = '3';
}
MismatchSeeded.define('mismatch-seeded');

// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { WebUIElement, observable } from '../../../src/index.js';

// Child component — has its own event binding.
export class TestNestedChild extends WebUIElement {
  @observable childClicks = 0;

  onChildClick(): void {
    this.childClicks += 1;
  }
}

TestNestedChild.define('test-nested-child');

// Parent component — has its own event binding AND contains test-nested-child.
// Verifies that event wiring during SSR hydration correctly scopes to
// the parent's own event targets without interfering with the child.
export class TestNestedEvent extends WebUIElement {
  @observable parentClicks = 0;

  onParentClick(): void {
    this.parentClicks += 1;
  }
}

TestNestedEvent.define('test-nested-event');

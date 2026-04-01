// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { WebUIElement, observable } from '../../../src/index.js';
import {
  bindEvent,
  bindText,
  dynamic,
  nodePath,
  registerCompiledTemplate,
  slot,
} from '@microsoft/webui-test-support';

// Child component — has its own event binding.
registerCompiledTemplate('test-nested-child', {
  h: '<button class="child-btn">Child</button>',
  events: [
    bindEvent('click', 'onChildClick'),
  ],
  eventTargets: [nodePath(0)],
});

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
registerCompiledTemplate('test-nested-event', {
  h: '<span class="count"></span><button class="parent-btn">Parent</button><test-nested-child></test-nested-child>',
  text: [
    bindText(slot({ parent: nodePath(0), before: 0 }), dynamic('parentClicks')),
  ],
  events: [
    bindEvent('click', 'onParentClick'),
  ],
  eventTargets: [nodePath(1)],
});

export class TestNestedEvent extends WebUIElement {
  @observable parentClicks = 0;

  onParentClick(): void {
    this.parentClicks += 1;
  }
}

TestNestedEvent.define('test-nested-event');

// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

/**
 * Regression fixture: custom elements inside a false <if> block must mount
 * correctly when the condition flips true client-side.
 *
 * Scenario 1 — flat: parent has <if condition="show"><child-comp></if>,
 *   SSR renders with show=false, child-comp not initially in the DOM.
 *
 * Scenario 2 — nested: parent <if> → mid component → mid's <if> → grandchild.
 *   Both mid and grandchild are initially hidden.
 */

import { WebUIElement, observable } from '../../../src/index.js';

export class TestChildComp extends WebUIElement {
  @observable label = 'Child Active';
}
TestChildComp.define('test-child-comp');

export class TestGrandchildComp extends WebUIElement {
  @observable message = 'Grandchild Active';
}
TestGrandchildComp.define('test-grandchild-comp');

export class TestMidComp extends WebUIElement {
  @observable inner = true;
}
TestMidComp.define('test-mid-comp');

export class TestCondParent extends WebUIElement {
  @observable show = false;

  toggleShow(): void {
    this.show = !this.show;
  }
}
TestCondParent.define('test-cond-parent');

export class TestNestedCondParent extends WebUIElement {
  @observable show = false;

  toggleShow(): void {
    this.show = !this.show;
  }
}
TestNestedCondParent.define('test-nested-cond-parent');


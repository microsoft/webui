// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

/**
 * Regression fixture: custom elements inside a false <if> block must mount
 * correctly when the condition flips true client-side.
 *
 * This fixture simulates the real SSR scenario where templates for components
 * inside false <if> blocks are NOT emitted by the server. The child template
 * is registered AFTER the parent hydrates, simulating late template delivery.
 *
 * Scenario 1 — flat: parent has <if condition="show"><child-comp></if>,
 *   SSR renders with show=false, child-comp template not initially available.
 *
 * Scenario 2 — nested: parent <if> → mid component → mid's <if> → grandchild.
 *   Both mid and grandchild templates are initially missing.
 */

import { WebUIElement, observable } from '../../../src/index.js';
import {
  bindEvent,
  bindText,
  dynamic,
  identifier,
  nodePath,
  registerCompiledTemplate,
  slot,
  when,
} from '@microsoft/webui-test-support';

// ── Child component: simple text display ──────────────────────
// Template registered but exposed for late-registration test
const childTemplate = {
  h: '<span class="child-text"></span>',
  shadowDom: true as const,
  text: [
    bindText(slot({ parent: nodePath(0), before: 0 }), dynamic('label')),
  ],
};

export class TestChildComp extends WebUIElement {
  @observable label = 'Child Active';
}
TestChildComp.define('test-child-comp');

// ── Grandchild component: for nested test ─────────────────────
const grandchildTemplate = {
  h: '<span class="grandchild-text"></span>',
  shadowDom: true as const,
  text: [
    bindText(slot({ parent: nodePath(0), before: 0 }), dynamic('message')),
  ],
};

export class TestGrandchildComp extends WebUIElement {
  @observable message = 'Grandchild Active';
}
TestGrandchildComp.define('test-grandchild-comp');

// ── Mid component: has inner <if> with grandchild ─────────────
const midTemplate = {
  h: '<span class="mid-label">Mid</span>',
  shadowDom: true as const,
  conditionals: [when(identifier('inner'), { blockIndex: 0, slot: { before: 1 } })],
  blocks: [{
    h: '<test-grandchild-comp></test-grandchild-comp>',
  }],
};

export class TestMidComp extends WebUIElement {
  @observable inner = true;
}
TestMidComp.define('test-mid-comp');

// ── Parent component: has <if> wrapping the child ─────────────
registerCompiledTemplate('test-cond-parent', {
  h: '<button class="toggle">Toggle</button>',
  shadowDom: true,
  conditionals: [when(identifier('show'), { blockIndex: 0, slot: { before: 1 } })],
  blocks: [{
    h: '<test-child-comp></test-child-comp>',
  }],
  events: [bindEvent('click', 'toggleShow', false, nodePath(0))],
});

export class TestCondParent extends WebUIElement {
  @observable show = false;

  toggleShow(): void {
    this.show = !this.show;
  }
}
TestCondParent.define('test-cond-parent');

// ── Nested parent ─────────────────────────────────────────────
registerCompiledTemplate('test-nested-cond-parent', {
  h: '<button class="toggle">Toggle</button>',
  shadowDom: true,
  conditionals: [when(identifier('show'), { blockIndex: 0, slot: { before: 1 } })],
  blocks: [{
    h: '<test-mid-comp></test-mid-comp>',
  }],
  events: [bindEvent('click', 'toggleShow', false, nodePath(0))],
});

export class TestNestedCondParent extends WebUIElement {
  @observable show = false;

  toggleShow(): void {
    this.show = !this.show;
  }
}
TestNestedCondParent.define('test-nested-cond-parent');

// ── Expose template registration for late-registration tests ──
// In the real app, these would come from SSR <script> IIFEs.
// We DON'T register child/mid/grandchild templates upfront —
// the test will register them to simulate the fixed SSR behavior.
(window as any).__fixture_register_child = () => {
  registerCompiledTemplate('test-child-comp', childTemplate);
};
(window as any).__fixture_register_mid = () => {
  registerCompiledTemplate('test-mid-comp', midTemplate);
};
(window as any).__fixture_register_grandchild = () => {
  registerCompiledTemplate('test-grandchild-comp', grandchildTemplate);
};
// Also register all upfront for the "with templates" test group
(window as any).__fixture_register_all = () => {
  registerCompiledTemplate('test-child-comp', childTemplate);
  registerCompiledTemplate('test-mid-comp', midTemplate);
  registerCompiledTemplate('test-grandchild-comp', grandchildTemplate);
};


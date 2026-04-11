// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

/**
 * Regression fixture for shadow-DOM components that receive pre-existing
 * slot content (the SPA partial rendering scenario).
 *
 * When the server sends a partial response during SPA navigation, child
 * components arrive with slot content already present as child nodes:
 *
 *   <test-slot-btn><svg>…</svg><span>Reply</span></test-slot-btn>
 *
 * The framework must still create a shadow root and populate it from the
 * compiled template — the existing children are slot content projected
 * through <slot>, NOT SSR light-DOM output.
 */

import { WebUIElement, attr } from '../../../src/index.js';
import { registerCompiledTemplate } from '@microsoft/webui-test-support';

// A simple shadow-DOM button component with a <slot> for projected content.
registerCompiledTemplate('test-slot-btn', {
  h: '<button class="btn"><slot></slot></button>',
  sd: 1,
});

// A parent component that programmatically creates a child with slot content,
// mimicking what the SPA router does when injecting a server partial.
registerCompiledTemplate('test-slot-parent', {
  h: '<div class="container"></div>',
  sd: 1,
});

export class TestSlotBtn extends WebUIElement {
  @attr appearance = '';
}

export class TestSlotParent extends WebUIElement {
  /**
   * Simulate SPA partial injection: create a child component element,
   * give it slot content, then append to DOM (triggering connectedCallback).
   */
  spawnSlotChild(): void {
    const root = this.shadowRoot ?? this;
    const container = root.querySelector('.container');
    if (!container) return;

    const btn = document.createElement('test-slot-btn');
    btn.setAttribute('appearance', 'primary');

    // Add slot content BEFORE appending to DOM — this is what the browser
    // does when parsing innerHTML from a server partial response.
    const icon = document.createElement('span');
    icon.className = 'icon';
    icon.textContent = '↩';
    const label = document.createElement('span');
    label.textContent = 'Reply';
    btn.appendChild(icon);
    btn.appendChild(label);

    container.appendChild(btn);
  }
}

TestSlotBtn.define('test-slot-btn');
TestSlotParent.define('test-slot-parent');

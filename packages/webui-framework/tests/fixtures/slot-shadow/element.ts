// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { WebUIElement, attr } from '../../../src/index.js';

export class TestSlotBtn extends WebUIElement {
  @attr appearance = '';
}

export class TestSlotParent extends WebUIElement {
  spawnSlotChild(): void {
    const root = this.shadowRoot ?? this;
    const container = root.querySelector('.container');
    if (!container) return;

    const btn = document.createElement('test-slot-btn');
    btn.setAttribute('appearance', 'primary');

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

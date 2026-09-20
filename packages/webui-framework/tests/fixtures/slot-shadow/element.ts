// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { WebUIElement, attr, observable } from '../../../src/index.js';

export class TestSlotBtn extends WebUIElement {
  @attr appearance = '';
}

export class TestSlotParent extends WebUIElement {
  @observable previews: Array<{
    preview_url: string;
    can_view_logs: boolean;
  }> = [];

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

class MaiMenu extends HTMLElement {
  connectedCallback(): void {
    if (this.shadowRoot) return;
    const root = this.attachShadow({ mode: 'open' });
    root.innerHTML = '<slot name="trigger"></slot><slot></slot>';
  }
}

class MaiMenuList extends HTMLElement {
  connectedCallback(): void {
    if (this.shadowRoot) return;
    const root = this.attachShadow({ mode: 'open' });
    root.innerHTML = '<div role="menu"><slot></slot></div>';
  }
}

class MaiMenuItem extends HTMLElement {
  connectedCallback(): void {
    if (this.shadowRoot) return;
    const root = this.attachShadow({ mode: 'open' });
    root.innerHTML = '<div role="menuitem"><slot></slot></div>';
  }
}

customElements.define('mai-menu', MaiMenu);
customElements.define('mai-button', class extends HTMLElement {});
customElements.define('mai-menu-list', MaiMenuList);
customElements.define('mai-menu-item', MaiMenuItem);
TestSlotBtn.define('test-slot-btn');
TestSlotParent.define('test-slot-parent');

// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { WebUIElement, observable } from '../../../src/index.js';

/**
 * A host-interactive component: the host element *is* the control. The shadow
 * tree holds only a `pointer-events: none` indicator, so real clicks and
 * keystrokes target the host and never enter the shadow tree.
 */
export class TestHostToggle extends WebUIElement {
  @observable checked = false;
  @observable activations = 0;
  @observable label = 'off';

  lastTarget = '';

  private readonly internals = this.attachInternals();

  override connectedCallback(): void {
    super.connectedCallback();
    if (!this.hasAttribute('tabindex')) this.setAttribute('tabindex', '0');
    this.internals.role = 'checkbox';
    this.internals.ariaChecked = String(this.checked);
  }

  onActivate(e: MouseEvent): void {
    this.record(e);
  }

  onKeydown(e: KeyboardEvent): void {
    if (e.key !== 'Enter' && e.key !== ' ') return;
    e.preventDefault();
    this.record(e);
  }

  private record(e: Event): void {
    this.lastTarget = (e.target as HTMLElement)?.tagName ?? '';
    this.checked = !this.checked;
    this.activations += 1;
    this.label = this.checked ? 'on' : 'off';
    this.internals.ariaChecked = String(this.checked);
  }
}

TestHostToggle.define('test-host-toggle');

// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { WebUIElement, observable, attr } from '../../../src/index.js';

// Grandchild — emits a composed custom event.
export class TestChild extends WebUIElement {
  @attr itemId = '';

  onSelect(): void {
    this.$emit('item-selected', { id: this.itemId });
  }
}
TestChild.define('test-child');

// Intermediary — just wraps children, no event handling.
export class TestParent extends WebUIElement {}
TestParent.define('test-parent');

// Grandparent — listens for composed event via @item-selected on <template>.
export class TestGrandparent extends WebUIElement {
  @observable selectedItem = 'none';

  onItemSelected(e: CustomEvent<{ id: string }>): void {
    this.selectedItem = e.detail.id;
  }
}
TestGrandparent.define('test-grandparent');

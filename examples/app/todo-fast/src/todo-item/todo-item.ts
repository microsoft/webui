// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { FASTElement, attr } from '@microsoft/fast-element';
import { declarativeTemplate } from '@microsoft/fast-element/declarative.js';
import { observerMap } from '@microsoft/fast-element/observer-map.js';

export class TodoItem extends FASTElement {
  @attr id = '';
  @attr title = '';
  @attr state = '';

  onClick(e: MouseEvent): void {
    const target = e.composedPath()[0] as HTMLElement;
    const action = target.closest('[data-action]')?.getAttribute('data-action');
    if (!action) return;

    if (action === 'toggle') {
      this.$emit('toggle-item', { id: this.id });
    } else if (action === 'delete') {
      this.$emit('delete-item', { id: this.id });
    }
  }
}

void TodoItem.define({
  name: 'todo-item',
  template: declarativeTemplate(),
}, [observerMap()]);

// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { FASTElement, attr } from '@microsoft/fast-element';
import { RenderableFASTElement } from '@microsoft/fast-html';

export class CbHeader extends RenderableFASTElement(FASTElement) {
  @attr({ attribute: 'search-query' }) searchQuery = '';

  private listenersAttached!: boolean;

  connectedCallback(): void {
    super.connectedCallback();
    if (this.listenersAttached) return;
    this.listenersAttached = true;
    this.addEventListener('click', (e: Event) => {
      this.onClick(e as MouseEvent);
    });
    this.addEventListener('input', (e: Event) => {
      this.onInput(e);
    });
  }

  private emit(type: string, detail?: unknown): void {
    this.dispatchEvent(new CustomEvent(type, { bubbles: true, composed: true, detail }));
  }

  onInput(e: Event): void {
    const input = e.composedPath().find(el => (el as HTMLElement).tagName === 'INPUT') as HTMLInputElement;
    if (input) {
      this.searchQuery = input.value;
      this.emit('search', { value: input.value });
    }
  }

  onClick(e: MouseEvent): void {
    const target = e.composedPath()[0] as HTMLElement;
    const action = target.closest('[data-action]')?.getAttribute('data-action');
    if (action === 'add-contact') {
      this.emit('add-contact');
    }
  }
}

CbHeader.defineAsync({
  name: 'cb-header',
  templateOptions: 'defer-and-hydrate',
});

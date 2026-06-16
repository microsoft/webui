// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { FASTElement, attr, observable } from '@microsoft/fast-element';
import { declarativeTemplate } from '@microsoft/fast-element/declarative.js';
import { observerMap } from '@microsoft/fast-element/observer-map.js';

interface TodoItemData {
  id: string;
  title: string;
  state: string;
}

export class TodoApp extends FASTElement {
  @attr title = '';
  @observable items: TodoItemData[] = [];
  @observable remainingCount = 0;

  addInput!: HTMLInputElement;

  private prepared = false;

  private nextId = 100;

  connectedCallback(): void {
    this.prepareOnce();
    super.connectedCallback();
    void this.$fastController.isPrerendered.then(() => {
      this.prepareOnce();
    });
    console.log('TodoApp connected');
  }

  disconnectedCallback(): void {
    super.disconnectedCallback();
    console.log('TodoApp disconnected');
  }

  private prepareOnce(): void {
    if (this.prepared) return;
    this.prepareFromDom();
  }

  private prepareFromDom(): boolean {
    const root = this.shadowRoot;
    if (!root) return false;

    const items: TodoItemData[] = [];
    for (const el of root.querySelectorAll('todo-item')) {
      items.push({
        id: el.getAttribute('id') || '',
        title: el.getAttribute('title') || '',
        state: el.getAttribute('state') || 'pending',
      });
    }
    this.setItems(items);
    return true;
  }

  private setItems(items: TodoItemData[]): void {
    this.items = items;
    if (items.length > 0) {
      let maxId = 0;
      for (const item of items) {
        maxId = Math.max(maxId, Number(item.id) || 0);
      }
      this.nextId = maxId + 1;
    }
    this.updateCount();
    this.prepared = true;
  }

  onToggleItem(e: CustomEvent<{id: string}>): void {
    this.items = this.items.map(item => item.id === e.detail.id
      ? {
          id: item.id,
          title: item.title,
          state: item.state === 'done' ? 'pending' : 'done',
        }
      : item);
    this.updateCount();
  }

  onDeleteItem(e: CustomEvent<{id: string}>): void {
    this.items = this.items.filter(item => item.id !== e.detail.id);
    this.updateCount();
  }

  onAddKeydown(e: KeyboardEvent): boolean {
    if (e.key === 'Enter') {
      this.addTodo();
    }
    return true;
  }

  onAddClick(): void {
    this.addTodo();
  }

  private addTodo(): void {
    const input = this.addInput;
    if (!input) return;

    const text = input.value.trim();
    if (!text) return;

    this.items = [
      ...this.items,
      { id: String(this.nextId++), title: text, state: 'pending' },
    ];
    this.updateCount();
    input.value = '';
    input.focus();
  }

  private updateCount(): void {
    this.remainingCount = this.items.filter(i => i.state !== 'done').length;
  }
}

void TodoApp.define({
  name: 'todo-app',
  template: declarativeTemplate(),
}, [observerMap()]);

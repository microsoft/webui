// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { FASTElement, attr, observable } from '@microsoft/fast-element';
import { attributeMap } from '@microsoft/fast-element/attribute-map.js';
import { declarativeTemplate } from '@microsoft/fast-element/declarative.js';
import { observerMap } from '@microsoft/fast-element/observer-map.js';

interface TodoItemData {
  id: string;
  title: string;
  state: string;
}

export class TodoApp extends FASTElement {
  @attr title = '';
  @observable items!: TodoItemData[];
  @observable remainingCount!: number;

  addInput!: HTMLInputElement;

  private nextId = 100;

  connectedCallback(): void {
    this.prepareFromDom();
    super.connectedCallback();
    void this.$fastController.isPrerendered.then(() => {
      this.prepareFromDom();
    });
    console.log('TodoApp connected');
  }

  disconnectedCallback(): void {
    super.disconnectedCallback();
    console.log('TodoApp disconnected');
  }

  private prepareFromDom(): void {
    const root = this.shadowRoot;
    if (!root) return;

    const items: TodoItemData[] = [];
    for (const el of root.querySelectorAll('todo-item')) {
      items.push({
        id: el.getAttribute('id') || '',
        title: el.getAttribute('title') || '',
        state: el.getAttribute('state') || 'pending',
      });
    }
    this.items = items;
    if (items.length > 0) {
      this.nextId = Math.max(...items.map(i => Number(i.id) || 0)) + 1;
    }
    this.updateCount();
  }

  onToggleItem(e: CustomEvent<{id: string}>): void {
    const item = this.items.find(i => i.id === e.detail.id);
    if (item) {
      item.state = item.state === 'done' ? 'pending' : 'done';
    }
    this.updateCount();
  }

  onDeleteItem(e: CustomEvent<{id: string}>): void {
    this.items = this.items.filter(item => item.id !== e.detail.id);
    this.updateCount();
  }

  onAddKeydown(e: KeyboardEvent) {
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
}, [attributeMap(), observerMap()]);

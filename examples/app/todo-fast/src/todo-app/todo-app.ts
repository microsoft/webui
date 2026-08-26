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

  private nextId = 100;
  private hasInitialState = false;

  connectedCallback(): void {
    if (!this.hasInitialState) {
      this.items = [
        { id: '1', title: 'Buy groceries', state: 'done' },
        { id: '2', title: 'Write documentation', state: 'pending' },
        { id: '3', title: 'Ship feature', state: 'pending' },
      ];
      this.remainingCount = 2;
      this.hasInitialState = true;
    }
    super.connectedCallback();
  }

  disconnectedCallback(): void {
    super.disconnectedCallback();
    console.log('TodoApp disconnected');
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
}, [observerMap()]);

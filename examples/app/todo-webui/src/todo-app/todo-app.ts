// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { WebUIElement, attr, observable } from '@microsoft/webui-framework';

interface TodoItemData {
  id: string;
  title: string;
  state: string;
}

export class TodoApp extends WebUIElement {
  @attr title = '';
  @observable items: TodoItemData[] = [];
  @observable remainingCount = '0';

  addInput!: HTMLInputElement;
  private nextId = 100;

  onToggleItem(e: CustomEvent<{ id: string }>): void {
    const item = (this.items ?? []).find(i => i.id === e.detail.id);
    if (item) {
      item.state = item.state === 'done' ? 'pending' : 'done';
      this.items = [...this.items];
      this.updateRemainingCount();
    }
  }

  onDeleteItem(e: CustomEvent<{ id: string }>): void {
    this.items = (this.items ?? []).filter(item => item.id !== e.detail.id);
    this.updateRemainingCount();
  }

  onAddKeydown(e: KeyboardEvent): void {
    if (e.key === 'Enter') this.addTodo();
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
    this.updateRemainingCount();
    input.value = '';
    input.focus();
  }

  private updateRemainingCount(): void {
    this.remainingCount = String((this.items ?? []).filter(i => i.state !== 'done').length);
  }
}

TodoApp.define('todo-app');

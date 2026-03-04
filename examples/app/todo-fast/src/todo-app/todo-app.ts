import { FASTElement, attr, observable } from '@microsoft/fast-element';
import { RenderableFASTElement } from '@microsoft/fast-html';

interface TodoItemData {
  id: string;
  title: string;
  state: string;
}

export class TodoApp extends RenderableFASTElement(FASTElement) {
  @attr title = '';
  @observable items: TodoItemData[] = [];
  @observable remainingCount = 0;

  addInput!: HTMLInputElement;

  private nextId = 100;

  connectedCallback(): void {
    super.connectedCallback();
    console.log('TodoApp connected');
  }
  disconnectedCallback(): void {
    super.disconnectedCallback();
    console.log('TodoApp disconnected');
  }
  async prepare(): Promise<void> {
    const items: TodoItemData[] = [];
    for (const el of this.querySelectorAll('todo-item')) {
      items.push({
        id: el.getAttribute('id') || '',
        title: el.getAttribute('title') || '',
        state: el.getAttribute('state') || 'pending',
      });
    }
    if (items.length > 0) {
      this.items = items;
      this.nextId = Math.max(...items.map(i => Number(i.id) || 0)) + 1;
    }
    this.updateCount();
  }

  onToggleItem(e: CustomEvent<{id: string}>): void {
    const id = e.detail.id;
    this.items = this.items.map(item =>
      item.id === id
        ? { ...item, state: item.state === 'done' ? 'pending' : 'done' }
        : item
    );
    this.updateCount();
  }

  onDeleteItem(e: CustomEvent<{id: string}>): void {
    this.items = this.items.filter(item => item.id !== e.detail.id);
    this.updateCount();
  }

  onAddKeydown(e: KeyboardEvent): void {
    if (e.key === 'Enter') {
      this.addTodo();
    }
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

TodoApp.defineAsync({
  name: 'todo-app',
  templateOptions: 'defer-and-hydrate',
});

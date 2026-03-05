import { FASTElement, attr } from '@microsoft/fast-element';
import { RenderableFASTElement } from '@microsoft/fast-html';

export class CbSearchBar extends RenderableFASTElement(FASTElement) {
  @attr placeholder = 'Search contacts...';
  @attr value = '';
  private listenersAttached!: boolean;

  connectedCallback(): void {
    super.connectedCallback();
    if (this.listenersAttached) return;
    this.listenersAttached = true;
    this.addEventListener('input', (e: Event) => this.onInput(e));
    this.addEventListener('click', (e: Event) => this.onClick(e as MouseEvent));
  }

  private emit(type: string, detail?: unknown): void {
    this.dispatchEvent(new CustomEvent(type, { bubbles: true, composed: true, detail }));
  }

  onInput(e: Event): void {
    const input = e.composedPath().find(el => (el as HTMLElement).tagName === 'INPUT') as HTMLInputElement;
    if (input) {
      this.value = input.value;
      this.emit('search', { value: this.value });
    }
  }

  onClick(e: MouseEvent): void {
    const target = (e.composedPath()[0] as HTMLElement);
    if (target.closest('[data-action="clear"]')) {
      this.value = '';
      const input = this.shadowRoot?.querySelector('input');
      if (input) input.value = '';
      this.emit('search', { value: '' });
    }
  }
}

CbSearchBar.defineAsync({
  name: 'cb-search-bar',
  templateOptions: 'defer-and-hydrate',
});

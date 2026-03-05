import { FASTElement, attr } from '@microsoft/fast-element';
import { RenderableFASTElement } from '@microsoft/fast-html';

export class CbNavItem extends RenderableFASTElement(FASTElement) {
  @attr icon = '';
  @attr label = '';
  @attr count = '';
  @attr({ mode: 'boolean' }) active = false;
  private listenersAttached!: boolean;

  connectedCallback(): void {
    super.connectedCallback();
    if (this.listenersAttached) return;
    this.listenersAttached = true;
    this.addEventListener('click', () => this.onClick());
  }

  onClick(): void {
    this.dispatchEvent(new CustomEvent('nav-select', { bubbles: true, composed: true, detail: { label: this.label } }));
  }
}

CbNavItem.defineAsync({
  name: 'cb-nav-item',
  templateOptions: 'defer-and-hydrate',
});

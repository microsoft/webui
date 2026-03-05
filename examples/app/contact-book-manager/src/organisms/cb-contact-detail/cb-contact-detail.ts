import { FASTElement, attr } from '@microsoft/fast-element';
import { RenderableFASTElement } from '@microsoft/fast-html';

export class CbContactDetail extends RenderableFASTElement(FASTElement) {
  @attr id = '';
  @attr({ attribute: 'first-name' }) firstName = '';
  @attr({ attribute: 'last-name' }) lastName = '';
  @attr email = '';
  @attr phone = '';
  @attr company = '';
  @attr group = '';
  @attr favorite = '';
  @attr initials = '';
  @attr({ attribute: 'avatar-color' }) avatarColor = '';
  @attr notes = '';
  @attr address = '';

  private listenersAttached!: boolean;

  connectedCallback(): void {
    super.connectedCallback();
    if (this.listenersAttached) return;
    this.listenersAttached = true;
    this.addEventListener('click', (e: Event) => {
      this.onClick(e as MouseEvent);
    });
  }

  private emit(type: string, detail?: unknown): void {
    this.dispatchEvent(new CustomEvent(type, { bubbles: true, composed: true, detail }));
  }

  onClick(e: MouseEvent): void {
    const target = e.composedPath()[0] as HTMLElement;
    const actionEl = target.closest('[data-action]');
    if (!actionEl) return;

    const action = actionEl.getAttribute('data-action');

    switch (action) {
      case 'edit':
        this.emit('edit-contact', { id: this.id });
        break;
      case 'toggle-favorite':
        this.emit('toggle-favorite', { id: this.id });
        break;
      case 'delete':
        this.emit('delete-contact', { id: this.id });
        break;
      case 'back':
        this.emit('back');
        break;
    }
  }
}

CbContactDetail.defineAsync({
  name: 'cb-contact-detail',
  templateOptions: 'defer-and-hydrate',
});

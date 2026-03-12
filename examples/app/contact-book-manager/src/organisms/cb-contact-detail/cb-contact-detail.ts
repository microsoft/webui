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

  async prepare(): Promise<void> {
    const sr = this.shadowRoot;
    if (!sr) return;

    // Read SSR'd values from the shadow DOM before FAST overwrites them
    const avatar = sr.querySelector('.avatar') as HTMLElement | null;
    if (avatar) {
      const bg = avatar.style.backgroundColor;
      if (bg) this.avatarColor = bg;
    }
    const initialsEl = sr.querySelector('.avatar-initials');
    if (initialsEl?.textContent) this.initials = initialsEl.textContent.trim();

    const h2 = sr.querySelector('h2');
    if (h2?.textContent) {
      const parts = h2.textContent.trim().split(' ');
      this.firstName = parts[0] || '';
      this.lastName = parts.slice(1).join(' ') || '';
    }

    const companyEl = sr.querySelector('.company');
    if (companyEl?.textContent) this.company = companyEl.textContent.trim();

    const fields = sr.querySelectorAll('.field-value');
    if (fields[0]?.textContent) this.email = fields[0].textContent.trim();
    if (fields[1]?.textContent) this.phone = fields[1].textContent.trim();
    if (fields[2]?.textContent) this.address = fields[2].textContent.trim();
    if (fields[3]?.textContent) this.notes = fields[3].textContent.trim();

    const badge = sr.querySelector('.badge');
    if (badge?.textContent) this.group = badge.textContent.trim();
  }

  setInitialState(state: Record<string, unknown>): void {
    // The API spreads contact fields at top level and also includes selectedContact
    const c = (state.selectedContact as Record<string, unknown>) ?? state;
    if (!c.id) return;
    this.id = String(c.id ?? '');
    this.firstName = String(c.firstName ?? '');
    this.lastName = String(c.lastName ?? '');
    this.email = String(c.email ?? '');
    this.phone = String(c.phone ?? '');
    this.company = String(c.company ?? '');
    this.group = String(c.group ?? '');
    this.favorite = String(c.favorite ?? '');
    this.initials = String(c.initials ?? '');
    this.avatarColor = String(c.avatarColor ?? '');
    this.notes = String(c.notes ?? '');
    this.address = String(c.address ?? '');
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

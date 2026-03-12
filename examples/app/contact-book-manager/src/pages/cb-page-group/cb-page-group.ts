import { FASTElement, attr, observable } from '@microsoft/fast-element';
import { RenderableFASTElement } from '@microsoft/fast-html';

// Child components used in cb-page-group.html
import '#organisms/cb-contact-card/cb-contact-card.js';

export class CbPageGroup extends RenderableFASTElement(FASTElement) {
  @attr({ attribute: 'group-name' }) groupName = '';
  @observable contacts: any[] = [];

  async prepare(): Promise<void> {
    const sr = this.shadowRoot;
    if (!sr) return;
    const title = sr.querySelector('.page-title');
    if (title?.textContent) this.groupName = title.textContent.trim();
    const cards = sr.querySelectorAll('cb-contact-card');
    if (cards.length > 0) {
      this.contacts = Array.from(cards).map((c) => ({
        id: c.getAttribute('id') || '',
        firstName: c.getAttribute('first-name') || '',
        lastName: c.getAttribute('last-name') || '',
        email: c.getAttribute('email') || '',
        phone: c.getAttribute('phone') || '',
        company: c.getAttribute('company') || '',
        group: c.getAttribute('group') || '',
        favorite: c.getAttribute('favorite') === 'true',
        initials: c.getAttribute('initials') || '',
        avatarColor: c.getAttribute('avatar-color') || '',
        notes: c.getAttribute('notes') || '',
        address: c.getAttribute('address') || '',
      }));
    }
  }

  setInitialState(state: Record<string, unknown>, params?: Record<string, string>): void {
    this.groupName = String(state.groupName ?? params?.['group'] ?? '');
    this.contacts = (state.contacts as any[]) ?? [];
  }
}

CbPageGroup.defineAsync({
  name: 'cb-page-group',
  templateOptions: 'defer-and-hydrate',
});

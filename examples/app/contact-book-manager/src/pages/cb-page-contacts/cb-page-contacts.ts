import { FASTElement, observable } from '@microsoft/fast-element';
import { RenderableFASTElement } from '@microsoft/fast-html';
import '#organisms/cb-contact-card/cb-contact-card.js';
import { Contact } from '#api';

export class CbPageContacts extends RenderableFASTElement(FASTElement) {
  @observable contacts?: Contact[];

  async prepare(): Promise<void> {
    const sr = this.shadowRoot;
    if (!sr) return;
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

  setInitialState(state: Record<string, unknown>): void {
    this.contacts = (state.contacts as any[]) ?? [];
  }
}

CbPageContacts.defineAsync({
  name: 'cb-page-contacts',
  templateOptions: 'defer-and-hydrate',
});

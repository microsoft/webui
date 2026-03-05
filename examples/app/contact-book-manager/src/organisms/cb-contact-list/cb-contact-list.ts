import { FASTElement, observable } from '@microsoft/fast-element';
import { RenderableFASTElement } from '@microsoft/fast-html';

interface Contact {
  id: string;
  firstName: string;
  lastName: string;
  email: string;
  phone: string;
  company: string;
  group: string;
  favorite: boolean;
  initials: string;
  avatarColor: string;
  notes: string;
  address: string;
}

export class CbContactList extends RenderableFASTElement(FASTElement) {
  @observable contacts!: Contact[];
  @observable hasContacts = true;

  async prepare(): Promise<void> {
    const contacts: Contact[] = [];
    for (const el of this.shadowRoot!.querySelectorAll('cb-contact-card')) {
      contacts.push({
        id: el.getAttribute('id') || '',
        firstName: el.getAttribute('first-name') || '',
        lastName: el.getAttribute('last-name') || '',
        email: el.getAttribute('email') || '',
        phone: el.getAttribute('phone') || '',
        company: el.getAttribute('company') || '',
        group: el.getAttribute('group') || '',
        favorite: el.getAttribute('favorite') === 'true',
        initials: el.getAttribute('initials') || '',
        avatarColor: el.getAttribute('avatar-color') || '#6B7280',
        notes: el.getAttribute('notes') || '',
        address: el.getAttribute('address') || '',
      });
    }
    this.contacts = contacts;
    this.hasContacts = contacts.length > 0;
  }

  contactsChanged(): void {
    this.hasContacts = this.contacts && this.contacts.length > 0;
  }
}

CbContactList.defineAsync({
  name: 'cb-contact-list',
  templateOptions: 'defer-and-hydrate',
});

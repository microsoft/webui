import { FASTElement, attr, observable } from '@microsoft/fast-element';
import { RenderableFASTElement } from '@microsoft/fast-html';

// Child components used in cb-page-dashboard.html
import '../../organisms/cb-contact-card/cb-contact-card.js';

export class CbPageDashboard extends RenderableFASTElement(FASTElement) {
  @attr({ attribute: 'total-contacts' }) totalContacts = '0';
  @attr({ attribute: 'total-favorites' }) totalFavorites = '0';
  @attr({ attribute: 'total-groups' }) totalGroups = '0';
  @observable recentContacts: any[] = [];

  async prepare(): Promise<void> {
    if (!this.shadowRoot) return;
    // Read recent contacts from SSR'd cards
    const cards = this.shadowRoot.querySelectorAll('cb-contact-card');
    const contacts: any[] = [];
    for (const card of cards) {
      contacts.push({
        id: card.getAttribute('id') || '',
        firstName: card.getAttribute('first-name') || '',
        lastName: card.getAttribute('last-name') || '',
        email: card.getAttribute('email') || '',
        phone: card.getAttribute('phone') || '',
        company: card.getAttribute('company') || '',
        group: card.getAttribute('group') || '',
        favorite: card.getAttribute('favorite') === 'true',
        initials: card.getAttribute('initials') || '',
        avatarColor: card.getAttribute('avatar-color') || '',
        notes: card.getAttribute('notes') || '',
        address: card.getAttribute('address') || '',
      });
    }
    if (contacts.length > 0) {
      this.recentContacts = contacts;
    }
  }

  setInitialState(state: Record<string, unknown>): void {
    this.totalContacts = String(state.totalContacts ?? 0);
    this.totalFavorites = String(state.totalFavorites ?? 0);
    this.totalGroups = String(state.totalGroups ?? 0);
    this.recentContacts = (state.recentContacts as any[]) ?? [];
  }
}

CbPageDashboard.defineAsync({
  name: 'cb-page-dashboard',
  templateOptions: 'defer-and-hydrate',
});

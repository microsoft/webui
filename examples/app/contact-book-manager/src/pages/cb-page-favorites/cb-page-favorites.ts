import { FASTElement, observable } from '@microsoft/fast-element';
import { RenderableFASTElement } from '@microsoft/fast-html';

// Child components used in cb-page-favorites.html
import '../../organisms/cb-contact-card/cb-contact-card.js';

export class CbPageFavorites extends RenderableFASTElement(FASTElement) {
  @observable favoriteContacts: any[] = [];

  async prepare(): Promise<void> {
    const sr = this.shadowRoot;
    if (!sr) return;
    const cards = sr.querySelectorAll('cb-contact-card');
    if (cards.length > 0) {
      this.favoriteContacts = Array.from(cards).map((c) => ({
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
    this.favoriteContacts = (state.contacts as any[]) ?? [];
  }
}

CbPageFavorites.defineAsync({
  name: 'cb-page-favorites',
  templateOptions: 'defer-and-hydrate',
});

import { FASTElement, attr, observable } from '@microsoft/fast-element';
import { RenderableFASTElement } from '@microsoft/fast-html';

export class CbSidebar extends RenderableFASTElement(FASTElement) {
  @attr page = 'dashboard';
  @attr({ attribute: 'active-group' }) activeGroup = 'all';
  @attr({ attribute: 'total-contacts' }) totalContacts = '0';
  @attr({ attribute: 'total-favorites' }) totalFavorites = '0';
  groups!: string[];
  @observable dashboardActive!: boolean;
  @observable contactsActive!: boolean;
  @observable favoritesActive!: boolean;

  private listenersAttached!: boolean;

  connectedCallback(): void {
    super.connectedCallback();
    if (this.listenersAttached) return;
    this.listenersAttached = true;
    this.addEventListener('click', (e: Event) => {
      this.onNavClick(e as MouseEvent);
    });
  }

  async prepare(): Promise<void> {
    if (!this.shadowRoot) return;

    const groups: string[] = [];
    for (const el of this.shadowRoot.querySelectorAll('.nav-item')) {
      const label = el.getAttribute('data-nav') || '';
      if (['Dashboard', 'All Contacts', 'Favorites'].includes(label)) continue;
      if (label) groups.push(label);
    }
    this.groups = groups;
    this.updateActiveState();
  }

  pageChanged(): void {
    this.updateActiveState();
  }

  private updateActiveState(): void {
    this.dashboardActive = this.page === 'dashboard';
    this.contactsActive = this.page === 'contacts';
    this.favoritesActive = this.page === 'favorites';

    // Update active class on nav items
    for (const el of this.shadowRoot?.querySelectorAll('.nav-item') || []) {
      const nav = el.getAttribute('data-nav') || '';
      let isActive = false;
      if (nav === 'Dashboard') isActive = this.dashboardActive;
      else if (nav === 'All Contacts') isActive = this.contactsActive;
      else if (nav === 'Favorites') isActive = this.favoritesActive;
      else if (this.page === 'group') isActive = nav === this.activeGroup;
      el.classList.toggle('active', isActive);
    }
  }

  private emit(type: string, detail?: unknown): void {
    this.dispatchEvent(new CustomEvent(type, { bubbles: true, composed: true, detail }));
  }

  onNavClick(e: MouseEvent): void {
    const target = (e.composedPath()[0] as HTMLElement).closest('.nav-item') as HTMLElement | null;
    if (!target) return;
    const label = target.getAttribute('data-nav') || '';
    if (label === 'Dashboard') this.emit('navigate', { page: 'dashboard' });
    else if (label === 'All Contacts') this.emit('navigate', { page: 'contacts' });
    else if (label === 'Favorites') this.emit('navigate', { page: 'favorites' });
    else this.emit('navigate', { page: 'group', group: label });
  }
}

CbSidebar.defineAsync({
  name: 'cb-sidebar',
  templateOptions: 'defer-and-hydrate',
});

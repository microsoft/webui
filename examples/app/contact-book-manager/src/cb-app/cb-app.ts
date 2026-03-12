import { FASTElement, attr } from '@microsoft/fast-element';
import { RenderableFASTElement } from '@microsoft/fast-html';
import { Router } from '@microsoft/webui-router';
import { api } from '#api';

// Child components used in cb-app.html
import '#organisms/cb-header/cb-header.js';
import '#organisms/cb-sidebar/cb-sidebar.js';

/** Map route name → sidebar page value for highlighting. */
const ROUTE_TO_PAGE: Record<string, string> = {
  dashboard: 'dashboard',
  contacts: 'contacts',
  favorites: 'favorites',
  group: 'group',
  detail: 'detail',
  add: 'add',
  edit: 'edit',
};

export class CbApp extends RenderableFASTElement(FASTElement) {
  @attr page = '';
  @attr({ attribute: 'search-query' }) searchQuery = '';
  @attr({ attribute: 'active-group' }) activeGroup = 'all';
  @attr({ attribute: 'total-contacts' }) totalContacts = '0';
  @attr({ attribute: 'total-favorites' }) totalFavorites = '0';
  @attr({ attribute: 'total-groups' }) totalGroups = '0';

  private listenersAttached!: boolean;

  connectedCallback(): void {
    super.connectedCallback();
    if (this.listenersAttached) return;
    this.listenersAttached = true;
    const root = this.shadowRoot;
    if (!root) return;

    root.addEventListener('search', (e: Event) => {
      e.stopPropagation();
      this.searchQuery = (e as CustomEvent).detail.value;
    });
    root.addEventListener('add-contact', (e: Event) => {
      e.stopPropagation();
      Router.navigate('/contacts/add');
    });
    root.addEventListener('select-contact', (e: Event) => {
      e.stopPropagation();
      Router.navigate(`/contacts/${(e as CustomEvent).detail.id}`);
    });
    root.addEventListener('edit-contact', (e: Event) => {
      e.stopPropagation();
      Router.navigate(`/contacts/${(e as CustomEvent).detail.id}/edit`);
    });
    root.addEventListener('delete-contact', (e: Event) => {
      e.stopPropagation();
      this.onDeleteContact((e as CustomEvent).detail.id);
    });
    root.addEventListener('toggle-favorite', (e: Event) => {
      e.stopPropagation();
      this.onToggleFavorite((e as CustomEvent).detail.id);
    });
    root.addEventListener('back', (e: Event) => {
      e.stopPropagation();
      Router.back();
    });
    root.addEventListener('form-save', (e: Event) => {
      e.stopPropagation();
      this.onFormSave((e as CustomEvent).detail);
    });
    root.addEventListener('form-cancel', (e: Event) => {
      e.stopPropagation();
      Router.back();
    });

    window.addEventListener('webui:route:navigated', (e: Event) => {
      const { routeName, params } = (e as CustomEvent).detail;
      this.onRouteChanged(routeName, params);
    });
  }

  async prepare(): Promise<void> {
    await this.refreshStats();
  }

  setInitialState(state: Record<string, unknown>): void {
    if (state.totalContacts !== undefined) this.totalContacts = String(state.totalContacts);
    if (state.totalFavorites !== undefined) this.totalFavorites = String(state.totalFavorites);
    if (state.totalGroups !== undefined) this.totalGroups = String(state.totalGroups);
  }

  /** Called when the router activates a new route. */
  private onRouteChanged(routeName: string, params: Record<string, string>): void {
    this.page = ROUTE_TO_PAGE[routeName] || '';
    if (routeName === 'group') {
      this.activeGroup = params['group'] || 'all';
    }
  }

  /** Fetch global stats from the API and update sidebar attributes. */
  private async refreshStats(): Promise<void> {
    try {
      const stats = await api.stats();
      this.totalContacts = String(stats.totalContacts);
      this.totalFavorites = String(stats.totalFavorites);
      this.totalGroups = String(stats.totalGroups);
    } catch {
      // Stats unavailable — keep current values
    }
  }

  private async onDeleteContact(id: string): Promise<void> {
    try {
      await api.contacts.delete(id);
      await this.refreshStats();
      Router.navigate('/contacts');
    } catch {
      // Deletion failed — stay on current page
    }
  }

  private async onToggleFavorite(id: string): Promise<void> {
    try {
      await api.contacts.toggleFavorite(id);
      await this.refreshStats();
      // Re-navigate to refetch state for the active route
      Router.navigate(location.pathname);
    } catch {
      // Toggle failed
    }
  }

  private async onFormSave(data: Record<string, string>): Promise<void> {
    try {
      let saved;
      if (data.id) {
        saved = await api.contacts.update(data.id, data);
      } else {
        saved = await api.contacts.create(data);
      }
      await this.refreshStats();
      Router.navigate(`/contacts/${saved.id}`);
    } catch {
      // Save failed
    }
  }
}

CbApp.defineAsync({
  name: 'cb-app',
  templateOptions: 'defer-and-hydrate',
});

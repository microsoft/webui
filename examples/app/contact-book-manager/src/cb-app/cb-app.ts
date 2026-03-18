// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { FASTElement, attr } from '@microsoft/fast-element';
import { RenderableFASTElement } from '@microsoft/fast-html';
import { Router } from '@microsoft/webui-router';
import { api } from '#api';

// Child components used in cb-app.html
import '#organisms/cb-header/cb-header.js';
import '#organisms/cb-sidebar/cb-sidebar.js';

/** Map route component → sidebar page value for highlighting. */
const COMPONENT_TO_PAGE: Record<string, string> = {
  'cb-page-dashboard': 'dashboard',
  'cb-page-contacts': 'contacts',
  'cb-page-favorites': 'favorites',
  'cb-page-group': 'group',
  'cb-contact-detail': 'contacts',
  'cb-contact-form': 'contacts',
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
      const { component, params } = (e as CustomEvent).detail;
      this.onRouteChanged(component, params);
    });
  }

  async prepare(): Promise<void> {
    await this.refreshStats();
  }

  setInitialState(state: Record<string, unknown>): void {
    if (state.page !== undefined) this.page = String(state.page);
    if (state.activeGroup !== undefined) this.activeGroup = String(state.activeGroup);
    if (state.totalContacts !== undefined) this.totalContacts = String(state.totalContacts);
    if (state.totalFavorites !== undefined) this.totalFavorites = String(state.totalFavorites);
    if (state.totalGroups !== undefined) this.totalGroups = String(state.totalGroups);
  }

  /** Called when the router activates a new route. */
  private onRouteChanged(component: string, params: Record<string, string>): void {
    this.page = COMPONENT_TO_PAGE[component] || '';
    if (component === 'cb-page-group') {
      this.activeGroup = (params['group'] || 'all').toLowerCase();
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
      console.error('Failed to fetch stats from API');
    }
  }

  private async onDeleteContact(id: string): Promise<void> {
    try {
      await api.contacts.delete(id);
      await this.refreshStats();
      Router.navigate('/contacts');
    } catch {
      console.error(`Failed to delete contact with id ${id}`);
    }
  }

  private async onToggleFavorite(id: string): Promise<void> {
    try {
      await api.contacts.toggleFavorite(id);
      await this.refreshStats();
      // Re-navigate to refetch state for the active route
      Router.navigate(location.pathname);
    } catch {
      console.error(`Failed to toggle favorite for contact with id ${id}`);
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
      console.error('Failed to save contact', data);
    }
  }
}

CbApp.defineAsync({
  name: 'cb-app',
  templateOptions: 'defer-and-hydrate',
});

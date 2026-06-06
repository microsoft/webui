// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { WebUIElement, observable } from '@microsoft/webui-framework';
import { Router, isStateful } from '@microsoft/webui-router';
import { api, type Contact } from '#api';

// Child components used in cb-app.html
import '#organisms/cb-header/cb-header.js';
import '#organisms/cb-sidebar/cb-sidebar.js';

type SearchChangeEvent = CustomEvent<{ value: string }>;
type ContactEvent = CustomEvent<{ id: string }>;
type FormSaveEvent = CustomEvent<Record<string, string>>;

export class CbApp extends WebUIElement {
  @observable page = '';
  @observable searchQuery = '';
  @observable activeGroup = 'all';
  @observable totalContacts = '0';
  @observable totalFavorites = '0';
  @observable groups: string[] = [];

  private searchGen = 0;

  private readonly onNavigated = (): void => {
    if (this.searchQuery) void this.applySearch(this.searchQuery);
  };

  override connectedCallback(): void {
    super.connectedCallback();
    window.addEventListener('webui:route:navigated', this.onNavigated);
  }

  override disconnectedCallback(): void {
    super.disconnectedCallback();
    window.removeEventListener('webui:route:navigated', this.onNavigated);
  }

  onSearch(e: SearchChangeEvent): void {
    this.searchQuery = e.detail.value;
    void this.applySearch(e.detail.value);
  }

  private async applySearch(query: string): Promise<void> {
    const gen = ++this.searchGen;
    const root = this.shadowRoot;
    if (!root) return;

    const activeRoute = root.querySelector('webui-route[active]');
    if (!activeRoute) return;

    const contactsPage = activeRoute.querySelector('cb-page-contacts');
    const favoritesPage = activeRoute.querySelector('cb-page-favorites');
    const groupPage = activeRoute.querySelector('cb-page-group');

    let pageEl: Element | null = null;
    let favorites = false;
    let group = '';

    if (contactsPage) {
      pageEl = contactsPage;
    } else if (favoritesPage) {
      pageEl = favoritesPage;
      favorites = true;
    } else if (groupPage) {
      pageEl = groupPage;
      group = this.activeGroup;
    }

    if (!pageEl) return;

    const contacts: Contact[] = await api.contacts.list({
      q: query || undefined,
      favorites: favorites || undefined,
      group: group || undefined,
    });

    if (gen !== this.searchGen) return;
    if (isStateful(pageEl)) pageEl.setState({ contacts });
  }

  onSelectContact(e: ContactEvent): void {
    Router.navigate(`/contacts/${e.detail.id}`);
  }

  onEditContact(e: ContactEvent): void {
    Router.navigate(`/contacts/${e.detail.id}/edit`);
  }

  async onDeleteContactEvent(e: ContactEvent): Promise<void> {
    const id = e.detail.id;
    try {
      await api.contacts.delete(id);
      Router.navigate('/contacts');
    } catch {
      console.error(`Failed to delete contact with id ${id}`);
    }
  }

  async onToggleFavoriteEvent(e: ContactEvent): Promise<void> {
    const id = e.detail.id;
    try {
      const updated = await api.contacts.toggleFavorite(id);
      const count = Number.parseInt(this.totalFavorites, 10) || 0;
      this.totalFavorites = String(count + (updated.favorite ? 1 : -1));
    } catch {
      console.error(`Failed to toggle favorite for contact with id ${id}`);
    }
  }

  onBack(): void {
    Router.back();
  }

  async onFormSaveEvent(e: FormSaveEvent): void {
    const data = e.detail;
    try {
      let saved;
      if (data.id) {
        saved = await api.contacts.update(data.id, data);
      } else {
        saved = await api.contacts.create(data);
      }
      Router.navigate(`/contacts/${saved.id}`);
    } catch {
      console.error('Failed to save contact', data);
    }
  }

  onFormCancel(): void {
    Router.back();
  }
}

CbApp.define('cb-app');

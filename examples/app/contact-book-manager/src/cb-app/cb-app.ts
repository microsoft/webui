// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { WebUIElement, observable } from '@microsoft/webui-framework';
import { Router } from '@microsoft/webui-router';
import type { NavigationEvent } from '@microsoft/webui-router';
import { api } from '#api';

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

  // Mirror the `q` route query param into the search box so the input stays in
  // sync on direct loads, back/forward, and when navigating to an unfiltered
  // route. Filtering itself is owned by each list route's loader.
  private readonly onNavigated = (e: Event): void => {
    const { query } = (e as CustomEvent<NavigationEvent>).detail;
    this.searchQuery = query.q ?? '';
  };

  override connectedCallback(): void {
    super.connectedCallback();
    window.addEventListener('webui:route:navigated', this.onNavigated);
  }

  override disconnectedCallback(): void {
    super.disconnectedCallback();
    window.removeEventListener('webui:route:navigated', this.onNavigated);
  }

  // Model search as route query state: write `q` to the URL and let the active
  // list route's loader fetch the filtered contacts. The shell no longer reads
  // the active page, calls the contacts API, or pushes state into pages.
  onSearch(e: SearchChangeEvent): void {
    this.searchQuery = e.detail.value;

    const next = new URL(window.location.href);
    const query = e.detail.value.trim();
    if (query) {
      next.searchParams.set('q', query);
    } else {
      next.searchParams.delete('q');
    }

    Router.navigate(`${next.pathname}${next.search}`);
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

  async onFormSaveEvent(e: FormSaveEvent): Promise<void> {
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

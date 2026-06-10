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
    // While a search debounce is pending the user is actively typing, so a
    // navigation settling mid-keystroke (for example the list route's own
    // navigated event arriving just after the first character) must not
    // overwrite the in-progress query. The pending debounce will write the
    // final `q` to the URL, and the resulting navigation mirrors it back.
    if (this.searchTimer !== null) return;
    const { query } = (e as CustomEvent<NavigationEvent>).detail;
    this.searchQuery = query.q ?? '';
  };

  // Trailing-debounce handle for search-as-you-type. Coalesces rapid keystrokes
  // into a single navigation so the history stack does not gain one entry per
  // character.
  private searchTimer: ReturnType<typeof setTimeout> | null = null;

  override connectedCallback(): void {
    super.connectedCallback();
    window.addEventListener('webui:route:navigated', this.onNavigated);
  }

  override disconnectedCallback(): void {
    super.disconnectedCallback();
    window.removeEventListener('webui:route:navigated', this.onNavigated);
    if (this.searchTimer !== null) {
      clearTimeout(this.searchTimer);
      this.searchTimer = null;
    }
  }

  // Model search as route query state: write `q` to the URL and let the active
  // list route's loader fetch the filtered contacts. The shell no longer reads
  // the active page, calls the contacts API, or pushes state into pages.
  //
  // Navigation is debounced so typing does not fire a route load per keystroke.
  // Entering or leaving a search (adding/removing `q`) pushes a history entry,
  // so one Back returns to the pre-search list; refining an existing query
  // replaces the entry instead, so the intermediate keystrokes do not stack.
  onSearch(e: SearchChangeEvent): void {
    this.searchQuery = e.detail.value;
    const value = e.detail.value;

    if (this.searchTimer !== null) clearTimeout(this.searchTimer);
    this.searchTimer = setTimeout(() => {
      this.searchTimer = null;

      const current = new URL(window.location.href);
      const hadQuery = current.searchParams.has('q');

      const next = new URL(current.href);
      const query = value.trim();
      if (query) {
        next.searchParams.set('q', query);
      } else {
        next.searchParams.delete('q');
      }

      // No-op if the URL would not change (e.g. clearing an already-empty box).
      if (next.search === current.search) return;

      const hasQuery = next.searchParams.has('q');
      Router.navigate(`${next.pathname}${next.search}`, { replace: hadQuery && hasQuery });
    }, 200);
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

// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

/**
 * API client for the contact-book-manager backend.
 */

const API_BASE = './api';

export interface Contact {
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

export interface Stats {
  totalContacts: number;
  totalFavorites: number;
  totalGroups: number;
  groups: string[];
  recentContacts: Contact[];
}

export interface GroupResult {
  groupName: string;
  contacts: Contact[];
}

async function request<T>(path: string, options?: RequestInit): Promise<T> {
  const resp = await fetch(`${API_BASE}${path}`, {
    headers: { 'Content-Type': 'application/json' },
    ...options,
  });
  if (!resp.ok) {
    throw new Error(`API error: ${resp.status} ${resp.statusText}`);
  }
  if (resp.status === 204) return null as T;
  return resp.json();
}

export const api = {
  contacts: {
    list: (
      params?: { q?: string; group?: string; favorites?: boolean },
      opts?: { signal?: AbortSignal },
    ) => {
      const qs = new URLSearchParams();
      if (params?.q) qs.set('q', params.q);
      if (params?.group) qs.set('group', params.group);
      if (params?.favorites) qs.set('favorites', 'true');
      const query = qs.toString();
      return request<Contact[]>(`/contacts${query ? `?${query}` : ''}`, { signal: opts?.signal });
    },
    get: (id: string) => request<Contact>(`/contacts/${id}`),
    create: (data: Partial<Contact>) =>
      request<Contact>('/contacts', { method: 'POST', body: JSON.stringify(data) }),
    update: (id: string, data: Partial<Contact>) =>
      request<Contact>(`/contacts/${id}`, { method: 'PUT', body: JSON.stringify(data) }),
    delete: (id: string) =>
      request<void>(`/contacts/${id}`, { method: 'DELETE' }),
    toggleFavorite: async (id: string) => {
      const contact = await request<Contact>(`/contacts/${id}`);
      return request<Contact>(`/contacts/${id}`, {
        method: 'PUT',
        body: JSON.stringify({ ...contact, favorite: !contact.favorite }),
      });
    },
  },
  groups: {
    // Resolve a group by its (case-insensitive) slug. The server canonicalizes
    // the display name and applies the optional `q` search filter, returning
    // both the canonical name and the filtered contacts in one response.
    get: (
      slug: string,
      params?: { q?: string },
      opts?: { signal?: AbortSignal },
    ) => {
      const qs = new URLSearchParams();
      if (params?.q) qs.set('q', params.q);
      const query = qs.toString();
      return request<GroupResult>(
        `/groups/${encodeURIComponent(slug)}${query ? `?${query}` : ''}`,
        { signal: opts?.signal },
      );
    },
  },
  stats: () => request<Stats>('/stats'),
};

// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

/**
 * Express API server for the contact-book-manager example.
 *
 * Loads contacts from data/state.json on startup and serves both:
 *   - Route state endpoints (content-negotiated, for SSR rendering)
 *   - REST API endpoints under /api/ (for client-side CRUD)
 */

import express, { type Request, type Response } from 'express';
import cors from 'cors';
import { readFileSync } from 'node:fs';
import { join, dirname } from 'node:path';
import { fileURLToPath } from 'node:url';
import { randomUUID } from 'node:crypto';

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

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

// ---------------------------------------------------------------------------
// Data layer
// ---------------------------------------------------------------------------

const __dirname = dirname(fileURLToPath(import.meta.url));
const DATA_PATH = join(__dirname, '..', 'data', 'state.json');

const AVATAR_COLORS = [
  '#4A90D9', '#E74C3C', '#2ECC71', '#F39C12', '#9B59B6',
  '#1ABC9C', '#E67E22', '#3498DB', '#E91E63', '#00BCD4',
];

const stateData = JSON.parse(readFileSync(DATA_PATH, 'utf-8'));
let contacts: Contact[] = stateData.contacts;
let groups: string[] = stateData.groups ?? [];

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

function findContact(id: string): Contact | undefined {
  return contacts.find(c => c.id === id);
}

function uniqueGroups(): string[] {
  return groups;
}

/** Ensures the group name exists in the stable groups list. */
function ensureGroup(group: string): void {
  if (group && !groups.includes(group)) {
    groups.push(group);
  }
}

function favoriteContacts(): Contact[] {
  return contacts.filter(c => c.favorite);
}

function recentContacts(count: number): Contact[] {
  // Return last N contacts in reverse order (most recent first)
  const start = contacts.length > count ? contacts.length - count : 0;
  const recent: Contact[] = [];
  for (let i = contacts.length - 1; i >= start; i--) {
    recent.push(contacts[i]);
  }
  return recent;
}

function buildStats() {
  const favorites = favoriteContacts();
  const groups = uniqueGroups();
  return {
    totalContacts: contacts.length,
    totalFavorites: favorites.length,
    totalGroups: groups.length,
    groups,
    recentContacts: recentContacts(5),
    favoriteContacts: favorites,
  };
}

function computeInitials(firstName: string, lastName: string): string {
  const first = firstName.length > 0 ? firstName[0].toUpperCase() : '';
  const last = lastName.length > 0 ? lastName[0].toUpperCase() : '';
  return first + last;
}

function pickAvatarColor(): string {
  return AVATAR_COLORS[contacts.length % AVATAR_COLORS.length];
}

/** Returns true if the Accept header includes application/json. */
function wantsJson(req: Request): boolean {
  const accept = req.headers.accept || '';
  return accept.indexOf('application/json') !== -1;
}

// ---------------------------------------------------------------------------
// Express app
// ---------------------------------------------------------------------------

const app = express();

app.use(cors({ origin: true }));
app.use(express.json());

// ---------------------------------------------------------------------------
// Route state endpoints (content-negotiated for SSR)
// ---------------------------------------------------------------------------

// All SSR routes require Accept: application/json
const ssr = express.Router();
ssr.use((req: Request, res: Response, next) => {
  if (!wantsJson(req)) { res.status(404).json({ error: 'Not found' }); return; }
  next();
});

/** Lightweight state needed by the sidebar shell during SSR. */
function sidebarState() {
  return {
    totalContacts: contacts.length,
    totalFavorites: favoriteContacts().length,
    totalGroups: uniqueGroups().length,
    groups: uniqueGroups(),
  };
}

// Dashboard — needs stats for the stat cards + recent contacts
ssr.get('/', (_req: Request, res: Response) => {
  res.json({
    state: {
      page: 'dashboard',
      ...sidebarState(),
      recentContacts: recentContacts(5),
    },
  });
});

// All contacts
ssr.get('/contacts', (_req: Request, res: Response) => {
  res.json({ state: { page: 'contacts', ...sidebarState(), contacts } });
});

// Add contact form — must be before /contacts/:id to avoid matching "add" as an id
ssr.get('/contacts/add', (_req: Request, res: Response) => {
  const sidebar = sidebarState();
  res.json({
    state: {
      page: 'contacts',
      ...sidebar,
      selectedGroup: sidebar.groups[0] ?? '',
      formTitle: 'Add Contact',
    },
  });
});

// Edit contact form — must be before /contacts/:id to avoid conflicts
ssr.get('/contacts/:id/edit', (req: Request, res: Response) => {
  const contact = findContact(req.params.id);
  if (!contact) { res.status(404).json({ error: 'Contact not found' }); return; }
  const { id, ...contactState } = contact;
  res.json({
    state: {
      page: 'contacts',
      ...sidebarState(),
      ...contactState,
      editId: id,
      selectedGroup: contact.group,
      formTitle: 'Edit Contact',
    },
  });
});

// Contact detail — spread contact fields at top level for SSR template bindings
ssr.get('/contacts/:id', (req: Request, res: Response) => {
  const contact = findContact(req.params.id);
  if (!contact) { res.status(404).json({ error: 'Contact not found' }); return; }
  res.json({ state: { page: 'contacts', ...sidebarState(), ...contact, selectedContact: contact } });
});

// Favorites
ssr.get('/favorites', (_req: Request, res: Response) => {
  res.json({ state: { page: 'favorites', ...sidebarState(), contacts: favoriteContacts() } });
});

// Group-filtered contacts
ssr.get('/groups/:group', (req: Request, res: Response) => {
  const groupSlug = req.params.group;
  const filtered = contacts.filter(c => c.group.toLowerCase() === groupSlug.toLowerCase());
  const displayName = filtered[0]?.group ?? groupSlug;
  res.json({
    state: {
      page: 'group',
      activeGroup: displayName,
      ...sidebarState(),
      contacts: filtered,
      groupName: displayName,
    },
  });
});

// ---------------------------------------------------------------------------
// REST API endpoints (client-side CRUD)
// Must be registered before the SSR router so that /api/* requests are not
// blocked by the SSR content-negotiation middleware.
// ---------------------------------------------------------------------------

// List contacts with optional filtering
app.get('/api/contacts', (_req: Request, res: Response) => {
  let result = contacts;

  const query = String(_req.query.q || '');
  if (query) {
    const q = query.toLowerCase();
    result = result.filter(c =>
      c.firstName.toLowerCase().indexOf(q) !== -1 ||
      c.lastName.toLowerCase().indexOf(q) !== -1 ||
      c.email.toLowerCase().indexOf(q) !== -1 ||
      c.company.toLowerCase().indexOf(q) !== -1
    );
  }

  const group = String(_req.query.group || '');
  if (group) {
    result = result.filter(c => c.group === group);
  }

  if (_req.query.favorites === 'true') {
    result = result.filter(c => c.favorite);
  }

  res.json(result);
});

// Get single contact
app.get('/api/contacts/:id', (req: Request, res: Response) => {
  const contact = findContact(req.params.id);
  if (!contact) { res.status(404).json({ error: 'Contact not found' }); return; }
  res.json(contact);
});

// Create contact
app.post('/api/contacts', (req: Request, res: Response) => {
  const body = req.body || {};
  const newContact: Contact = {
    id: randomUUID(),
    firstName: String(body.firstName || ''),
    lastName: String(body.lastName || ''),
    email: String(body.email || ''),
    phone: String(body.phone || ''),
    company: String(body.company || ''),
    group: String(body.group || 'Other'),
    favorite: Boolean(body.favorite),
    initials: computeInitials(String(body.firstName || ''), String(body.lastName || '')),
    avatarColor: String(body.avatarColor || '') || pickAvatarColor(),
    notes: String(body.notes || ''),
    address: String(body.address || ''),
  };
  contacts.push(newContact);
  ensureGroup(newContact.group);
  res.status(201).json(newContact);
});

// Update contact
app.put('/api/contacts/:id', (req: Request, res: Response) => {
  const idx = contacts.findIndex(c => c.id === req.params.id);
  if (idx === -1) { res.status(404).json({ error: 'Contact not found' }); return; }

  const existing = contacts[idx];
  const body = req.body || {};

  const firstName = body.firstName !== undefined ? String(body.firstName) : existing.firstName;
  const lastName = body.lastName !== undefined ? String(body.lastName) : existing.lastName;

  const updated: Contact = {
    id: existing.id,
    firstName,
    lastName,
    email: body.email !== undefined ? String(body.email) : existing.email,
    phone: body.phone !== undefined ? String(body.phone) : existing.phone,
    company: body.company !== undefined ? String(body.company) : existing.company,
    group: body.group !== undefined ? String(body.group) : existing.group,
    favorite: body.favorite !== undefined ? Boolean(body.favorite) : existing.favorite,
    initials: computeInitials(firstName, lastName),
    avatarColor: body.avatarColor !== undefined ? String(body.avatarColor) : existing.avatarColor,
    notes: body.notes !== undefined ? String(body.notes) : existing.notes,
    address: body.address !== undefined ? String(body.address) : existing.address,
  };

  contacts[idx] = updated;
  ensureGroup(updated.group);
  res.json(updated);
});

// Delete contact
app.delete('/api/contacts/:id', (req: Request, res: Response) => {
  const idx = contacts.findIndex(c => c.id === req.params.id);
  if (idx === -1) { res.status(404).json({ error: 'Contact not found' }); return; }
  contacts.splice(idx, 1);
  res.status(204).end();
});

// Stats
app.get('/api/stats', (_req: Request, res: Response) => {
  res.json(buildStats());
});

// ---------------------------------------------------------------------------
// SSR route state endpoints (content-negotiated, must come after /api/*)
// ---------------------------------------------------------------------------

app.use(ssr);

// ---------------------------------------------------------------------------
// Start
// ---------------------------------------------------------------------------

const PORT = Number(process.env.PORT) || 3013;

app.listen(PORT, () => {
  console.log(`Contact Book API server listening on http://localhost:${PORT}`);
  console.log(`  Loaded ${contacts.length} contacts from ${DATA_PATH}`);
});

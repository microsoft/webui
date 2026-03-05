import { FASTElement, attr, observable } from '@microsoft/fast-element';
import { RenderableFASTElement } from '@microsoft/fast-html';

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

const AVATAR_COLORS = [
  '#4A90D9', '#E74C3C', '#2ECC71', '#F39C12', '#9B59B6',
  '#1ABC9C', '#E67E22', '#3498DB', '#E91E63', '#00BCD4',
];

const groupsStore = new WeakMap<object, string[]>();

export class CbApp extends RenderableFASTElement(FASTElement) {
  @attr page = 'dashboard';
  @attr({ attribute: 'search-query' }) searchQuery = '';
  @attr({ attribute: 'active-group' }) activeGroup = 'all';
  @attr({ attribute: 'total-contacts' }) totalContacts = '0';
  @attr({ attribute: 'total-favorites' }) totalFavorites = '0';
  @attr({ attribute: 'total-groups' }) totalGroups = '0';

  @observable contacts!: Contact[];
  @observable filteredContacts!: Contact[];
  @observable favoriteContacts!: Contact[];
  @observable recentContacts!: Contact[];
  @observable selectedContact: Contact | null = null;

  private nextId = 100;
  private db: IDBDatabase | null = null;
  private listenersAttached!: boolean;

  connectedCallback(): void {
    super.connectedCallback();
    if (this.listenersAttached) return;
    this.listenersAttached = true;
    const root = this.shadowRoot;
    if (!root) return;
    root.addEventListener('navigate', (e: Event) => {
      e.stopPropagation();
      this.onNavigate(e as CustomEvent);
    });
    root.addEventListener('search', (e: Event) => {
      e.stopPropagation();
      this.onSearch(e as CustomEvent);
    });
    root.addEventListener('add-contact', (e: Event) => {
      e.stopPropagation();
      this.onAddContact();
    });
    root.addEventListener('select-contact', (e: Event) => {
      e.stopPropagation();
      this.onSelectContact(e as CustomEvent);
    });
    root.addEventListener('edit-contact', (e: Event) => {
      e.stopPropagation();
      this.onEditContact(e as CustomEvent);
    });
    root.addEventListener('delete-contact', (e: Event) => {
      e.stopPropagation();
      this.onDeleteContact(e as CustomEvent);
    });
    root.addEventListener('toggle-favorite', (e: Event) => {
      e.stopPropagation();
      this.onToggleFavorite(e as CustomEvent);
    });
    root.addEventListener('back', (e: Event) => {
      e.stopPropagation();
      this.onBack();
    });
    root.addEventListener('form-save', (e: Event) => {
      e.stopPropagation();
      this.onFormSave(e as CustomEvent);
    });
    root.addEventListener('form-cancel', (e: Event) => {
      e.stopPropagation();
      this.onFormCancel();
    });
  }

  async prepare(): Promise<void> {
    if (!this.shadowRoot) return;

    // Read ALL contacts from the hidden data store
    const dataEls = this.shadowRoot.querySelectorAll('.contact-data');
    const contacts: Contact[] = [];
    const seen = new Set<string>();
    for (const el of dataEls) {
      const id = (el as HTMLElement).dataset.id || '';
      if (!id || seen.has(id)) continue;
      seen.add(id);
      const ds = (el as HTMLElement).dataset;
      contacts.push({
        id,
        firstName: ds.fn || '',
        lastName: ds.ln || '',
        email: ds.email || '',
        phone: ds.phone || '',
        company: ds.company || '',
        group: ds.group || '',
        favorite: ds.favorite === 'true',
        initials: ds.initials || '',
        avatarColor: ds.color || '#6B7280',
        notes: ds.notes || '',
        address: ds.address || '',
      });
    }
    this.contacts = contacts;

    // Read groups from sidebar nav items
    const sidebar = this.shadowRoot.querySelector('cb-sidebar');
    const navItems = sidebar?.shadowRoot?.querySelectorAll('.nav-item') || [];
    const groups: string[] = [];
    for (const el of navItems) {
      const label = (el as HTMLElement).getAttribute('data-nav') || '';
      if (!['Dashboard', 'All Contacts', 'Favorites'].includes(label) && label) {
        groups.push(label);
      }
    }
    groupsStore.set(this, groups);

    if (this.contacts.length > 0) {
      this.nextId = Math.max(...this.contacts.map(c => Number(c.id) || 0)) + 1;
    }

    this.updateDerivedState();
    await this.initDB();
  }

  // --- IndexedDB ---
  private async initDB(): Promise<void> {
    return new Promise((resolve) => {
      const request = indexedDB.open('ContactBookDB', 1);
      request.onupgradeneeded = () => {
        const db = request.result;
        if (!db.objectStoreNames.contains('contacts')) {
          db.createObjectStore('contacts', { keyPath: 'id' });
        }
      };
      request.onsuccess = () => {
        this.db = request.result;
        this.loadFromDB().then(resolve);
      };
      request.onerror = () => resolve();
    });
  }

  private async loadFromDB(): Promise<void> {
    if (!this.db) return;
    return new Promise((resolve) => {
      const tx = this.db!.transaction('contacts', 'readonly');
      const store = tx.objectStore('contacts');
      const req = store.getAll();
      req.onsuccess = () => {
        const stored = req.result as Contact[];
        if (stored.length >= this.contacts.length && stored.length > 0) {
          this.contacts = stored;
          this.updateDerivedState();
        } else {
          this.saveToDB();
        }
        resolve();
      };
      req.onerror = () => resolve();
    });
  }

  private saveToDB(): void {
    if (!this.db) return;
    const tx = this.db.transaction('contacts', 'readwrite');
    const store = tx.objectStore('contacts');
    store.clear();
    for (const c of this.contacts) {
      store.put(c);
    }
  }

  // --- Derived state ---
  private updateDerivedState(): void {
    this.totalContacts = String(this.contacts.length);
    this.favoriteContacts = this.contacts.filter(c => c.favorite);
    this.totalFavorites = String(this.favoriteContacts.length);
    this.recentContacts = this.contacts.slice(-5).reverse();

    const uniqueGroups = [...new Set(this.contacts.map(c => c.group).filter(Boolean))];
    const groups = groupsStore.get(this) || [];
    if (groups.length === 0) {
      groupsStore.set(this, uniqueGroups);
    }
    this.totalGroups = String((groupsStore.get(this) || uniqueGroups).length);

    this.applyFilter();
  }

  private applyFilter(): void {
    let filtered = this.contacts;
    if (this.page === 'favorites') {
      filtered = this.favoriteContacts;
    } else if (this.page === 'group' && this.activeGroup !== 'all') {
      filtered = this.contacts.filter(c => c.group === this.activeGroup);
    } else if (this.page === 'dashboard') {
      filtered = this.recentContacts;
    }

    if (this.searchQuery) {
      const q = this.searchQuery.toLowerCase();
      filtered = filtered.filter(c =>
        c.firstName.toLowerCase().includes(q) ||
        c.lastName.toLowerCase().includes(q) ||
        c.email.toLowerCase().includes(q) ||
        c.company.toLowerCase().includes(q)
      );
    }
    this.filteredContacts = filtered;

    // Push filtered contacts to the visible cb-contact-list
    const contactList = this.shadowRoot!.querySelector(
      `.content > [class$="-page"]:not([hidden]) cb-contact-list, .dashboard cb-contact-list`
    ) as any;
    if (contactList && contactList.contacts !== undefined) {
      contactList.contacts = this.filteredContacts;
    }
  }

  // --- Event handlers ---
  onNavigate(e: CustomEvent<{ page: string; group?: string }>): void {
    this.page = e.detail.page;
    if (e.detail.group) this.activeGroup = e.detail.group;
    this.selectedContact = null;
    this.updateDerivedState();
  }

  onSearch(e: CustomEvent<{ value: string }>): void {
    this.searchQuery = e.detail.value;
    this.applyFilter();
  }

  onAddContact(): void {
    this.page = 'add';
    this.selectedContact = null;
  }

  onSelectContact(e: CustomEvent<{ id: string }>): void {
    this.selectedContact = this.contacts.find(c => c.id === e.detail.id) || null;
    if (this.selectedContact) this.page = 'detail';
  }

  onEditContact(e: CustomEvent<{ id: string }>): void {
    this.selectedContact = this.contacts.find(c => c.id === e.detail.id) || null;
    if (this.selectedContact) this.page = 'edit';
  }

  onDeleteContact(e: CustomEvent<{ id: string }>): void {
    this.contacts = this.contacts.filter(c => c.id !== e.detail.id);
    this.selectedContact = null;
    this.page = 'contacts';
    this.updateDerivedState();
    this.saveToDB();
  }

  onToggleFavorite(e: CustomEvent<{ id: string }>): void {
    const contact = this.contacts.find(c => c.id === e.detail.id);
    if (contact) {
      contact.favorite = !contact.favorite;
      this.contacts = [...this.contacts];
      if (this.selectedContact?.id === e.detail.id) {
        this.selectedContact = { ...contact };
      }
      this.updateDerivedState();
      this.saveToDB();
    }
  }

  onBack(): void {
    this.page = 'contacts';
    this.selectedContact = null;
    this.updateDerivedState();
  }

  onFormSave(e: CustomEvent<Record<string, string>>): void {
    const data = e.detail;
    if (data.id) {
      // Edit existing
      const idx = this.contacts.findIndex(c => c.id === data.id);
      if (idx >= 0) {
        const existing = this.contacts[idx];
        const updated: Contact = {
          ...existing,
          firstName: data.firstName || existing.firstName,
          lastName: data.lastName || existing.lastName,
          email: data.email || existing.email,
          phone: data.phone || existing.phone,
          company: data.company || '',
          address: data.address || '',
          group: data.group || existing.group,
          notes: data.notes || '',
          initials: (data.firstName?.[0] || existing.firstName[0] || '').toUpperCase() +
                    (data.lastName?.[0] || existing.lastName[0] || '').toUpperCase(),
        };
        this.contacts = [...this.contacts.slice(0, idx), updated, ...this.contacts.slice(idx + 1)];
        this.selectedContact = updated;
      }
    } else {
      // Add new
      const initials = ((data.firstName?.[0] || '') + (data.lastName?.[0] || '')).toUpperCase();
      const newContact: Contact = {
        id: String(this.nextId++),
        firstName: data.firstName || '',
        lastName: data.lastName || '',
        email: data.email || '',
        phone: data.phone || '',
        company: data.company || '',
        address: data.address || '',
        group: data.group || 'Other',
        notes: data.notes || '',
        favorite: false,
        initials,
        avatarColor: AVATAR_COLORS[this.contacts.length % AVATAR_COLORS.length],
      };
      this.contacts = [...this.contacts, newContact];
    }
    this.page = 'contacts';
    this.updateDerivedState();
    this.saveToDB();
  }

  onFormCancel(): void {
    if (this.selectedContact) {
      this.page = 'detail';
    } else {
      this.page = 'contacts';
    }
  }
}

CbApp.defineAsync({
  name: 'cb-app',
  templateOptions: 'defer-and-hydrate',
});

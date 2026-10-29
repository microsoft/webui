// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { WebUIElement, observable } from '@microsoft/webui-framework';
import { track } from './ntp-telemetry.js';

/** A quick link as the top-sites grid stores it. */
export interface Site {
  title: string;
  url: string;
  letter: string;
  color: string;
}

interface Row {
  index: number;
  title: string;
  host: string;
  letter: string;
  color: string;
  first: boolean;
  last: boolean;
  dragging: boolean;
  target: boolean;
}

interface Drag {
  from: number;
  to: number;
  pointer: number;
}

type Validation = { ok: true; url: string; host: string } | { ok: false; error: string };

const LIMIT = 16;
const UNDO_LIMIT = 10;
const MAX_URL = 2048;
const PALETTE = ['#0f6cbd', '#107c41', '#c239b3', '#da3b01', '#8764b8', '#00b294', '#c19c00', '#5c6b7a', '#e3008c', '#038387'];
/** Second-level labels under which a registrable name sits one level deeper, as in bbc.co.uk. */
const SHARED_SUFFIXES = new Set(['co', 'com', 'org', 'net', 'gov', 'ac', 'edu', 'ne', 'or']);
/** The quick links a new profile starts with, restored on request. */
const DEFAULT_SITES: readonly (readonly [title: string, url: string])[] = [
  ['Outlook', 'https://outlook.live.com/mail/'],
  ['Office', 'https://www.office.com/'],
  ['LinkedIn', 'https://www.linkedin.com/feed/'],
  ['Weather', 'https://www.msn.com/weather/forecast'],
  ['Xbox', 'https://www.xbox.com/play'],
  ['Booking.com', 'https://www.booking.com/'],
  ['Wikipedia', 'https://en.wikipedia.org/wiki/Main_Page'],
  ['Maps', 'https://www.bing.com/maps'],
];

function hasScheme(text: string): boolean {
  const colon = text.indexOf(':');
  if (colon <= 0) return false;
  for (let i = 0; i < colon; i++) {
    const code = text.charCodeAt(i);
    const letter = (code | 32) >= 97 && (code | 32) <= 122;
    if (!letter && (i === 0 || !((code >= 48 && code <= 57) || code === 43 || code === 45 || code === 46))) return false;
  }
  // "localhost:3000" is a host and port, not a scheme.
  return !/^\d/.test(text.slice(colon + 1));
}

/** Accept what people type in an address bar and return a normalized URL. */
export function validateUrl(raw: string): Validation {
  const text = raw.trim();
  if (!text) return { ok: false, error: 'Enter a web address.' };
  if (text.length > MAX_URL) return { ok: false, error: 'That address is too long.' };
  let url: URL;
  try {
    url = new URL(hasScheme(text) ? text : 'https://' + text);
  } catch {
    return { ok: false, error: 'That does not look like a web address.' };
  }
  if (url.protocol !== 'https:' && url.protocol !== 'http:') {
    return { ok: false, error: 'Only http and https addresses can be added.' };
  }
  if (url.username || url.password) {
    return { ok: false, error: 'Addresses that include a user name or password cannot be added.' };
  }
  const host = url.hostname.toLowerCase().replace(/\.$/, '');
  if (host !== 'localhost' && !host.includes('.')) return { ok: false, error: 'Add a domain, such as .com or .org.' };
  if (host.split('.').some(label => label.length > 63 || label.startsWith('-') || label.endsWith('-'))) {
    return { ok: false, error: 'That domain name is not valid.' };
  }
  const path = url.pathname === '/' ? '' : url.pathname.replace(/\/+$/, '');
  return { ok: true, url: url.protocol + '//' + host + (url.port ? ':' + url.port : '') + path + url.search, host };
}

/** A readable name for a host: "news.bbc.co.uk" becomes "Bbc". */
export function titleFor(host: string): string {
  const labels = host.replace(/^www\d*\./, '').split('.');
  let index = Math.max(0, labels.length - 2);
  if (labels.length > 2 && SHARED_SUFFIXES.has(labels[labels.length - 2])) index = labels.length - 3;
  const name = labels[index] || host;
  return name.charAt(0).toUpperCase() + name.slice(1);
}

/** A stable tile color for a host (FNV-1a hash). */
export function colorFor(host: string): string {
  let hash = 2166136261;
  for (let i = 0; i < host.length; i++) {
    hash ^= host.charCodeAt(i);
    hash = Math.imul(hash, 16777619);
  }
  return PALETTE[(hash >>> 0) % PALETTE.length];
}

function hostOf(url: string): string {
  try {
    return new URL(url).hostname.replace(/^www\./, '');
  } catch {
    return url;
  }
}

/** Compare sites without scheme, www, or letter case. */
function identity(url: string): string {
  return url.replace(/^https?:\/\/(www\.)?/i, '').replace(/\/+$/, '').toLowerCase();
}

/** Edit, reorder, add, and remove quick links, with undo. */
export class NtpTopSitesEditor extends WebUIElement {
  @observable sites: readonly Site[] = [];
  @observable rows: readonly Row[] = [];
  @observable newTitle = '';
  @observable newUrl = '';
  @observable error = '';
  @observable status = '';
  @observable limit = LIMIT;
  @observable full = false;
  @observable pristine = true;
  @observable noUndo = true;

  list!: HTMLElement;
  private draft: Site[] = [];
  private undoStack: Site[][] = [];
  private drag: Drag | undefined;

  protected override hydratedCallback(): void {
    this.load();
    requestAnimationFrame(() =>
      this.shadowRoot?.querySelector<HTMLInputElement>('input[name="url"]')?.focus({ preventScroll: true }));
    track('sites.edit', { count: this.draft.length });
  }

  sitesChanged(): void {
    this.load();
  }

  onTitle(event: Event): void {
    this.newTitle = (event.currentTarget as HTMLInputElement).value;
  }

  onUrl(event: Event): void {
    this.newUrl = (event.currentTarget as HTMLInputElement).value;
    if (this.error) this.error = '';
  }

  add(event: Event): void {
    event.preventDefault();
    if (this.draft.length >= LIMIT) {
      this.error = 'Remove a site before you add another.';
      return;
    }
    const result = validateUrl(this.newUrl);
    if (!result.ok) {
      this.error = result.error;
      return;
    }
    if (this.draft.some(site => identity(site.url) === identity(result.url))) {
      this.error = 'That site is already in your quick links.';
      return;
    }
    const title = this.newTitle.trim().slice(0, 40) || titleFor(result.host);
    const site: Site = { title, url: result.url, letter: title.charAt(0).toUpperCase(), color: colorFor(result.host) };
    this.edit(draft => draft.push(site), title + ' added');
    this.newTitle = '';
    this.newUrl = '';
    track('sites.add', { host: result.host });
  }

  removeSite(event: Event): void {
    const index = this.indexOf(event);
    const site = this.draft[index];
    if (!site) return;
    this.edit(draft => draft.splice(index, 1), site.title + ' removed');
    track('sites.remove', { host: hostOf(site.url) });
  }

  moveUp(event: Event): void {
    const index = this.indexOf(event);
    if (index > 0) this.move(index, index - 1);
  }

  moveDown(event: Event): void {
    const index = this.indexOf(event);
    if (index >= 0 && index < this.draft.length - 1) this.move(index, index + 1);
  }

  startDrag(event: PointerEvent): void {
    const row = (event.currentTarget as Element).closest('li');
    const from = Number(row?.getAttribute('data-index') ?? -1);
    if (from < 0) return;
    event.preventDefault();
    this.drag = { from, to: from, pointer: event.pointerId };
    this.list.setPointerCapture?.(event.pointerId);
    this.render();
  }

  onDrag(event: PointerEvent): void {
    const drag = this.drag;
    if (!drag || event.pointerId !== drag.pointer) return;
    const items = this.list.children;
    let to = items.length - 1;
    for (let i = 0; i < items.length; i++) {
      const rect = items[i].getBoundingClientRect();
      if (event.clientY < rect.top + rect.height / 2) {
        to = i;
        break;
      }
    }
    if (to !== drag.to) {
      drag.to = to;
      this.render();
    }
  }

  endDrag(event: PointerEvent): void {
    const drag = this.drag;
    if (!drag || event.pointerId !== drag.pointer) return;
    this.drag = undefined;
    if (event.type === 'pointercancel' || drag.from === drag.to) {
      this.render();
      return;
    }
    this.move(drag.from, drag.to > drag.from ? drag.to - 1 : drag.to);
  }

  /** Escape closes; Alt+Arrow moves the focused row. */
  onKeydown(event: KeyboardEvent): void {
    if (event.key === 'Escape') {
      event.preventDefault();
      this.dismiss();
      return;
    }
    if (!event.altKey || (event.key !== 'ArrowUp' && event.key !== 'ArrowDown')) return;
    const row = (this.shadowRoot?.activeElement as Element | null)?.closest('li');
    const index = Number(row?.getAttribute('data-index') ?? -1);
    const to = index + (event.key === 'ArrowUp' ? -1 : 1);
    if (index < 0 || to < 0 || to >= this.draft.length) return;
    event.preventDefault();
    this.move(index, to);
  }

  undo(): void {
    const previous = this.undoStack.pop();
    if (!previous) return;
    this.draft = previous;
    this.status = 'Change undone';
    this.pristine = this.undoStack.length === 0;
    this.render();
  }

  /** Replace the list with the default quick links; sites the user added stay at the end. */
  restoreDefaults(): void {
    const defaults = DEFAULT_SITES.map(([title, url]): Site =>
      ({ title, url, letter: title.charAt(0), color: colorFor(hostOf(url)) }));
    const known = new Set(defaults.map(site => identity(site.url)));
    const added = this.draft.filter(site => !known.has(identity(site.url)));
    this.edit(draft => {
      draft.splice(0, draft.length, ...defaults, ...added);
      draft.length = Math.min(draft.length, LIMIT);
    }, 'Default quick links restored');
    track('sites.restore', { kept: added.length });
  }

  save(): void {
    track('sites.save', { count: this.draft.length });
    this.$emit('commit', { sites: this.draft.slice() });
  }

  dismiss(): void {
    if (!this.pristine) track('sites.discard', { changes: this.undoStack.length });
    this.$emit('dismiss');
  }

  private load(): void {
    this.draft = this.sites.slice(0, LIMIT);
    this.undoStack = [];
    this.pristine = true;
    this.status = '';
    this.render();
  }

  private move(from: number, to: number): void {
    const title = this.draft[from]?.title ?? '';
    this.edit(draft => {
      const [site] = draft.splice(from, 1);
      draft.splice(to, 0, site);
    }, title + ' moved to position ' + (to + 1));
  }

  private edit(change: (draft: Site[]) => void, message: string): void {
    this.undoStack.push(this.draft.slice());
    if (this.undoStack.length > UNDO_LIMIT) this.undoStack.shift();
    const next = this.draft.slice();
    change(next);
    this.draft = next;
    this.status = message;
    this.error = '';
    this.pristine = false;
    this.render();
  }

  private render(): void {
    const last = this.draft.length - 1;
    const drag = this.drag;
    this.rows = this.draft.map((site, index) => ({
      index,
      title: site.title,
      host: hostOf(site.url),
      letter: site.letter,
      color: site.color,
      first: index === 0,
      last: index === last,
      dragging: drag?.from === index,
      target: drag !== undefined && drag.to === index && drag.from !== index,
    }));
    this.full = this.draft.length >= LIMIT;
    this.noUndo = this.undoStack.length === 0;
  }

  private indexOf(event: Event): number {
    return Number((event.currentTarget as HTMLElement).dataset['index'] ?? -1);
  }
}

NtpTopSitesEditor.define('ntp-top-sites-editor');

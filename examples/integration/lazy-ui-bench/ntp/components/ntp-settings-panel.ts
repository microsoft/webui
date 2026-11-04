// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { WebUIElement, observable } from '@microsoft/webui-framework';
import { track } from './ntp-telemetry.js';

interface Choice {
  id: string;
  label: string;
  description?: string;
}

interface Swatch {
  id: string;
  label: string;
  color: string;
}

/** Persisted page settings, schema version 3. */
export interface PageSettings {
  layout: string;
  theme: string;
  feed: string;
  rows: number;
  showBackground: boolean;
  imageOfTheDay: boolean;
  showQuickLinks: boolean;
  showWeather: boolean;
  openInNewTab: boolean;
  reduceMotion: boolean;
}

type Toggle = 'showBackground' | 'imageOfTheDay' | 'showQuickLinks' | 'showWeather' | 'openInNewTab' | 'reduceMotion';

const STORAGE_KEY = 'ntp.page-settings';
const VERSION = 3;
const HISTORY_LIMIT = 20;
const TOGGLES: readonly Toggle[] = [
  'showBackground', 'imageOfTheDay', 'showQuickLinks', 'showWeather', 'openInNewTab', 'reduceMotion',
];
const LAYOUTS: readonly Choice[] = [
  { id: 'focused', label: 'Focused', description: 'Search and quick links, with headlines below the fold.' },
  { id: 'inspirational', label: 'Inspirational', description: 'The image of the day fills the page; headlines wait for a scroll.' },
  { id: 'informational', label: 'Informational', description: 'Headlines, weather, and markets are visible right away.' },
  { id: 'custom', label: 'Custom', description: 'Choose each section yourself.' },
];
const FEEDS: readonly Choice[] = [
  { id: 'visible', label: 'Content visible' },
  { id: 'headings', label: 'Headings only' },
  { id: 'scroll', label: 'Content visible on scroll' },
  { id: 'off', label: 'Content off' },
];
const THEMES: readonly Swatch[] = [
  { id: 'system', label: 'System', color: 'linear-gradient(135deg, #f5f5f5 50%, #292929 50%)' },
  { id: 'light', label: 'Light', color: '#fafafa' },
  { id: 'dark', label: 'Dark', color: '#1f1f1f' },
  { id: 'ocean', label: 'Ocean', color: '#0f6cbd' },
  { id: 'forest', label: 'Forest', color: '#107c41' },
  { id: 'sunset', label: 'Sunset', color: '#da3b01' },
  { id: 'orchid', label: 'Orchid', color: '#8764b8' },
  { id: 'berry', label: 'Berry', color: '#c239b3' },
  { id: 'sand', label: 'Sand', color: '#c19c00' },
  { id: 'slate', label: 'Slate', color: '#5c6b7a' },
  { id: 'mint', label: 'Mint', color: '#00b294' },
  { id: 'coral', label: 'Coral', color: '#f7630c' },
];
const DEFAULTS: PageSettings = {
  layout: 'inspirational', theme: 'system', feed: 'scroll', rows: 1,
  showBackground: true, imageOfTheDay: true, showQuickLinks: true, showWeather: true,
  openInNewTab: false, reduceMotion: false,
};
/** Sections that each preset layout turns on or off. */
const PRESETS: Readonly<Record<string, Partial<PageSettings>>> = {
  focused: { showBackground: false, feed: 'headings', showWeather: false, rows: 1 },
  inspirational: { showBackground: true, imageOfTheDay: true, feed: 'scroll', rows: 1 },
  informational: { showBackground: false, feed: 'visible', showWeather: true, rows: 2 },
};
const LABELS: Readonly<Record<Toggle, string>> = {
  showBackground: 'background image', imageOfTheDay: 'image of the day', showQuickLinks: 'quick links',
  showWeather: 'weather', openInNewTab: 'links in new tabs', reduceMotion: 'reduced motion',
};

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === 'object' && value !== null && !Array.isArray(value);
}

/** Read any stored schema and upgrade it to the current one. */
function migrate(raw: unknown): Partial<PageSettings> {
  if (!isRecord(raw)) return {};
  const version = typeof raw['version'] === 'number' ? raw['version'] : 1;
  if (version === 1) {
    // Version 1 stored a layout and an on/off background flag.
    return { layout: String(raw['layout'] ?? DEFAULTS.layout), showBackground: raw['background'] !== 'off' };
  }
  if (version === 2) {
    return {
      layout: String(raw['layout'] ?? DEFAULTS.layout),
      theme: String(raw['theme'] ?? DEFAULTS.theme),
      feed: raw['feed'] === 'hidden' ? 'off' : String(raw['feed'] ?? DEFAULTS.feed),
      showQuickLinks: raw['quickLinks'] !== false,
    };
  }
  const settings = raw['settings'];
  return isRecord(settings) ? (settings as Partial<PageSettings>) : {};
}

/** Clamp untrusted values to known choices. */
function sanitize(value: Partial<PageSettings>): PageSettings {
  const pick = (id: unknown, choices: readonly { id: string }[], fallback: string): string =>
    choices.some(choice => choice.id === id) ? (id as string) : fallback;
  const result: PageSettings = {
    ...DEFAULTS,
    layout: pick(value.layout, LAYOUTS, DEFAULTS.layout),
    theme: pick(value.theme, THEMES, DEFAULTS.theme),
    feed: pick(value.feed, FEEDS, DEFAULTS.feed),
    rows: value.rows === 2 ? 2 : 1,
  };
  for (const key of TOGGLES) {
    const flag = value[key];
    if (typeof flag === 'boolean') result[key] = flag;
  }
  return result;
}

function same(left: PageSettings, right: PageSettings): boolean {
  return (Object.keys(DEFAULTS) as (keyof PageSettings)[]).every(key => left[key] === right[key]);
}

function label(choices: readonly { id: string; label: string }[], id: string): string {
  return choices.find(choice => choice.id === id)?.label ?? id;
}

/** Settings flyout: layout, theme, page sections, and content preferences. */
export class NtpSettingsPanel extends WebUIElement {
  @observable layout = DEFAULTS.layout;
  @observable theme = DEFAULTS.theme;
  @observable feed = DEFAULTS.feed;
  @observable rows = DEFAULTS.rows;
  @observable showBackground = DEFAULTS.showBackground;
  @observable imageOfTheDay = DEFAULTS.imageOfTheDay;
  @observable showQuickLinks = DEFAULTS.showQuickLinks;
  @observable showWeather = DEFAULTS.showWeather;
  @observable openInNewTab = DEFAULTS.openInNewTab;
  @observable reduceMotion = DEFAULTS.reduceMotion;
  @observable layouts = LAYOUTS;
  @observable feeds = FEEDS;
  @observable themes = THEMES;
  @observable summary = '';
  @observable status = '';
  @observable pristine = true;

  private saved: PageSettings = DEFAULTS;
  private history: PageSettings[] = [];
  private opener: Element | null = null;

  protected override hydratedCallback(): void {
    this.opener = document.activeElement;
    this.saved = this.read();
    this.assign(this.saved);
    this.describe();
    // Move focus into the panel once it has painted.
    requestAnimationFrame(() => this.focusables()[0]?.focus({ preventScroll: true }));
    track('settings.open', { layout: this.layout, theme: this.theme });
  }

  override disconnectedCallback(): void {
    if (this.opener instanceof HTMLElement && this.opener.isConnected) this.opener.focus({ preventScroll: true });
    this.opener = null;
    super.disconnectedCallback();
  }

  chooseLayout(event: Event): void {
    const value = (event.currentTarget as HTMLInputElement).value;
    // A preset layout also sets the sections it implies; custom keeps the current ones.
    this.change({ ...PRESETS[value], layout: value }, 'Layout changed to ' + label(LAYOUTS, value));
  }

  chooseTheme(event: CustomEvent<{ id: string }>): void {
    this.change({ theme: event.detail.id }, 'Theme changed to ' + label(THEMES, event.detail.id));
  }

  toggle(event: Event): void {
    const input = event.currentTarget as HTMLInputElement;
    const key = input.dataset['key'] as Toggle | undefined;
    if (!key || !TOGGLES.includes(key)) return;
    // Changing a section makes the layout custom.
    this.change({ [key]: input.checked, layout: 'custom' }, 'Turned ' + (input.checked ? 'on ' : 'off ') + LABELS[key]);
  }

  chooseFeed(event: Event): void {
    this.change({ feed: (event.currentTarget as HTMLSelectElement).value, layout: 'custom' });
  }

  chooseRows(event: Event): void {
    this.change({ rows: Number((event.currentTarget as HTMLSelectElement).value) });
  }

  /** Arrow keys move between layouts and select them, as native radio groups do. */
  onRadioKey(event: KeyboardEvent): void {
    const step = event.key === 'ArrowRight' || event.key === 'ArrowDown' ? 1
      : event.key === 'ArrowLeft' || event.key === 'ArrowUp' ? -1 : 0;
    if (!step) return;
    event.preventDefault();
    const index = LAYOUTS.findIndex(option => option.id === this.layout);
    const next = LAYOUTS[(index + step + LAYOUTS.length) % LAYOUTS.length];
    this.change({ ...PRESETS[next.id], layout: next.id });
    this.shadowRoot?.querySelector<HTMLInputElement>('input[value="' + next.id + '"]')?.focus();
  }

  /** Escape closes; Tab wraps inside the panel. */
  onKeydown(event: KeyboardEvent): void {
    if (event.key === 'Escape') {
      event.preventDefault();
      this.dismiss();
      return;
    }
    if (event.key !== 'Tab') return;
    const items = this.focusables();
    if (items.length === 0) return;
    const active = this.shadowRoot?.activeElement;
    const first = items[0];
    const last = items[items.length - 1];
    if (event.shiftKey && active === first) {
      event.preventDefault();
      last.focus();
    } else if (!event.shiftKey && active === last) {
      event.preventDefault();
      first.focus();
    }
  }

  reset(): void {
    this.change(DEFAULTS, 'Default settings restored');
    track('settings.reset');
  }

  undo(): void {
    const previous = this.history.pop();
    if (!previous) return;
    this.assign(previous);
    this.pristine = same(previous, this.saved);
    this.status = 'Change undone';
    this.describe();
  }

  save(): void {
    const settings = this.snapshot();
    try {
      localStorage.setItem(STORAGE_KEY, JSON.stringify({ version: VERSION, settings }));
    } catch {
      this.status = 'Settings apply to this page but could not be saved.';
    }
    const changed = (Object.keys(settings) as (keyof PageSettings)[]).filter(key => settings[key] !== this.saved[key]);
    this.saved = settings;
    this.history = [];
    this.pristine = true;
    track('settings.save', { changed: changed.join(',') });
    this.$emit('commit', settings);
  }

  dismiss(): void {
    if (!this.pristine) track('settings.discard', { pending: this.history.length });
    this.$emit('dismiss');
  }

  private change(patch: Partial<PageSettings>, message = ''): void {
    const current = this.snapshot();
    const next = sanitize({ ...current, ...patch });
    if (same(current, next)) return;
    this.history.push(current);
    if (this.history.length > HISTORY_LIMIT) this.history.shift();
    this.assign(next);
    this.pristine = same(next, this.saved);
    this.status = message;
    this.describe();
  }

  private read(): PageSettings {
    try {
      const stored = localStorage.getItem(STORAGE_KEY);
      return sanitize(stored ? migrate(JSON.parse(stored)) : {});
    } catch {
      return { ...DEFAULTS };
    }
  }

  private snapshot(): PageSettings {
    return {
      layout: this.layout, theme: this.theme, feed: this.feed, rows: this.rows,
      showBackground: this.showBackground, imageOfTheDay: this.imageOfTheDay,
      showQuickLinks: this.showQuickLinks, showWeather: this.showWeather,
      openInNewTab: this.openInNewTab, reduceMotion: this.reduceMotion,
    };
  }

  private assign(settings: PageSettings): void {
    this.layout = settings.layout;
    this.theme = settings.theme;
    this.feed = settings.feed;
    this.rows = settings.rows;
    this.showBackground = settings.showBackground;
    this.imageOfTheDay = settings.imageOfTheDay;
    this.showQuickLinks = settings.showQuickLinks;
    this.showWeather = settings.showWeather;
    this.openInNewTab = settings.openInNewTab;
    this.reduceMotion = settings.reduceMotion;
  }

  /** One sentence for screen readers that describes the current choices. */
  private describe(): void {
    const on = TOGGLES.filter(key => this[key]).map(key => LABELS[key]);
    this.summary = label(LAYOUTS, this.layout) + ' layout, ' + label(THEMES, this.theme).toLowerCase() + ' theme, '
      + label(FEEDS, this.feed).toLowerCase() + ', ' + this.rows + (this.rows === 1 ? ' row' : ' rows')
      + ' of quick links' + (on.length ? '; showing ' + on.join(', ') : '') + '.';
  }

  private focusables(): HTMLElement[] {
    return Array.from(this.shadowRoot?.querySelectorAll<HTMLElement>('button:not([disabled]), input, select') ?? []);
  }
}

NtpSettingsPanel.define('ntp-settings-panel');

// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

// Writes one variant of the NTP-shaped app for `ntp.mjs`.
//
// The page, owner templates, menus, and the four scripted dialogs are real
// files next to this module. Only what a real app would write differently per
// delivery model is generated: how each on-demand UI is declared in its owner's
// template, the owner methods that open it, and the entry module.
//
//   eager   plain `<if>` for all ten UIs; every component script imported statically
//   assets  the four dialogs as component assets, loaded with `import()` and created
//           with `asset.create()` on intent (hover or focus) or click; the six menus
//           as plain `<if>` in their owners
//   lazy    all ten as `<if lazy>`; the app imports the generated `lazy.webui.js`

import { mkdirSync, readFileSync, readdirSync, rmSync, writeFileSync } from 'node:fs';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';

const here = dirname(fileURLToPath(import.meta.url));
const source = (...parts) => readFileSync(join(here, ...parts), 'utf8');
const cap = id => id[0].toUpperCase() + id.slice(1);

export const VARIANTS = ['eager', 'assets', 'lazy'];

/**
 * The ten on-demand UIs. Dialogs are scripted components (about 2.5 KB of
 * Brotli-compressed JavaScript each); menus are template-heavy bodies of their
 * owners. Owners that hold one UI behind one button use intent; large owners
 * (the tile grid and the background) use demand plus `$prefetch` from the
 * specific control.
 */
export const UIS = {
  settings: {
    owner: 'ntp-settings', kind: 'dialog', shell: 'flyout', component: 'ntp-settings-panel',
    scripts: ['ntp-settings-panel', 'ntp-swatch-grid'], trigger: 'intent',
    events: { dismiss: 'closeSettings', commit: 'onSettingsCommit' },
  },
  market: {
    owner: 'ntp-market', kind: 'dialog', shell: 'modal', component: 'ntp-market-dialog',
    scripts: ['ntp-market-dialog'], trigger: 'intent',
    attrs: { current: 'marketCode' }, events: { dismiss: 'closeMarket', commit: 'onMarketCommit' },
  },
  edit: {
    owner: 'ntp-top-sites', kind: 'dialog', shell: 'modal', component: 'ntp-top-sites-editor',
    scripts: ['ntp-top-sites-editor'], trigger: 'demand',
    props: { sites: 'sites' }, events: { dismiss: 'closeEdit', commit: 'onEditCommit' },
  },
  gallery: {
    owner: 'ntp-background', kind: 'dialog', shell: 'modal', component: 'ntp-background-gallery',
    scripts: ['ntp-background-gallery', 'ntp-swatch-grid'], trigger: 'demand',
    events: { dismiss: 'closeGallery', commit: 'onGalleryCommit' },
  },
  launcher: { owner: 'ntp-app-launcher', kind: 'menu', trigger: 'intent' },
  signin: { owner: 'ntp-sign-in', kind: 'menu', trigger: 'intent' },
  suggest: { owner: 'ntp-search', kind: 'menu', trigger: 'intent', benchOpen: 'benchSuggest' },
  context: { owner: 'ntp-top-sites', kind: 'menu', trigger: 'demand', benchOpen: 'benchContext' },
  toast: { owner: 'ntp-top-sites', kind: 'menu', trigger: 'demand', noTrigger: true, benchOpen: 'benchToast' },
  bgmenu: { owner: 'ntp-background', kind: 'menu', trigger: 'demand' },
};

/** Components that only the on-demand UIs use. */
export const DEFERRED = [
  'ntp-settings-panel', 'ntp-market-dialog', 'ntp-top-sites-editor', 'ntp-background-gallery', 'ntp-swatch-grid',
];

/** Render state, as a server would pass it. */
export const STATE = {
  title: 'New tab',
  temperature: '68\u00b0',
  weatherSummary: 'Seattle: mostly sunny, high 68\u00b0',
  imageTitle: 'Mount Rainier at sunrise',
  imageCredit: '\u00a9 Kenji Watanabe / Getty Images',
  marketCode: 'en-US',
  marketLabel: 'English (United States)',
  userName: 'Avery Kim',
  userEmail: 'avery.kim@outlook.com',
  initials: 'AK',
  rewardsPoints: 4210,
  query: '',
  sites: [
    ['Outlook', 'https://outlook.live.com/mail/', '#0f6cbd'],
    ['YouTube', 'https://www.youtube.com/', '#c50f1f'],
    ['LinkedIn', 'https://www.linkedin.com/feed/', '#0a66c2'],
    ['Amazon', 'https://www.amazon.com/', '#e47911'],
    ['Weather', 'https://www.msn.com/weather/forecast', '#038387'],
    ['Booking.com', 'https://www.booking.com/', '#003580'],
    ['Wikipedia', 'https://en.wikipedia.org/wiki/Main_Page', '#5c6b7a'],
    ['Xbox', 'https://www.xbox.com/play', '#107c10'],
  ].map(([title, url, color]) => ({ title, url, letter: title[0], color })),
};

/** Owner behavior that does not depend on the delivery model. */
const OWNERS = {
  'ntp-app': { className: 'NtpApp' },
  'ntp-background': {
    className: 'NtpBackground',
    fields: `  @observable imageTitle = '';
  @observable imageCredit = '';
`,
    methods: `
  rateImage(): void {
    track('background.rate');
    this.closeBgmenu();
  }

  hideBackground(): void {
    track('background.hide');
    this.closeBgmenu();
  }

  toggleDailyImage(event: Event): void {
    track('background.daily', { on: (event.currentTarget as HTMLInputElement).checked });
  }

  toggleVideo(event: Event): void {
    track('background.video', { on: (event.currentTarget as HTMLInputElement).checked });
  }

  onGalleryCommit(event: CustomEvent<{ title: string; credit: string }>): void {
    this.imageTitle = event.detail.title;
    this.imageCredit = event.detail.credit;
    this.closeGallery();
  }
`,
  },
  'ntp-app-launcher': { className: 'NtpAppLauncher' },
  'ntp-market': {
    className: 'NtpMarket',
    fields: `  @observable marketCode = 'en-US';
  @observable marketLabel = '';
`,
    methods: `
  onMarketCommit(event: CustomEvent<{ code: string; label: string }>): void {
    this.marketCode = event.detail.code;
    this.marketLabel = event.detail.label;
    track('market.change', { code: event.detail.code });
    this.closeMarket();
  }
`,
  },
  'ntp-settings': {
    className: 'NtpSettings',
    methods: `
  onSettingsCommit(event: CustomEvent<Record<string, unknown>>): void {
    track('settings.commit', { layout: event.detail['layout'] });
    this.closeSettings();
  }
`,
  },
  'ntp-sign-in': {
    className: 'NtpSignIn',
    fields: `  @observable userName = '';
  @observable userEmail = '';
  @observable initials = '';
  @observable rewardsPoints = 0;
`,
    methods: `
  switchAccount(): void {
    track('account.switch');
    this.closeSignin();
  }

  signOut(): void {
    track('account.signout');
    this.closeSignin();
  }
`,
  },
  'ntp-search': {
    className: 'NtpSearch',
    prelude: `const SUGGESTIONS = [
  'weather', 'weather tomorrow', 'weather radar', 'weather this weekend', 'web whatsapp', 'wells fargo',
  'walmart', 'world series', 'wordle', 'wikipedia', 'news', 'netflix', 'nba scores', 'nfl standings',
  'outlook', 'office 365', 'translate', 'target', 'traffic', 'maps', 'mortgage rates', 'movies near me',
  'msn money', 'stock market', 'seattle weather', 'speed test', 'spotify', 'youtube', 'yahoo mail',
];
`,
    fields: `  @observable query = '';
  @observable suggestions: readonly { text: string; prefix: string; rest: string }[] = [];
`,
    methods: `
  onInput(event: Event): void {
    this.query = (event.currentTarget as HTMLInputElement).value;
    this.suggest();
  }

  onKeydown(event: KeyboardEvent): void {
    if (event.key === 'Escape') this.closeSuggest();
  }

  submit(event: Event): void {
    event.preventDefault();
    track('search.submit', { length: this.query.length });
    this.closeSuggest();
  }

  pickSuggestion(event: Event): void {
    this.query = (event.currentTarget as HTMLElement).dataset['text'] ?? this.query;
    this.closeSuggest();
  }

  /** The harness types "we" to open suggestions from code. */
  benchSuggest(): void {
    this.query = 'we';
    this.suggest();
  }

  private suggest(): void {
    const query = this.query.trim().toLowerCase();
    if (!query) {
      this.closeSuggest();
      return;
    }
    const matches = SUGGESTIONS.filter(text => text.startsWith(query)).slice(0, 8);
    this.suggestions = (matches.length ? matches : [query])
      .map(text => ({ text, prefix: text.slice(0, query.length), rest: text.slice(query.length) }));
    void this.openSuggest();
  }
`,
  },
  'ntp-top-sites': {
    className: 'NtpTopSites',
    prelude: `interface Site {
  title: string;
  url: string;
  letter: string;
  color: string;
}
`,
    fields: `  @observable sites: readonly Site[] = [];
  @observable menuX = 0;
  @observable menuY = 0;
  @observable menuTitle = '';
  @observable toastText = '';
  private removed: { site: Site; index: number } | undefined;
  private toastTimer = 0;
`,
    methods: `
  openMenu(event: Event): void {
    this.placeMenu(event.currentTarget as HTMLElement);
    void this.openContext();
  }

  /** The harness opens the first tile's menu from code. */
  benchContext(): void {
    const button = this.shadowRoot?.querySelector<HTMLElement>('[data-trigger="context"]');
    if (button) this.placeMenu(button);
    void this.openContext();
  }

  /** The harness shows the removal toast from code, without removing a site. */
  benchToast(): void {
    this.toastText = (this.sites[0]?.title ?? 'A site') + ' was removed from quick links';
    void this.openToast();
  }

  openInNewTab(): void {
    this.visit('tab');
  }

  openInWindow(): void {
    this.visit('window');
  }

  openInPrivate(): void {
    this.visit('inprivate');
  }

  renameSite(): void {
    track('sites.rename');
    this.closeContext();
  }

  pinSite(): void {
    const index = this.sites.findIndex(site => site.title === this.menuTitle);
    if (index > 0) {
      const next = this.sites.slice();
      next.unshift(...next.splice(index, 1));
      this.sites = next;
    }
    track('sites.pin', { index });
    this.closeContext();
  }

  removeSite(): void {
    const index = this.sites.findIndex(site => site.title === this.menuTitle);
    this.closeContext();
    if (index < 0) return;
    const site = this.sites[index];
    this.removed = { site, index };
    this.sites = this.sites.filter((_, i) => i !== index);
    this.toastText = site.title + ' was removed from quick links';
    void this.openToast();
    clearTimeout(this.toastTimer);
    this.toastTimer = window.setTimeout(() => this.closeToast(), 8000);
  }

  undoRemove(): void {
    const removed = this.removed;
    this.removed = undefined;
    if (removed) {
      const next = this.sites.slice();
      next.splice(removed.index, 0, removed.site);
      this.sites = next;
    }
    this.closeToast();
  }

  onEditCommit(event: CustomEvent<{ sites: Site[] }>): void {
    this.sites = event.detail.sites;
    track('sites.commit', { count: this.sites.length });
    this.closeEdit();
  }

  private visit(target: string): void {
    track('sites.open', { target });
    this.closeContext();
  }

  private placeMenu(button: HTMLElement): void {
    const rect = button.getBoundingClientRect();
    this.menuX = Math.round(Math.max(8, Math.min(rect.right - 248, innerWidth - 256)));
    this.menuY = Math.round(rect.bottom + 4);
    this.menuTitle = button.dataset['title'] ?? '';
  }
`,
  },
};

export const OWNER_TAGS = Object.keys(OWNERS);

function uisOf(owner) {
  return Object.keys(UIS).filter(id => UIS[id].owner === owner);
}

/** How a UI is declared inside its owner's template. */
function region(id, variant, triggers) {
  const ui = UIS[id];
  const trigger = triggers?.[id] ?? ui.trigger;
  const lazy = variant !== 'lazy' ? '' : trigger === 'intent' ? ' lazy' : ` lazy="${trigger}"`;
  if (ui.kind === 'menu') return `<if condition="${id}Open"${lazy}>${source('menus', `${id}.html`).trim()}</if>`;
  if (variant === 'assets') return `<div class="${ui.shell === 'flyout' ? 'flyout' : 'slot'}" w-ref="{${id}Slot}"></div>`;
  const attributes = [
    ...Object.entries(ui.attrs ?? {}).map(([name, field]) => `${name}="{{${field}}}"`),
    ...Object.entries(ui.props ?? {}).map(([name, field]) => `:${name}="{{${field}}}"`),
    ...Object.entries(ui.events ?? {}).map(([name, handler]) => `@${name}="{${handler}(${name === 'commit' ? 'e' : ''})}"`),
  ].join(' ');
  const element = `<${ui.component} ${attributes}></${ui.component}>`;
  return `<if condition="${id}Open"${lazy}>${ui.shell === 'flyout' ? `<div class="flyout" data-ui="${id}">${element}</div>` : element}</if>`;
}

/** Event bindings that start loading a UI from its own control. */
function prefetchAttributes(id, variant) {
  const ui = UIS[id];
  if (variant === 'lazy' && ui.trigger === 'demand') {
    return `@pointerenter="{$prefetch('${id}Open')}" @focus="{$prefetch('${id}Open')}"`;
  }
  if (variant === 'assets' && ui.kind === 'dialog') {
    return `@pointerover="{preload${cap(id)}()}" @focusin="{preload${cap(id)}()}"`;
  }
  return '';
}

function ownerTemplate(tag, variant, triggers) {
  return source('owners', `${tag}.html`)
    .replace(/<!--ui:(\w+)-->/g, (_, id) => region(id, variant, triggers))
    .replace(/ data-prefetch="(\w+)"/g, (_, id) => {
      const handlers = prefetchAttributes(id, variant);
      return handlers ? ` ${handlers}` : '';
    });
}

/** Owner methods that open, close, and preload one UI. */
function uiMethods(id, variant) {
  const ui = UIS[id];
  const C = cap(id);
  const flag = `this.${id}Open`;
  const modal = ui.kind === 'dialog' && ui.shell === 'modal';
  const lines = [];
  const toggle = `
  toggle${C}(): void {
    if (${flag}) this.close${C}();
    else void this.open${C}();
  }
`;
  if (ui.kind === 'dialog' && variant === 'assets') {
    const wire = [
      ...Object.entries(ui.attrs ?? {}).map(([name, field]) => `    element.setAttribute('${name}', String(this.${field}));`),
      ...Object.entries(ui.props ?? {}).map(([name, field]) => `    (element as unknown as Record<string, unknown>)['${name}'] = this.${field};`),
      ...Object.entries(ui.events ?? {}).map(([name, handler]) => name === 'commit'
        ? `    element.addEventListener('${name}', event => this.${handler}(event as CustomEvent));`
        : `    element.addEventListener('${name}', () => this.${handler}());`),
    ].join('\n');
    lines.push(`
  preload${C}(): void {
    preloadComponentAssetStyles('${ui.component}');
    void load${C}().then(asset => asset.preload()).catch(() => undefined);
  }

  async open${C}(): Promise<void> {
    ${flag} = true;
    preloadComponentAssetStyles('${ui.component}');
    const element = await (await load${C}()).create();
    if (!${flag}) return;
${wire}
    this.${id}Slot.replaceChildren(element);${modal ? `
    if (!this.${id}Dialog.open) this.${id}Dialog.showModal();` : ''}
  }

  close${C}(): void {${modal ? `
    if (this.${id}Dialog.open) this.${id}Dialog.close();` : ''}
    ${flag} = false;
    this.${id}Slot.replaceChildren();
  }
`);
    if (modal) {
      lines.push(`
  on${C}Closed(): void {
    if (${flag}) this.close${C}();
  }
`);
    } else lines.push(toggle);
    return lines.join('');
  }
  if (modal) {
    lines.push(variant === 'lazy' ? `
  async open${C}(): Promise<void> {
    ${flag} = true;
    // showModal() moves focus into the dialog, so its body must exist first.
    await this.$rendered();
    if (${flag} && !this.${id}Dialog.open) this.${id}Dialog.showModal();
  }
` : `
  open${C}(): void {
    ${flag} = true;
    this.$flushUpdates();
    if (!this.${id}Dialog.open) this.${id}Dialog.showModal();
  }
`);
    lines.push(`
  close${C}(): void {
    if (this.${id}Dialog.open) this.${id}Dialog.close();
    ${flag} = false;
  }

  on${C}Closed(): void {
    ${flag} = false;
  }
`);
    return lines.join('');
  }
  lines.push(`
  open${C}(): void {
    ${flag} = true;
  }

  close${C}(): void {
    ${flag} = false;
  }
`, toggle);
  return lines.join('');
}

function ownerScript(tag, variant) {
  const spec = OWNERS[tag];
  const ids = uisOf(tag);
  const assetDialogs = variant === 'assets' ? ids.filter(id => UIS[id].kind === 'dialog') : [];
  let text = `import { WebUIElement, observable } from '@microsoft/webui-framework';
import { bench } from './bench-client.js';
import { track } from './ntp-telemetry.js';
`;
  if (assetDialogs.length > 0) {
    text += `import { defineComponentAsset, preloadComponentAssetStyles } from '@microsoft/webui-framework/component-asset-runtime.js';\n`;
  }
  text += '\n';
  for (const id of assetDialogs) {
    text += `const load${cap(id)} = () => import('./${UIS[id].component}.asset.js').then(module => defineComponentAsset(module.default));\n`;
  }
  text += spec.prelude ?? '';
  text += `\nexport class ${spec.className} extends WebUIElement {\n`;
  for (const id of ids) text += `  @observable ${id}Open = false;\n`;
  text += spec.fields ?? '';
  for (const id of ids) {
    const ui = UIS[id];
    if (ui.kind === 'dialog' && ui.shell === 'modal') text += `  ${id}Dialog!: HTMLDialogElement;\n`;
    if (ui.kind === 'dialog' && variant === 'assets') text += `  ${id}Slot!: HTMLElement;\n`;
  }
  text += `
  protected override hydratedCallback(): void {
    bench.register(this);
    track('owner.hydrated', { tag: '${tag}' });
  }
`;
  for (const id of ids) text += uiMethods(id, variant);
  text += spec.methods ?? '';
  return `${text}}\n\n${spec.className}.define('${tag}');\n`;
}

/** Selector paths that must be visible and styled for a UI to count as open. */
function sentinels(id) {
  const ui = UIS[id];
  if (ui.kind === 'menu') return [[`[data-ui="${id}"] [data-ready]`]];
  const paths = [[ui.component, '[data-ready]']];
  for (const tag of ui.scripts.slice(1)) paths.push([ui.component, tag, '[data-ready]']);
  return paths;
}

/** The harness's view of every UI; `regions` maps UI ids to lazy region identities. */
export function benchConfig(variant, regions = {}) {
  const uis = {};
  for (const [id, ui] of Object.entries(UIS)) {
    const C = cap(id);
    uis[id] = {
      owner: ui.owner,
      ...(ui.noTrigger ? {} : { trigger: `[data-trigger="${id}"]` }),
      sentinels: sentinels(id),
      tags: ui.kind === 'dialog' ? ui.scripts : [],
      ...(regions[id] ? { region: regions[id] } : {}),
      ...(variant === 'assets' && ui.kind === 'dialog' ? { template: ui.component, preload: `preload${C}` } : {}),
      cond: `${id}Open`,
      open: ui.benchOpen ?? `open${C}`,
      close: `close${C}`,
    };
  }
  return { variant, owners: OWNER_TAGS, uis };
}

/** Find the UI each lazy pack delivers from its region template. */
export function regionsOf(packs) {
  const regions = {};
  for (const pack of packs) {
    const body = JSON.parse(pack.content).templates?.[pack.region]?.h ?? '';
    for (const [id, ui] of Object.entries(UIS)) {
      if (body.includes(`data-ui="${id}"`) || (ui.component && body.includes(`<${ui.component}`))) regions[id] = pack.region;
    }
  }
  return regions;
}

/**
 * Write the app sources for a variant, except the entry module. `triggers`
 * overrides lazy triggers by UI id, for the priority and prerender probes.
 */
export function writeApp(dir, variant, triggers) {
  rmSync(dir, { recursive: true, force: true });
  mkdirSync(dir, { recursive: true });
  const write = (name, text) => writeFileSync(join(dir, name), text);
  write('index.html', source('index.html'));
  write('bench-client.ts', source('bench-client.ts'));
  for (const name of readdirSync(join(here, 'components'))) write(name, source('components', name));
  for (const tag of OWNER_TAGS) {
    write(`${tag}.html`, ownerTemplate(tag, variant, triggers));
    write(`${tag}.css`, source('owners', `${tag}.css`));
    write(`${tag}.ts`, ownerScript(tag, variant));
  }
  if (variant === 'assets') {
    // One chunk per dialog: its generated asset and the scripts of the custom elements it renders.
    for (const ui of Object.values(UIS).filter(entry => entry.kind === 'dialog')) {
      write(`${ui.component}.asset.ts`, `import asset from './.webui/${ui.component}.webui.js';\n`
        + ui.scripts.map(tag => `import './${tag}.js';\n`).join('') + 'export default asset;\n');
    }
  }
}

/** Write the entry module once the harness configuration is known. */
export function writeMain(dir, variant, config) {
  let text = `import { startRuntime } from '@microsoft/webui-framework/runtime.js';
import '@microsoft/webui-framework/styles.js';
import '@microsoft/webui-framework/template-cache.js';
`;
  if (variant === 'lazy') text += `import './.webui/lazy.webui.js';\n`;
  text += `import { bench } from './bench-client.js';\n`;
  for (const tag of OWNER_TAGS) text += `import './${tag}.js';\n`;
  if (variant === 'eager') for (const tag of DEFERRED) text += `import './${tag}.js';\n`;
  text += `\nbench.configure(${JSON.stringify(config)});\nstartRuntime();\n`;
  writeFileSync(join(dir, 'main.ts'), text);
}

/** Component asset roots for the assets variant. */
export const ASSET_ROOTS = Object.values(UIS).filter(ui => ui.kind === 'dialog').map(ui => ui.component);

/** Entry modules of each UI's code, for size attribution: scripts (lazy) or asset wrappers (assets). */
export function uiEntries(id, variant) {
  const ui = UIS[id];
  if (ui.kind !== 'dialog' || variant === 'eager') return [];
  return variant === 'assets' ? [`${ui.component}.asset.ts`] : ui.scripts.map(tag => `${tag}.ts`);
}

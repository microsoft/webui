// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { strict as assert } from 'node:assert';
import { after, beforeEach, test } from 'node:test';
import type { CondBinding, TemplateInstance } from './element/types.js';
import type { CompiledConditionalMeta, LazyTrigger, TemplateMeta } from './template-types.js';

const restore: [string, PropertyDescriptor | undefined][] = [];

function setGlobal(name: string, value: unknown): void {
  restore.push([name, Object.getOwnPropertyDescriptor(globalThis, name)]);
  Object.defineProperty(globalThis, name, { configurable: true, writable: true, value });
}

type Listener = (event: Event) => void;

const requests: { url: string; priority?: string }[] = [];
const idleCallbacks: (() => void)[] = [];
const reported: unknown[] = [];
const loaded: string[] = [];
const listeners = new Map<string, Set<{ listener: Listener; once: boolean }>>();
const page = {
  readyState: 'complete' as DocumentReadyState,
  prerendering: false,
  activeElement: null as unknown,
  querySelector: () => null,
  getElementById: () => null,
  addEventListener(type: string, listener: Listener, options?: { once?: boolean }): void {
    let set = listeners.get(type);
    if (!set) listeners.set(type, set = new Set());
    set.add({ listener, once: options?.once === true });
  },
};
const browser = Object.assign(new EventTarget(), {
  __webui: { inventory: '' } as NonNullable<Window['__webui']>,
});
setGlobal('HTMLElement', class extends EventTarget {});
setGlobal('window', browser);
setGlobal('document', page);
setGlobal('customElements', { get: (): undefined => undefined });
// Every pack is missing: speculative failures must stay silent.
setGlobal('fetch', async (url: string, init?: { priority?: string }): Promise<Response> => {
  requests.push({ url, priority: init?.priority });
  return new Response('missing', { status: 404 });
});
setGlobal('requestIdleCallback', (callback: () => void): number => idleCallbacks.push(callback));
setGlobal('reportError', (error: unknown): void => { reported.push(error); });

after(() => {
  for (const [name, descriptor] of restore.reverse()) {
    if (descriptor) Object.defineProperty(globalThis, name, descriptor);
    else Reflect.deleteProperty(globalThis, name);
  }
});

/** Dispatch a document event to its listeners, dropping `once` listeners. */
function fire(type: string, event = new Event(type)): void {
  const set = listeners.get(type);
  if (!set) return;
  for (const entry of [...set]) {
    if (entry.once) set.delete(entry);
    entry.listener(event);
  }
}

function region(owner: string, ordinal = 0): string {
  return `${owner}::lazy:@${ordinal}`;
}

function pack(owner: string, ordinal = 0): string {
  return `_webui/packs/${owner}.${ordinal}.json`;
}

function condition(id: string, trigger: LazyTrigger, paths: string[] = ['open']): CompiledConditionalMeta {
  return [[0, paths], id, [0, 0], trigger];
}

/** A component template with lazy regions in its root block. */
function owner(tag: string, regions: [ordinal: number, trigger: LazyTrigger, paths?: string[]][]): TemplateMeta {
  const lp: Record<string, string> = {};
  for (const [ordinal] of regions) lp[region(tag, ordinal)] = pack(tag, ordinal);
  return {
    v: 3,
    h: '',
    c: regions.map(([ordinal, trigger, paths]) => condition(region(tag, ordinal), trigger, paths)),
    lp,
  };
}

const tick = (): Promise<void> => new Promise(resolve => setTimeout(resolve, 0));

function runIdle(): void {
  for (const callback of idleCallbacks.splice(0)) callback();
}

const { defineLazyScripts } = await import('./lazy-entry.js');
const runtime = await import('./lazy-runtime.js');
const { WebUIElement } = await import('./element.js');

interface Component extends EventTarget {
  $meta?: TemplateMeta;
  $prefetch(path?: string): void;
  $toggleCond(binding: CondBinding): void;
}

function component(tag: string, meta: TemplateMeta): Component {
  const Host = class extends WebUIElement {};
  const host = new Host() as unknown as Component;
  Object.defineProperty(host, 'localName', { value: tag });
  host.$meta = meta;
  return host;
}

beforeEach(() => {
  requests.length = 0;
  reported.length = 0;
  page.readyState = 'complete';
  page.prerendering = false;
  page.activeElement = null;
});

test('eager and idle prefetch always use low priority', async () => {
  page.readyState = 'interactive';
  runtime.blockOwner(owner('early-owner', [[0, 3], [1, 2]]));
  await tick();
  assert.deepEqual(requests, [{ url: pack('early-owner', 0), priority: 'low' }]);
  runIdle();
  assert.equal(requests.length, 1, 'idle prefetch waits for the window load event');
  browser.dispatchEvent(new Event('load'));
  page.readyState = 'complete';
  runIdle();
  assert.deepEqual(requests.at(-1), { url: pack('early-owner', 1), priority: 'low' });
  await tick();
  assert.deepEqual(reported, [], 'speculative failures are silent');
});

test('intent prefetch stays low until the window has loaded', async () => {
  const hover = (tag: string): Event => ({
    composedPath: () => [{ localName: 'button' }, { localName: tag }],
  }) as unknown as Event;
  page.readyState = 'interactive';
  runtime.blockOwner(owner('intent-early', [[0, 1]]));
  fire('pointerover', hover('intent-early'));
  assert.deepEqual(requests, [{ url: pack('intent-early'), priority: 'low' }]);
  page.readyState = 'complete';
  runtime.blockOwner(owner('intent-late', [[0, 1]]));
  fire('pointerover', hover('intent-late'));
  assert.deepEqual(requests.at(-1), { url: pack('intent-late'), priority: 'high' });
});

test('$prefetch() covers every region of the template, including demand, at load-aware priority', async () => {
  const meta = owner('prefetch-all', [[0, 4], [1, 1], [2, 2]]);
  meta.b = [{ h: '', c: [condition(region('prefetch-all', 3), 4)] }];
  meta.lp = { ...meta.lp, [region('prefetch-all', 3)]: pack('prefetch-all', 3) };
  const host = component('prefetch-all', meta);
  page.readyState = 'interactive';
  host.$prefetch();
  assert.deepEqual(requests.map(request => request.url), [0, 1, 2, 3].map(ordinal => pack('prefetch-all', ordinal)));
  assert.ok(requests.every(request => request.priority === 'low'), 'before load');
  await tick();
  requests.length = 0;
  page.readyState = 'complete';
  host.$prefetch();
  assert.equal(requests.length, 4);
  assert.ok(requests.every(request => request.priority === 'high'), 'after load');
  await tick();
});

test('$prefetch(path) selects regions whose condition reads that path or a path below it', async () => {
  const host = component('prefetch-path', owner('prefetch-path', [
    [0, 4, ['settings.open']],
    [1, 4, ['settingsOpen']],
    [2, 2, ['settings']],
    [3, 1, ['user', 'settings.theme.dark']],
  ]));
  host.$prefetch('settings');
  assert.deepEqual(requests.map(request => request.url), [0, 2, 3].map(ordinal => pack('prefetch-path', ordinal)));
  await tick();
  requests.length = 0;
  host.$prefetch('settings.open');
  assert.deepEqual(requests.map(request => request.url), [pack('prefetch-path', 0)]);
  await tick();
  requests.length = 0;
  host.$prefetch('missing');
  assert.deepEqual(requests, []);
});

test('a compiled event binding passes its string literal to $prefetch', () => {
  const host = component('prefetch-event', owner('prefetch-event', [[0, 4, ['settingsOpen']], [1, 4, ['other']]]));
  const bound = new Map<string, Listener>();
  const button = {
    nodeType: 1,
    addEventListener(type: string, listener: Listener): void { bound.set(type, listener); },
    removeEventListener(): void {},
  };
  const instance = { texts: [], attrs: [], conds: [], repeats: [] } as unknown as TemplateInstance;
  (host as unknown as {
    $wireEvents(instance: TemplateInstance, root: null, meta: TemplateMeta, resolve: () => unknown): void;
  }).$wireEvents(instance, null, {
    v: 3,
    h: '<button></button>',
    eg: [
      ['pointerenter', [['$prefetch', [['s', 'settingsOpen']], 1]]],
      ['focus', [['$prefetch', [['s', 'settingsOpen']], 1]]],
    ],
  }, () => button);
  bound.get('pointerenter')!(new Event('pointerenter'));
  assert.deepEqual(requests, [{ url: pack('prefetch-event', 0), priority: 'high' }]);
  assert.equal(typeof bound.get('focus'), 'function');
});

test('speculative loads wait for prerendering to end while activation does not', async () => {
  page.prerendering = true;
  runtime.blockOwner(owner('prerender-owner', [[0, 3], [1, 2], [2, 1]]));
  const host = component('prerender-owner', owner('prerender-owner', [[3, 4]]));
  await tick();
  runIdle();
  fire('pointerover', {
    composedPath: () => [{ localName: 'prerender-owner' }],
  } as unknown as Event);
  host.$prefetch();
  assert.equal(requests.length, 0, 'nothing speculative loads while prerendering');

  const c = {
    blockIndex: region('prerender-owner', 4),
    owner: { conds: [] },
    instance: null,
    condition: [() => true],
    anchor: { parentNode: {} },
  } as unknown as CondBinding;
  (c.owner.conds as CondBinding[]).push(c);
  host.$meta!.lp = { ...host.$meta!.lp, [region('prerender-owner', 4)]: pack('prerender-owner', 4) };
  page.readyState = 'interactive';
  host.$toggleCond(c);
  assert.deepEqual(requests, [{ url: pack('prerender-owner', 4), priority: 'high' }], 'activation never waits');

  page.prerendering = false;
  page.readyState = 'complete';
  fire('prerenderingchange');
  assert.deepEqual(requests.slice(1).sort((a, b) => a.url.localeCompare(b.url)), [
    { url: pack('prerender-owner', 0), priority: 'low' },
    { url: pack('prerender-owner', 1), priority: 'low' },
    { url: pack('prerender-owner', 2), priority: 'high' },
    { url: pack('prerender-owner', 3), priority: 'high' },
  ]);
  await tick();
  assert.equal(reported.length, 1, 'only the failed activation is reported');
});

test('pack requests resolve against the runtime asset base while the raw path stays the key', async () => {
  const site = page as typeof page & { baseURI?: string };
  site.baseURI = 'https://site.example/news/';
  browser.__webui.assetBase = 'https://cdn.example/assets/';
  try {
    const host = component('asset-owner', owner('asset-owner', [[0, 4]]));
    host.$prefetch();
    host.$prefetch();
    assert.deepEqual(requests, [{ url: 'https://cdn.example/assets/_webui/packs/asset-owner.0.json', priority: 'high' }]);
    assert.equal(runtime.installing(pack('asset-owner')), true, 'the compiled path keys the shared load');
    await tick();
  } finally {
    delete site.baseURI;
    delete browser.__webui.assetBase;
  }
});

test('every prefetch trigger starts the region scripts with its pack', async () => {
  const tags = ['eager', 'idle', 'intent', 'demand'];
  for (let i = 0; i < tags.length; i++) {
    defineLazyScripts({
      [region('scripted-owner', i)]: {
        [`scripted-${tags[i]}`]: () => { loaded.push(tags[i]); return Promise.reject(new Error('offline')); },
      },
    });
  }
  runtime.blockOwner(owner('scripted-owner', [[0, 3], [1, 2], [2, 1], [3, 4]]));
  await tick();
  assert.deepEqual(loaded, ['eager']);
  runIdle();
  assert.deepEqual(loaded, ['eager', 'idle']);
  fire('pointerover', { composedPath: () => [{ localName: 'scripted-owner' }] } as unknown as Event);
  assert.deepEqual(loaded, ['eager', 'idle', 'intent']);
  component('scripted-owner', owner('scripted-owner', [[3, 4]])).$prefetch();
  assert.deepEqual(loaded, ['eager', 'idle', 'intent', 'demand']);
  await tick();
  assert.equal(requests.length, 4, 'each pack loads beside its scripts');
  assert.deepEqual(reported, [], 'speculative script failures are silent');
});

/** A closed, live binding whose condition holds, for the given region. */
function openBinding(id: string): CondBinding {
  const c = {
    blockIndex: id,
    owner: { conds: [] },
    instance: null,
    condition: [() => true],
    anchor: { parentNode: {} },
  } as unknown as CondBinding;
  (c.owner.conds as CondBinding[]).push(c);
  return c;
}

test('speculative loads before the window loads import region scripts only once it has', async () => {
  const imported: string[] = [];
  defineLazyScripts({
    [region('deferred-owner', 0)]: { 'deferred-eager': async () => { imported.push('eager'); } },
    [region('deferred-owner', 1)]: { 'deferred-intent': async () => { imported.push('intent'); } },
  });
  page.readyState = 'interactive';
  runtime.blockOwner(owner('deferred-owner', [[0, 3], [1, 1]]));
  await tick();
  fire('pointerover', { composedPath: () => [{ localName: 'deferred-owner' }] } as unknown as Event);
  assert.deepEqual(requests, [
    { url: pack('deferred-owner', 0), priority: 'low' },
    { url: pack('deferred-owner', 1), priority: 'low' },
  ], 'packs load at once');
  assert.deepEqual(imported, [], 'a bundler import takes no priority, so scripts wait');
  page.readyState = 'complete';
  browser.dispatchEvent(new Event('load'));
  assert.deepEqual(imported, ['eager', 'intent']);
  await tick();
});

test('after the window loads, every load of a region preloads its link stylesheets beside the pack', async () => {
  const preloads: [readonly string[], RequestPriority | undefined][] = [];
  const target = browser as unknown as Window;
  target.__webuiPreloadStyles = (hrefs, priority) => { preloads.push([hrefs, priority]); return undefined; };
  try {
    const meta = owner('styled-owner', [[0, 3], [1, 1], [2, 4]]);
    meta.lc = {
      [region('styled-owner', 0)]: ['eager.css'],
      [region('styled-owner', 1)]: ['intent.css', 'shared.css'],
      [region('styled-owner', 2)]: ['demand.css'],
    };
    runtime.blockOwner(meta);
    await tick();
    fire('pointerover', { composedPath: () => [{ localName: 'styled-owner' }] } as unknown as Event);
    assert.deepEqual(preloads, [[['eager.css'], 'low'], [['intent.css', 'shared.css'], 'high']]);

    // Activation, through the element, uses the template that names the region.
    const host = component('styled-owner', meta);
    host.$toggleCond(openBinding(region('styled-owner', 2)));
    assert.deepEqual(preloads.at(-1), [['demand.css'], 'high']);

    component('plain-owner', owner('plain-owner', [[0, 4]])).$prefetch();
    assert.equal(preloads.length, 3, 'a region without stylesheet paths preloads none');
    assert.equal(requests.length, 4);
    await tick();
  } finally {
    delete target.__webuiPreloadStyles;
  }
});

test('focus already inside an intent owner prefetches its regions as they arm', async () => {
  // Autofocus put focus on an input in <app-search>'s shadow root, which sits
  // in <app-shell>'s shadow root, before the runtime listened for focus.
  const input = { closest: () => null, shadowRoot: null };
  const search = { closest: (tag: string) => (tag === 'app-search' ? search : null), shadowRoot: { activeElement: input } };
  page.activeElement = { closest: () => null, shadowRoot: { activeElement: search } };
  page.readyState = 'interactive';
  runtime.blockOwner(owner('app-search', [[0, 1]]));
  assert.deepEqual(requests, [{ url: pack('app-search'), priority: 'low' }], 'focus counts as intent');
  runtime.blockOwner(owner('app-other', [[0, 1]]));
  assert.equal(requests.length, 1, 'other owners still wait for intent');

  page.prerendering = true;
  runtime.blockOwner(owner('app-search', [[1, 1]]));
  assert.equal(requests.length, 1, 'it waits for prerendering like any speculative load');
  page.prerendering = false;
  fire('prerenderingchange');
  assert.deepEqual(requests.at(-1), { url: pack('app-search', 1), priority: 'low' });
  await tick();
  assert.deepEqual(reported, []);
});

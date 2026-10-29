// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { strict as assert } from 'node:assert';
import { after, test } from 'node:test';
import type { CondBinding, TemplateInstance } from './element/types.js';
import type { TemplateMeta } from './template-types.js';

const restore: [string, PropertyDescriptor | undefined][] = [];

function setGlobal(name: string, value: unknown): void {
  restore.push([name, Object.getOwnPropertyDescriptor(globalThis, name)]);
  Object.defineProperty(globalThis, name, { configurable: true, writable: true, value });
}

const requests: { url: string; priority?: string }[] = [];
const responses = new Map<string, PromiseWithResolvers<Response>>();
const idleCallbacks: (() => void)[] = [];
const reported: unknown[] = [];
const defined = new Map<string, unknown>();
const page = {
  readyState: 'complete',
  prerendering: false,
  querySelector: () => null,
  getElementById: () => null,
  addEventListener(): void {},
};
const browser = Object.assign(new EventTarget(), {
  __webui: { inventory: '' } as NonNullable<Window['__webui']>,
});
setGlobal('HTMLElement', class extends EventTarget {});
setGlobal('window', browser);
setGlobal('document', page);
setGlobal('customElements', {
  get: (tag: string) => defined.get(tag),
  define(tag: string, ctor: unknown): void { defined.set(tag, ctor); },
});
setGlobal('fetch', (url: string, init?: { priority?: string }): Promise<Response> => {
  requests.push({ url, priority: init?.priority });
  return response(url).promise;
});
setGlobal('requestIdleCallback', (callback: () => void): number => idleCallbacks.push(callback));
setGlobal('reportError', (error: unknown): void => { reported.push(error); });

after(() => {
  for (const [name, descriptor] of restore.reverse()) {
    if (descriptor) Object.defineProperty(globalThis, name, descriptor);
    else Reflect.deleteProperty(globalThis, name);
  }
});

function response(url: string): PromiseWithResolvers<Response> {
  let pending = responses.get(url);
  if (!pending) responses.set(url, pending = Promise.withResolvers<Response>());
  return pending;
}

function region(ordinal: number): string {
  return `test-shell::lazy:@${ordinal}`;
}

function template(id: string): TemplateMeta {
  return { v: 3, f: id, o: 'test-shell', e: 1, h: '<p></p>' };
}

/** Answer a pack request with the region's template. */
function serve(id: string, url: string): void {
  response(url).resolve(new Response(JSON.stringify({
    type: 'webui-pack',
    version: 1,
    fragment: id,
    inventory: '',
    styleInventory: '',
    required: [id],
    bits: {},
    templates: { [id]: template(id) },
  })));
}

/** A loader whose module defines its tag when the test releases it. */
function deferredLoader(tag: string): { load(): Promise<unknown>; calls: number; release(): void; fail(error: Error): void } {
  let pending = Promise.withResolvers<void>();
  const loader = {
    calls: 0,
    load(): Promise<unknown> {
      loader.calls++;
      return pending.promise.then(() => { defined.set(tag, class {}); });
    },
    release(): void { pending.resolve(); },
    fail(error: Error): void {
      pending.reject(error);
      pending = Promise.withResolvers<void>();
    },
  };
  return loader;
}

const tick = (): Promise<void> => new Promise(resolve => setTimeout(resolve, 0));

const contract = await import('./lazy-contract.js');
const { TemplateElement } = await import('./template-element.js');
// Stand-ins for the core: a toggle records which custom elements were defined
// when a body mounted, and a component mount hydrates the given bodies or runs
// nested mounts.
const mounts: { region: unknown; defined: string[] }[] = [];
const core = TemplateElement.prototype as unknown as Record<string, unknown>;
core.$toggleCond = function (c: CondBinding): void {
  mounts.push({ region: c.blockIndex, defined: [...defined.keys()] });
  c.instance = {} as TemplateInstance;
};
core.$mount = function (...steps: (TemplateMeta | (() => void))[]): void {
  for (const step of steps) {
    if (typeof step === 'function') step();
    else contract.lazyBlockOwner(step);
  }
};
const { defineLazyScripts } = await import('./lazy-entry.js');
const scripts = await import('./lazy-scripts.js');

interface Shell extends EventTarget {
  $meta?: TemplateMeta;
  $toggleCond(binding: CondBinding): void;
  $mount(...steps: (TemplateMeta | (() => void))[]): void;
  $rendered(): Promise<void>;
  $prefetch(path?: string): void;
}

/** A component host whose template names the packs of the given regions. */
function shell(packs: Record<string, string>): Shell {
  const Owner = class extends TemplateElement {};
  const host = new Owner() as unknown as Shell;
  Object.defineProperty(host, 'localName', { value: 'test-shell' });
  host.$meta = { v: 3, h: '', lp: packs };
  return host;
}

/** A closed region binding in a live owner instance. */
function binding(id: string, open = true): CondBinding {
  const owner = { conds: [] as CondBinding[] } as unknown as TemplateInstance;
  const c = {
    blockIndex: id,
    owner,
    instance: null,
    condition: [() => open],
    anchor: { parentNode: {} },
  } as unknown as CondBinding;
  owner.conds.push(c);
  return c;
}

test('generated loaders merge by region and load each undefined custom element once', async () => {
  const a = deferredLoader('merge-a');
  const b = deferredLoader('merge-b');
  const c = deferredLoader('merge-c');
  let duplicate = 0;
  defined.set('merge-defined', class {});
  defineLazyScripts({ [region(90)]: { 'merge-a': a.load, 'merge-b': b.load } });
  defineLazyScripts({
    [region(90)]: { 'merge-defined': () => Promise.reject(new Error('already defined')) },
    [region(91)]: { 'merge-b': () => { duplicate++; return Promise.resolve(); }, 'merge-c': c.load },
  });
  const first = scripts.loadScripts(region(90));
  const second = scripts.loadScripts(region(91));
  assert.ok(first && second);
  assert.deepEqual([a.calls, b.calls, c.calls, duplicate], [1, 1, 1, 0], 'a tag in flight loads once across regions');
  a.release();
  b.release();
  c.fail(new Error('chunk failed'));
  await first;
  await assert.rejects(second, /chunk failed/);
  assert.equal(scripts.loadScripts(region(90)), undefined, 'loaded regions have nothing left to load');
  const retry = scripts.loadScripts(region(91));
  assert.equal(c.calls, 2, 'a failed loader is forgotten so the next request retries it');
  c.release();
  await retry;
  assert.equal(scripts.loadScripts(region(91)), undefined);
});

test('activation loads the pack and scripts in parallel and mounts only after both', async () => {
  const id = region(0);
  const url = '_webui/packs/test-shell.0000.json';
  const panel = deferredLoader('shell-panel');
  defineLazyScripts({ [id]: { 'shell-panel': panel.load } });
  const host = shell({ [id]: url });
  const c = binding(id);
  requests.length = 0;
  mounts.length = 0;
  host.$toggleCond(c);
  assert.deepEqual(requests, [{ url, priority: 'high' }]);
  assert.equal(panel.calls, 1, 'scripts start with the pack request');
  let settled = false;
  const done = host.$rendered().then(() => { settled = true; });
  serve(id, url);
  await tick();
  assert.ok(browser.__webui.templates?.[id], 'the pack installed');
  assert.equal(mounts.length, 0, 'the body waits for its custom elements');
  assert.equal(settled, false, '$rendered() waits for scripts');
  panel.release();
  await done;
  assert.equal(mounts.length, 1);
  assert.ok(mounts[0].defined.includes('shell-panel'), 'custom elements are defined before the body mounts');
});

test('a mount waits for its pack even when its scripts arrive first', async () => {
  const id = region(1);
  const url = '_webui/packs/test-shell.0001.json';
  const card = deferredLoader('shell-card');
  defineLazyScripts({ [id]: { 'shell-card': card.load } });
  const host = shell({ [id]: url });
  mounts.length = 0;
  host.$toggleCond(binding(id));
  card.release();
  await tick();
  assert.equal(mounts.length, 0, 'scripts alone do not mount the body');
  serve(id, url);
  await host.$rendered();
  assert.equal(mounts.length, 1);
});

test('a failed script rejects $rendered, raises lazyerror, and a retry loads only the scripts', async () => {
  const id = region(2);
  const url = '_webui/packs/test-shell.0002.json';
  const chart = deferredLoader('shell-chart');
  defineLazyScripts({ [id]: { 'shell-chart': chart.load } });
  const host = shell({ [id]: url });
  const events: CustomEvent<{ error: unknown }>[] = [];
  host.addEventListener('lazyerror', event => { events.push(event as CustomEvent<{ error: unknown }>); });
  const unhandled: unknown[] = [];
  const onUnhandled = (reason: unknown): void => { unhandled.push(reason); };
  process.on('unhandledRejection', onUnhandled);
  try {
    requests.length = 0;
    mounts.length = 0;
    reported.length = 0;
    host.$toggleCond(binding(id));
    serve(id, url);
    const failure = new Error('chunk failed');
    chart.fail(failure);
    await tick();
    await tick();
    assert.deepEqual(unhandled, [], 'internal failures are never unhandled rejections');
    assert.equal(events.length, 1);
    assert.equal(events[0].detail.error, failure);
    assert.deepEqual([events[0].bubbles, events[0].composed, events[0].cancelable], [true, true, true]);
    assert.deepEqual(reported, [failure], 'an unhandled lazyerror is reported');

    // A component that presents the error itself cancels the report.
    host.addEventListener('lazyerror', event => event.preventDefault());
    const retry = host.$rendered();
    assert.equal(chart.calls, 2, '$rendered() retries the scripts');
    chart.fail(new Error('chunk failed again'));
    await assert.rejects(retry, /chunk failed again/);
    assert.equal(events.length, 2);
    assert.equal(reported.length, 1, 'a cancelled lazyerror is not reported');

    const retried = host.$rendered();
    assert.equal(chart.calls, 3);
    chart.release();
    await retried;
    assert.equal(mounts.length, 1);
    assert.deepEqual(requests, [{ url, priority: 'high' }], 'the installed pack is not requested again');
    await tick();
    assert.deepEqual(unhandled, []);
  } finally {
    process.off('unhandledRejection', onUnhandled);
  }
});

test('reportError falls back to console.error', async () => {
  const id = region(3);
  const url = '_webui/packs/test-shell.0003.json';
  const host = shell({ [id]: url });
  const logged: unknown[] = [];
  const previousError = console.error;
  setGlobal('reportError', undefined);
  console.error = (...args: unknown[]) => { logged.push(args[0]); };
  try {
    host.$toggleCond(binding(id));
    response(url).resolve(new Response('unavailable', { status: 503 }));
    await tick();
    assert.equal(logged.length, 1);
    assert.match(String(logged[0]), /failed with HTTP 503/);
  } finally {
    console.error = previousError;
    setGlobal('reportError', (error: unknown): void => { reported.push(error); });
  }
});

test('a body that hydrates open loads its scripts at once, even while prerendering', async () => {
  const id = region(4);
  const url = '_webui/packs/test-shell.0004.json';
  browser.__webui.templates![id] = template(id);
  const menu = deferredLoader('shell-menu');
  defineLazyScripts({ [id]: { 'shell-menu': menu.load } });
  const host = shell({ [id]: url });
  page.prerendering = true;
  try {
    host.$mount(template(id));
  } finally {
    page.prerendering = false;
  }
  assert.equal(menu.calls, 1, 'hydration starts the scripts at activation priority');

  // Reopening before the scripts arrive must not insert undefined elements.
  requests.length = 0;
  mounts.length = 0;
  host.$toggleCond(binding(id));
  await tick();
  assert.equal(mounts.length, 0);
  assert.deepEqual(requests, [], 'the body templates are already present');
  let settled = false;
  const done = host.$rendered().then(() => { settled = true; });
  await tick();
  assert.equal(settled, false, '$rendered() waits for the hydrated body scripts');
  menu.release();
  await done;
  assert.equal(mounts.length, 1);
  assert.ok(mounts[0].defined.includes('shell-menu'));
  assert.equal(menu.calls, 1);
});

test('hydrated body script failures raise lazyerror and $rendered() retries them', async () => {
  const id = region(5);
  browser.__webui.templates![id] = template(id);
  const toast = deferredLoader('shell-toast');
  defineLazyScripts({ [id]: { 'shell-toast': toast.load } });
  const host = shell({ [id]: '_webui/packs/test-shell.0005.json' });
  const errors: unknown[] = [];
  host.addEventListener('lazyerror', event => {
    errors.push((event as CustomEvent<{ error: unknown }>).detail.error);
    event.preventDefault();
  });
  host.$mount(template(id));
  const failure = new Error('toast chunk failed');
  toast.fail(failure);
  await tick();
  assert.deepEqual(errors, [failure]);
  const retry = host.$rendered();
  assert.equal(toast.calls, 2, '$rendered() retries the failed scripts');
  toast.fail(new Error('toast chunk failed again'));
  await assert.rejects(retry, /toast chunk failed again/);
  assert.equal(errors.length, 2);
  const retried = host.$rendered();
  assert.equal(toast.calls, 3);
  toast.release();
  await retried;
  assert.ok(defined.has('shell-toast'));
});

test('hydrated bodies belong to the host that is mounting them, also across nested mounts', async () => {
  const outerId = region(6);
  const innerId = region(7);
  const strayId = region(8);
  for (const id of [outerId, innerId, strayId]) browser.__webui.templates![id] = template(id);
  const outer = deferredLoader('shell-outer');
  const inner = deferredLoader('shell-inner');
  const stray = deferredLoader('shell-stray');
  defineLazyScripts({
    [outerId]: { 'shell-outer': outer.load },
    [innerId]: { 'shell-inner': inner.load },
    [strayId]: { 'shell-stray': stray.load },
  });
  const parent = shell({});
  const child = shell({});
  const failures = new Map<EventTarget, unknown[]>([[parent, []], [child, []]]);
  for (const host of [parent, child]) {
    host.addEventListener('lazyerror', event => {
      failures.get(host)!.push((event as CustomEvent<{ error: Error }>).detail.error.message);
      event.preventDefault();
    });
  }
  parent.$mount(() => child.$mount(template(innerId)), template(outerId));
  assert.throws(() => parent.$mount(() => { throw new Error('mount failed'); }), /mount failed/);
  contract.lazyBlockOwner(template(strayId));
  assert.deepEqual([outer.calls, inner.calls, stray.calls], [1, 1, 0], 'only mounting hosts start body scripts');
  outer.fail(new Error('outer failed'));
  inner.fail(new Error('inner failed'));
  await tick();
  assert.deepEqual(failures.get(parent), ['outer failed']);
  assert.deepEqual(failures.get(child), ['inner failed']);
});

test('development warns once about lazy-only tags that a static import defined at startup', async () => {
  const warnings: string[] = [];
  const previousWarn = console.warn;
  console.warn = (...args: unknown[]) => { warnings.push(String(args[0])); };
  try {
    defined.set('startup-chart', class {});
    defined.set('startup-menu', class {});
    defineLazyScripts({
      [region(20)]: { 'startup-chart': () => Promise.resolve() },
      [region(21)]: { 'startup-menu': () => Promise.resolve() },
    });
    assert.equal(scripts.loadScripts(region(21)), undefined, 'a defined tag needs no load');
    for (const callback of idleCallbacks.splice(0)) callback();
    assert.equal(warnings.filter(message => message.includes('startup-')).length, 1);
    assert.match(warnings.find(message => message.includes('startup-'))!, /<startup-chart> is only used inside lazy regions but was defined at startup; remove its static import/);
    defineLazyScripts({ [region(22)]: { 'startup-chart': () => Promise.resolve() } });
    for (const callback of idleCallbacks.splice(0)) callback();
    assert.equal(warnings.filter(message => message.includes('startup-')).length, 1, 'each tag warns once');
  } finally {
    console.warn = previousWarn;
  }
});

test('development reports a lazy-only component whose static import still waits for its template', () => {
  const warnings: string[] = [];
  const previousWarn = console.warn;
  console.warn = (...args: unknown[]) => { warnings.push(String(args[0])); };
  try {
    // Statically imported: the class is declared, but WebUI defines the custom
    // element only once its template arrives with the region's pack.
    class PendingPanel extends TemplateElement {}
    PendingPanel.define('pending-panel');
    assert.equal(customElements.get('pending-panel'), undefined);
    defineLazyScripts({ [region(23)]: { 'pending-panel': () => Promise.resolve() } });
    for (const callback of idleCallbacks.splice(0)) callback();
    assert.deepEqual(warnings.filter(message => message.includes('pending-panel')), [
      '[WebUI] <pending-panel> is only used inside lazy regions but was defined at startup; remove its static import so it loads with its region.',
    ]);
  } finally {
    console.warn = previousWarn;
  }
});

test('a speculative load before the window loads defers scripts that an activation imports at once', async () => {
  const id = region(30);
  const url = '_webui/packs/test-shell.0030.json';
  const panel = deferredLoader('deferred-panel');
  defineLazyScripts({ [id]: { 'deferred-panel': panel.load } });
  const host = shell({ [id]: url });
  host.$meta!.c = [[[0, ['open']], id, [0, 0], 4]];
  page.readyState = 'interactive';
  try {
    requests.length = 0;
    mounts.length = 0;
    host.$prefetch();
    assert.deepEqual(requests, [{ url, priority: 'low' }]);
    assert.equal(panel.calls, 0, 'the scripts wait for the window load event');
    serve(id, url);
    await tick();
    assert.ok(browser.__webui.templates?.[id], 'the pack installed');

    host.$toggleCond(binding(id));
    assert.equal(panel.calls, 1, 'activation imports the scripts at once');
    let settled = false;
    const done = host.$rendered().then(() => { settled = true; });
    await tick();
    assert.equal(settled, false, '$rendered() waits for the scripts');
    panel.release();
    await done;
    assert.equal(mounts.length, 1);
    page.readyState = 'complete';
    browser.dispatchEvent(new Event('load'));
    assert.equal(panel.calls, 1, 'the deferred import finds the loaded element');
  } finally {
    page.readyState = 'complete';
  }
});

/**
 * Record calls to the stylesheet hooks that `styles.js` installs. Before the
 * window loads they hold low-priority work and return nothing to wait for.
 */
function captureStyles(prepared: () => Promise<void> | undefined = () => undefined): {
  calls: [hook: string, list: readonly string[], priority: RequestPriority | undefined][];
  restore(): void;
} {
  const calls: [string, readonly string[], RequestPriority | undefined][] = [];
  const target = browser as unknown as Window;
  target.__webuiPreloadStyles = (hrefs, priority) => { calls.push(['preload', hrefs, priority]); return undefined; };
  target.__webuiPrepareResources = (ids, priority) => { calls.push(['prepare', ids, priority]); return prepared(); };
  return {
    calls,
    restore(): void {
      delete target.__webuiPreloadStyles;
      delete target.__webuiPrepareResources;
    },
  };
}

/** A shell whose template holds one demand region with link stylesheets. */
function styledShell(id: string, url: string): Shell {
  const host = shell({ [id]: url });
  host.$meta!.c = [[[0, ['open']], id, [0, 0], 4]];
  host.$meta!.lc = { [id]: ['region.css'] };
  return host;
}

test('a speculative load hands its stylesheets to styles.js at low priority and waits for nothing it holds', async () => {
  const id = region(50);
  const url = '_webui/packs/test-shell.0050.json';
  const styles = captureStyles();
  const host = styledShell(id, url);
  page.readyState = 'interactive';
  try {
    requests.length = 0;
    mounts.length = 0;
    host.$prefetch();
    assert.deepEqual(requests, [{ url, priority: 'low' }]);
    assert.deepEqual(styles.calls, [['preload', ['region.css'], 'low']], 'the stylesheets go beside the pack');
    serve(id, url);
    await tick();
    assert.deepEqual(styles.calls.at(-1), ['prepare', [id], 'low']);
    host.$toggleCond(binding(id));
    assert.equal(mounts.length, 1, 'the region mounts without waiting for its stylesheets');
  } finally {
    page.readyState = 'complete';
    styles.restore();
  }
});

test('an activation starts its stylesheets at once, also when it joins a speculative pack', async () => {
  const id = region(51);
  const url = '_webui/packs/test-shell.0051.json';
  const styles = captureStyles();
  const host = styledShell(id, url);
  page.readyState = 'interactive';
  try {
    mounts.length = 0;
    host.$prefetch();
    host.$toggleCond(binding(id));
    assert.deepEqual(styles.calls, [
      ['preload', ['region.css'], 'low'],
      ['preload', ['region.css'], 'high'],
    ], 'they start at activation priority while the pack loads');
    serve(id, url);
    await host.$rendered();
    assert.equal(mounts.length, 1);
  } finally {
    page.readyState = 'complete';
    styles.restore();
  }
});

test('an install waits for the stylesheets that styles.js prepares', async () => {
  const id = region(52);
  const url = '_webui/packs/test-shell.0052.json';
  const prepared = Promise.withResolvers<void>();
  const styles = captureStyles(() => prepared.promise);
  const host = styledShell(id, url);
  try {
    mounts.length = 0;
    host.$toggleCond(binding(id));
    serve(id, url);
    await tick();
    assert.deepEqual(styles.calls.at(-1), ['prepare', [id], 'high']);
    assert.equal(mounts.length, 0, 'the region waits for its stylesheets');
    prepared.resolve();
    await host.$rendered();
    assert.equal(mounts.length, 1);
  } finally {
    styles.restore();
  }
});

test('an activation after a speculative install starts its stylesheets with its scripts', async () => {
  const id = region(53);
  const url = '_webui/packs/test-shell.0053.json';
  const card = deferredLoader('styled-card');
  defineLazyScripts({ [id]: { 'styled-card': card.load } });
  const styles = captureStyles();
  const host = styledShell(id, url);
  page.readyState = 'interactive';
  try {
    mounts.length = 0;
    host.$prefetch();
    serve(id, url);
    await tick();
    assert.ok(browser.__webui.templates?.[id], 'the speculative load installed the pack');
    assert.equal(card.calls, 0, 'its scripts wait for the window load event');
    host.$prefetch();
    const preloads = (): number => styles.calls.filter(([hook]) => hook === 'preload').length;
    assert.equal(preloads(), 1, 'another speculative load starts nothing for an installed pack');

    host.$toggleCond(binding(id));
    assert.equal(card.calls, 1);
    assert.deepEqual(styles.calls.at(-1), ['preload', ['region.css'], 'high'], 'the stylesheets start with the scripts');
    assert.equal(mounts.length, 0, 'the body waits for its scripts');
    card.release();
    await host.$rendered();
    assert.equal(mounts.length, 1);
    assert.equal(preloads(), 2);
  } finally {
    page.readyState = 'complete';
    styles.restore();
  }
});

test('development warns once when a pack registers scripted components nothing loads or declares', async () => {
  const warnings: string[] = [];
  const previousWarn = console.warn;
  console.warn = (...args: unknown[]) => { warnings.push(String(args[0])); };
  try {
    class PackDeclared extends TemplateElement {}
    PackDeclared.define('pack-declared');
    defined.set('pack-defined', class {});
    const component = (extra: Partial<TemplateMeta> = {}): TemplateMeta => ({ v: 3, h: '<p></p>', ...extra });
    const install = async (id: string, url: string, templates: Record<string, TemplateMeta>): Promise<void> => {
      const host = shell({ [id]: url });
      host.$toggleCond(binding(id));
      response(url).resolve(new Response(JSON.stringify({
        type: 'webui-pack',
        version: 1,
        fragment: id,
        inventory: '',
        styleInventory: '',
        required: [id],
        bits: {},
        templates: { [id]: template(id), ...templates },
      })));
      await host.$rendered();
    };
    defineLazyScripts({ [region(40)]: { 'pack-loaded': () => Promise.resolve() } });
    await install(region(40), '_webui/packs/test-shell.0040.json', {
      'pack-loaded': component(),
      'pack-declared': component(),
      'pack-defined': component(),
      'pack-scriptless': component({ th: 1 }),
      'pack-missing': component(),
      'pack-forgotten': component(),
    });
    const expected = '[WebUI] Lazy region "test-shell::lazy:@40" registered <pack-missing>, <pack-forgotten>, '
      + 'but no script defines them, so they render empty. '
      + 'Import the generated lazy.webui.js instead of @microsoft/webui-framework/lazy.js.';
    assert.deepEqual(warnings, [expected]);
    await install(region(41), '_webui/packs/test-shell.0041.json', { 'pack-unloaded': component() });
    assert.deepEqual(warnings, [expected], 'it warns once');
  } finally {
    console.warn = previousWarn;
  }
});

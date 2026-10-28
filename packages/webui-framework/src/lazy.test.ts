// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { strict as assert } from 'node:assert';
import { after, test } from 'node:test';
import type { CondBinding, TemplateInstance } from './element/types.js';
import type { CompiledConditionalMeta, TemplateMeta } from './template-types.js';

interface FakeScript {
  nonce: string;
  textContent: string;
  remove(): void;
}

const requests: { url: string; priority?: string }[] = [];
const responses = new Map<string, () => Response>();
const idleCallbacks: (() => void)[] = [];
const scripts: FakeScript[] = [];
const documentListeners = new Map<string, (event: Event) => void>();
const restore: [string, PropertyDescriptor | undefined][] = [];

function setGlobal(name: string, value: unknown): void {
  restore.push([name, Object.getOwnPropertyDescriptor(globalThis, name)]);
  Object.defineProperty(globalThis, name, { configurable: true, writable: true, value });
}

const browser = Object.assign(new EventTarget(), {
  __webui: { nonce: 'pack-nonce', inventory: '10' } as NonNullable<Window['__webui']>,
});
// `lazy.js` extends `TemplateElement extends HTMLElement` at module scope.
setGlobal('HTMLElement', class {});
setGlobal('window', browser);
setGlobal('document', {
  readyState: 'complete',
  head: {
    appendChild(script: FakeScript): void {
      scripts.push(script);
      new Function(script.textContent)();
    },
  },
  createElement: (): FakeScript => ({ nonce: '', textContent: '', remove() {} }),
  querySelector: () => null,
  getElementById: () => null,
  addEventListener(type: string, listener: (event: Event) => void): void {
    documentListeners.set(type, listener);
  },
});
setGlobal('fetch', async (url: string, init?: { priority?: string; signal?: unknown }): Promise<Response> => {
  if (!(init?.signal instanceof AbortSignal)) throw new Error('pack requests need a deadline');
  requests.push({ url, priority: init?.priority });
  return responses.get(url)?.() ?? new Response('missing', { status: 404 });
});
setGlobal('requestIdleCallback', (callback: () => void): number => idleCallbacks.push(callback));

after(() => {
  for (const [name, descriptor] of restore.reverse()) {
    if (descriptor) Object.defineProperty(globalThis, name, descriptor);
    else Reflect.deleteProperty(globalThis, name);
  }
});

function region(owner: string, ordinal = 0): string {
  return `${owner}::lazy:@${ordinal}`;
}

function pack(owner: string, extra: Record<string, unknown> = {}): Record<string, unknown> {
  const id = region(owner);
  return {
    type: 'webui-pack',
    version: 1,
    fragment: id,
    inventory: '02',
    styleInventory: '',
    required: [id],
    bits: { [id]: 1 },
    templates: { [id]: { v: 3, f: id, o: owner, e: 1, h: '<p></p>', c: [[[0, ['open']], -1, [0, 0]]], b: [{ h: '<i></i>' }] } },
    templateFunctions: { [id]: '[function(v,s,t){return t(v("open",s))}]' },
    ...extra,
  };
}

function serve(url: string, body: Record<string, unknown>): void {
  responses.set(url, () => new Response(JSON.stringify(body), {
    headers: { 'Content-Type': 'application/json' },
  }));
}

function runIdle(): void {
  for (const callback of idleCallbacks.splice(0)) callback();
}

const tick = (): Promise<void> => new Promise(resolve => setTimeout(resolve, 0));

async function until(check: () => unknown): Promise<void> {
  for (let i = 0; i < 100 && !check(); i++) await tick();
  assert.ok(check());
}

/** Hold a pack's install after its templates are published, until `release()`. */
function holdResources(): { release(): void; restore(): void } {
  let release!: () => void;
  const prepared = new Promise<void>(resolve => { release = resolve; });
  const target = browser as unknown as Window;
  target.__webuiPrepareResources = () => prepared;
  return { release, restore: () => { delete target.__webuiPrepareResources; } };
}

/** A compiled conditional entry for a lazy region with a prefetch trigger. */
function lazyCondition(id: string, trigger: 1 | 2 | 3 | 4): CompiledConditionalMeta {
  return [[0, ['open']], id, [0, 0], trigger];
}

/** A binding in a live owner instance unless `detached`. */
function bindingFor(id: string, detached = false): CondBinding {
  const owner = { conds: [] as CondBinding[] } as unknown as TemplateInstance;
  const binding = { blockIndex: id, owner, instance: null } as unknown as CondBinding;
  if (!detached) owner.conds.push(binding);
  return binding;
}

const registry = (): Record<string, TemplateMeta> => browser.__webui.templates ?? {};
let runtime: typeof import('./lazy-runtime.js');

test('the core needs lazy.js and the runtime owns hostless fragments and their blocks', async () => {
  const contract = await import('./lazy-contract.js');
  assert.equal(contract.findLazyFragment('x::lazy:@0', 'x'), undefined);
  assert.equal(contract.lazyBlockOwner({ h: '' }), undefined);
  assert.throws(() => contract.requireLazyFragment('x::lazy:@0', 'x'), /webui-framework\/lazy\.js/);
  await import('./lazy-entry.js');
  runtime = await import('./lazy-runtime.js');
  const id = region('app-shell');
  const meta: TemplateMeta = { v: 3, f: id, o: 'app-shell', e: 1, h: '<p></p>', b: [{ h: '<span></span>' }] };
  (browser.__webui.templates ??= {})[id] = meta;
  assert.equal(contract.requireLazyFragment(id, 'app-shell'), meta);
  assert.equal(contract.lazyBlockOwner(meta), meta);
  assert.equal(contract.lazyBlockOwner(meta.b![0]), meta);
  assert.throws(() => contract.requireLazyFragment(id, 'other-owner'), /does not belong to <other-owner>/);
  assert.throws(() => contract.requireLazyFragment('missing::lazy:@0', 'app-shell'), /is missing/);
  registry()['wrong-identity'] = meta;
  assert.throws(() => contract.requireLazyFragment('wrong-identity', 'app-shell'), /Invalid fragment/);
});

test('a pack loads once, runs closures with the document nonce, and merges its inventory', async () => {
  const id = region('pack-owner');
  const url = '_webui/packs/pack-owner.0001.json';
  serve(url, pack('pack-owner'));
  await Promise.all([runtime.prefetchPacks({ [id]: url }), runtime.prefetchPacks({ [id]: url })]);
  assert.deepEqual(requests, [{ url, priority: 'auto' }]);
  const template = registry()[id];
  const condition = template.c![0][0][0] as unknown as (
    v: (path: string) => unknown, s: unknown, t: (value: unknown) => boolean,
  ) => boolean;
  assert.equal(typeof condition, 'function');
  assert.equal(condition(path => path === 'open', undefined, Boolean), true);
  assert.equal(scripts.at(-1)?.nonce, 'pack-nonce');
  assert.equal(browser.__webui.inventory, '12');
  await runtime.prefetchPacks({ [id]: url });
  assert.equal(requests.length, 1, 'a registered region needs no request');
});

test('invalid or failed packs reject with guidance and the next load retries', async () => {
  const id = region('retry-owner');
  const url = '_webui/packs/retry-owner.0002.json';
  requests.length = 0;
  await assert.rejects(runtime.prefetchPacks({ [id]: url }), /failed with HTTP 404/);
  serve(url, pack('retry-owner', { fragment: region('other-owner') }));
  await assert.rejects(runtime.prefetchPacks({ [id]: url }), /is not the template pack for lazy region/);
  serve(url, pack('retry-owner', { version: 2 }));
  await assert.rejects(runtime.prefetchPacks({ [id]: url }), /Serve packs from the same build/);
  serve(url, pack('retry-owner'));
  await runtime.prefetchPacks({ [id]: url });
  assert.equal(requests.length, 4);
  assert.ok(registry()[id]);
});

test('templates the page already owns are neither replaced nor re-executed', async () => {
  const id = region('shared-owner');
  const url = '_webui/packs/shared-owner.0003.json';
  const owned: TemplateMeta = { v: 3, h: '<b>page</b>' };
  registry()['shared-card'] = owned;
  const scriptCount = scripts.length;
  const base = pack('shared-owner');
  serve(url, {
    ...base,
    templates: { ...base.templates as object, 'shared-card': { v: 3, h: '<b>pack</b>' } },
    templateFunctions: {
      ...base.templateFunctions as object,
      'shared-card': '[function(){throw new Error("page closures must not run twice")}]',
    },
  });
  await runtime.prefetchPacks({ [id]: url });
  assert.equal(registry()['shared-card'], owned);
  assert.ok(registry()[id]);
  assert.equal(scripts.length, scriptCount + 1, 'only missing templates contribute closures');
  assert.ok(!scripts.at(-1)!.textContent.includes('shared-card'));
});

test('templates only the template cache holds are registered again before their bits merge', async () => {
  const id = region('cached-owner');
  const url = '_webui/packs/cached-owner.0014.json';
  const cached: TemplateMeta = { v: 3, h: '<b>cached</b>' };
  const retained: string[] = [];
  (browser as unknown as Window).__webuiRetainTemplate = (tag: string): TemplateMeta | undefined => {
    if (tag !== 'cached-card') return undefined;
    retained.push(tag);
    registry()[tag] = cached;
    return cached;
  };
  const base = pack('cached-owner');
  serve(url, {
    ...base,
    templates: { ...base.templates as object, 'cached-card': { v: 3, h: '<b>pack</b>' } },
    templateFunctions: {
      ...base.templateFunctions as object,
      'cached-card': '[function(){throw new Error("cached closures must not run twice")}]',
    },
  });
  await runtime.prefetchPacks({ [id]: url });
  assert.deepEqual(retained, ['cached-card']);
  assert.equal(registry()['cached-card'], cached, 'the page strongly owns the template its bit names');
});

test('regions no binding shows are released at idle and revived while their metadata lives', () => {
  const id = region('pack-owner');
  const template = registry()[id];
  const binding = bindingFor(id);
  binding.instance = {} as TemplateInstance;
  runtime.opened(id, binding);
  runtime.opened(id, binding);
  runtime.disposed();
  runIdle();
  assert.equal(registry()[id], template, 'a shown region is kept');

  binding.instance = null;
  runtime.disposed();
  binding.instance = {} as TemplateInstance;
  runIdle();
  assert.equal(registry()[id], template, 'a region reopened before idle is kept');

  // Disposing an owner clears the instance of every binding it contains.
  binding.instance = null;
  runtime.disposed();
  runIdle();
  assert.equal(registry()[id], undefined);
  assert.equal(browser.__webui.inventory, '10', 'release clears exactly the region bit');
  assert.equal(runtime.resolve(id, 'pack-owner'), undefined, 'a lookup does not pin a released region');
  assert.equal(registry()[id], undefined);
  assert.equal(runtime.resolve(id, 'pack-owner', true), template, 'mounting revives it');
  assert.equal(registry()[id], template);
  assert.equal(browser.__webui.inventory, '12');
});

test('regions the page or a prefetch registered are never tracked or released', () => {
  const id = region('app-shell');
  const binding = bindingFor(id);
  binding.instance = {} as TemplateInstance;
  runtime.opened(id, binding);
  binding.instance = null;
  const before = idleCallbacks.length;
  runtime.disposed();
  assert.equal(idleCallbacks.length, before, 'nothing to check');
  assert.ok(registry()[id]);
});

test('intent prefetches the waiting regions of every owner on the event path once', async () => {
  const id = region('intent-owner');
  const nested = region('intent-owner', 1);
  const demand = region('intent-owner', 2);
  const url = '_webui/packs/intent-owner.0004.json';
  const nestedUrl = '_webui/packs/intent-owner.0005.json';
  serve(url, pack('intent-owner'));
  requests.length = 0;
  const template: TemplateMeta = {
    v: 3,
    h: '',
    c: [lazyCondition(id, 1), lazyCondition(demand, 4)],
    b: [{ h: '', c: [lazyCondition(nested, 1)] }],
    lp: { [id]: url, [nested]: nestedUrl, [demand]: '_webui/packs/intent-owner.0006.json' },
  };
  assert.equal(runtime.blockOwner(template), undefined, 'a component template has no hostless owner');
  runtime.blockOwner(template);
  assert.deepEqual([...documentListeners.keys()], ['pointerover', 'focusin', 'touchstart', 'pointerdown']);
  const hover = (owner: string): Event => ({
    composedPath: () => [{ localName: 'span' }, { localName: owner }, browser],
  }) as unknown as Event;
  documentListeners.get('pointerover')!(hover('other-owner'));
  documentListeners.get('pointerover')!(hover('intent-owner'));
  documentListeners.get('pointerdown')!(hover('intent-owner'));
  await new Promise(resolve => setTimeout(resolve, 0));
  assert.deepEqual(requests, [{ url, priority: 'high' }], 'only rendered blocks arm, demand never does');
});

test('idle and load triggers prefetch after startup unless the user saves data', async () => {
  const idleId = region('idle-owner');
  const loadId = region('load-owner');
  const idleUrl = '_webui/packs/idle-owner.0007.json';
  const loadUrl = '_webui/packs/load-owner.0008.json';
  serve(idleUrl, pack('idle-owner'));
  serve(loadUrl, pack('load-owner'));
  requests.length = 0;
  runtime.blockOwner({
    v: 3,
    h: '',
    c: [lazyCondition(idleId, 2), lazyCondition(loadId, 3)],
    lp: { [idleId]: idleUrl, [loadId]: loadUrl },
  } as TemplateMeta);
  assert.equal(requests.length, 0);
  await new Promise(resolve => setTimeout(resolve, 0));
  assert.deepEqual(requests, [{ url: loadUrl, priority: 'auto' }]);
  runIdle();
  assert.deepEqual(requests.at(-1), { url: idleUrl, priority: 'low' });
  await new Promise(resolve => setTimeout(resolve, 0));

  // Show, close, and release the load region; rendering its block again must not re-pin it.
  const binding = bindingFor(loadId);
  binding.instance = {} as TemplateInstance;
  runtime.opened(loadId, binding);
  binding.instance = null;
  runtime.disposed();
  runIdle();
  assert.equal(registry()[loadId], undefined);
  requests.length = 0;
  runtime.blockOwner({
    v: 3,
    h: '',
    c: [lazyCondition(loadId, 3)],
    lp: { [loadId]: loadUrl },
  } as TemplateMeta);
  await new Promise(resolve => setTimeout(resolve, 0));
  runIdle();
  assert.equal(requests.length, 0);
  assert.equal(registry()[loadId], undefined, 'speculative prefetch runs once per region');

  Object.defineProperty(navigator, 'connection', { configurable: true, value: { saveData: true } });
  try {
    requests.length = 0;
    const saver = region('saver-owner');
    const saverMore = region('saver-owner', 1);
    runtime.blockOwner({
      v: 3,
      h: '',
      c: [lazyCondition(saver, 2), lazyCondition(saverMore, 3)],
      lp: { [saver]: '_webui/packs/saver-owner.0009.json', [saverMore]: '_webui/packs/saver-owner.0010.json' },
    } as TemplateMeta);
    await new Promise(resolve => setTimeout(resolve, 0));
    runIdle();
    assert.equal(requests.length, 0);
  } finally {
    Reflect.deleteProperty(navigator, 'connection');
  }
});

test('activation loads at high priority, re-enters its host, and rendered retries failures', async () => {
  const id = region('active-owner');
  const url = '_webui/packs/active-owner.0011.json';
  const binding = bindingFor(id);
  const mounted: CondBinding[] = [];
  const host = Object.assign(new EventTarget(), {
    $toggleCond(c: CondBinding): void {
      if (runtime.resolve(id, 'active-owner', true)) mounted.push(c);
      else runtime.activate(host, c, url);
    },
  }) as unknown as import('./lazy-runtime.js').LazyHost;
  requests.length = 0;
  host.$toggleCond(binding);
  host.$toggleCond(binding);
  await assert.rejects(runtime.rendered(host), /failed with HTTP 404/);
  assert.deepEqual(mounted, []);
  serve(url, pack('active-owner'));
  await runtime.rendered(host);
  assert.deepEqual(mounted, [binding]);
  assert.deepEqual(requests, [{ url, priority: 'high' }, { url, priority: 'high' }]);
  await runtime.rendered(host);
  assert.equal(mounted.length, 1, 'settled hosts have nothing left to render');
});

test('a pack that is still installing is joined rather than treated as ready', async () => {
  const id = region('ready-owner');
  const url = '_webui/packs/ready-owner.0015.json';
  serve(url, pack('ready-owner'));
  const resources = holdResources();
  try {
    requests.length = 0;
    const first = runtime.prefetchPacks({ [id]: url });
    await until(() => registry()[id]);
    let joined = false;
    const second = runtime.prefetchPacks({ [id]: url }).then(() => { joined = true; });
    await tick();
    assert.equal(joined, false, 'published templates still wait for their resources');
    resources.release();
    await Promise.all([first, second]);
    assert.equal(joined, true);
    assert.equal(requests.length, 1);
  } finally {
    resources.restore();
  }
});

test('the element waits for a closed region condition and never mounts into a disposed owner', async () => {
  const { TemplateElement } = await import('./template-element.js');
  const id = region('glue-owner');
  const url = '_webui/packs/glue-owner.0012.json';
  serve(url, pack('glue-owner'));
  let open = false;
  const host = Object.assign(Object.create(TemplateElement.prototype) as object, {
    localName: 'glue-owner',
    $meta: { v: 3, h: '', lp: { [id]: url } } as TemplateMeta,
  }) as import('./lazy-runtime.js').LazyHost & {
    $meta?: TemplateMeta;
    $prefetch(): Promise<void>;
    $rendered(): Promise<void>;
  };
  const binding = bindingFor(id, true);
  Object.assign(binding, { condition: [() => open], anchor: { parentNode: {} } });
  requests.length = 0;
  host.$toggleCond(binding);
  assert.equal(requests.length, 0, 'a false condition loads nothing');
  open = true;
  host.$toggleCond(binding);
  await runtime.rendered(host);
  assert.deepEqual(requests, [{ url, priority: 'high' }]);
  assert.equal(binding.instance, null, 'the detached binding stays unmounted');
  assert.ok(registry()[id], 'the pack stays available for the next activation');

  const other = region('glue-owner', 1);
  const otherUrl = '_webui/packs/glue-owner.0013.json';
  serve(otherUrl, pack('glue-owner', { fragment: other, templates: { [other]: { v: 3, f: other, o: 'glue-owner', h: '' } } }));
  host.$meta!.lp = { [id]: url, [other]: otherUrl };
  await host.$prefetch();
  assert.deepEqual(requests.at(-1), { url: otherUrl, priority: 'auto' });
  assert.equal(typeof host.$rendered, 'function');
});

test('the element joins an installing pack instead of mounting before its resources', async () => {
  const { TemplateElement } = await import('./template-element.js');
  const id = region('join-owner');
  const url = '_webui/packs/join-owner.0016.json';
  serve(url, pack('join-owner'));
  const resources = holdResources();
  try {
    const host = Object.assign(Object.create(TemplateElement.prototype) as object, {
      localName: 'join-owner',
      $meta: { v: 3, h: '', lp: { [id]: url } } as TemplateMeta,
    }) as unknown as import('./lazy-runtime.js').LazyHost;
    // Detached bindings never mount, so the core needs no DOM here.
    const first = bindingFor(id, true);
    const second = bindingFor(id, true);
    for (const binding of [first, second]) Object.assign(binding, { condition: [() => true], anchor: { parentNode: {} } });
    requests.length = 0;
    host.$toggleCond(first);
    await until(() => registry()[id]);
    host.$toggleCond(second);
    let settled = false;
    const done = runtime.rendered(host).then(() => { settled = true; });
    await tick();
    assert.equal(settled, false, 'the second activation waits for the same install');
    resources.release();
    await done;
    assert.deepEqual(requests, [{ url, priority: 'high' }]);
    assert.equal(second.instance, null);
  } finally {
    resources.restore();
  }
});

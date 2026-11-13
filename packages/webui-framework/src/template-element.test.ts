// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { FLUSH_UPDATES, REQUEST_UPDATE } from './render-scheduler.js';
import { strict as assert } from 'node:assert';
import { describe, test } from 'node:test';
import type { TemplateMeta } from './template.js';
import type { TemplateInstance } from './element/types.js';
import { registerStreamingRootResume, resumeStreamingRoot } from './streaming-mode.js';

/**
 * `TemplateElement extends HTMLElement` at module scope, so `HTMLElement`
 * must exist globally before the module is evaluated — same mocking pattern
 * as `static-host.test.ts` and `streaming.test.ts`.
 */
Object.defineProperty(globalThis, 'HTMLElement', {
  value: class HTMLElement {
    tagName = '';
    get localName(): string { return this.tagName.toLowerCase(); }
    set localName(value: string) { this.tagName = value.toUpperCase(); }
    isConnected = false;
    childNodes: unknown[] = [];
    children: unknown[] = [];
    shadowRoot = null;
    ownerDocument = document;
    _attrs: Record<string, string> = {};

    hasAttribute(name: string): boolean {
      return Object.prototype.hasOwnProperty.call(this._attrs, name);
    }

    getAttribute(name: string): string | null {
      return Object.prototype.hasOwnProperty.call(this._attrs, name) ? this._attrs[name] : null;
    }

    setAttribute(name: string, value: string): void {
      this._attrs[name] = String(value);
    }

    removeAttribute(name: string): void {
      delete this._attrs[name];
    }

    getRootNode(): Document {
      return this.ownerDocument;
    }

    contains(other: unknown): boolean {
      return other === this;
    }
  },
  configurable: true,
});

Object.defineProperty(globalThis, 'document', {
  value: {
    nodeType: 9,
    readyState: 'loading',
    getElementById() {
      return null;
    },
    querySelector(selector: string) {
      // Matches the real streaming meta tag detection query in
      // streaming-mode.ts — asserted so this test breaks loudly if that
      // selector ever changes instead of silently detecting non-streaming.
      assert.equal(selector, 'meta[name="webui-streaming"][content="1"]');
      return { getAttribute: () => '1' };
    },
    querySelectorAll() {
      return [];
    },
  },
  configurable: true,
});

const dispatchedEvents: string[] = [];
Object.defineProperty(globalThis, 'window', {
  value: {
    __webui: { templates: {} },
    dispatchEvent(event: Event): boolean {
      dispatchedEvents.push(event.type);
      return true;
    },
  },
  configurable: true,
});

Object.defineProperty(globalThis, 'customElements', {
  value: {
    get() {
      return undefined;
    },
  },
  configurable: true,
});

const { TemplateElement, writeComplexProperty } = await import('./template-element.js');
const { registerTemplateData } = await import('./template.js');
const { activateTemplateDefinitions } = await import('./template-registry.js');
activateTemplateDefinitions();
const {
  ACTIVATION_ACTIVATED,
  ACTIVATION_ANCESTOR_BARRIER,
  ACTIVATION_MISSING_TEMPLATE,
  ACTIVATION_STATIC_HOST_OPT_OUT,
  resetStreamingModeForTests,
} = await import('./streaming-mode.js');
const {
  beginStreamingGate,
  markBoundaryPending,
  markBoundaryCommitted,
  __getLifecycleStateForTests,
  __resetLifecycleForTests,
} = await import('./lifecycle.js');

/** The activation hook the streaming coordinator invokes on a committed boundary. */
const STREAMING_BOUNDARY_ACTIVATE = Symbol.for('microsoft.webui.boundaryActivate');
const STREAMING_BOUNDARY_ABANDON = Symbol.for('microsoft.webui.boundaryAbandon');

/** Register template metadata for a tag exactly like `registerTemplateData()`. */
function registerTemplate(tag: string): void {
  (window as unknown as { __webui: { templates: Record<string, unknown> } }).__webui.templates[tag] = {
    v: 3,
    h: '<div></div>',
  };
}

interface PendingParentStateConsumer {
  $applyPendingParentState(replayAfterHydration: boolean): void;
}

test('condition resolvers retain their owner and nested repeat scope', () => {
  const first = Object.assign(new TemplateElement(), { label: 'first' });
  const second = Object.assign(new TemplateElement(), { label: 'second' });
  const resolve = first['$conditionResolver']();
  const scope = {
    name: 'child',
    value: { label: 'nested' },
    parent: { name: 'parent', value: { label: 'outer' } },
  };
  assert.equal(first['$conditionResolver'](), resolve);
  assert.notEqual(second['$conditionResolver'](), resolve);
  assert.equal(resolve('label'), 'first');
  assert.equal(resolve('child.label', scope), 'nested');
  assert.equal(resolve('parent.label', scope), 'outer');
  assert.equal(second['$conditionResolver']()('label'), 'second');
});

test('scope resolution distinguishes complete names from prefixes and retains readiness', () => {
  const host = Object.assign(new TemplateElement(), {
    itemized: { label: 'component' },
    item: { label: 'component fallback' },
  });
  const scope = {
    name: 'item', value: { label: 'local' }, known: false,
    parent: { name: 'items', value: { label: 'outer' }, known: true },
  };
  assert.equal(host.$resolveValue('item.label', scope), 'local');
  assert.equal(host.$resolveValue('items.label', scope), 'outer');
  assert.equal(host.$resolveValue('itemized.label', scope), 'component');
  assert.equal(host.$hasStateRoot('item.label', scope), false);
  assert.equal(host.$hasStateRoot('items.label', scope), true);
  assert.equal(host.$hasStateRoot('itemized.label', scope), true);
});

test('an unavailable inner scope shadows both an outer scope and component state', () => {
  const host = Object.assign(new TemplateElement(), { item: { label: 'component' } });
  const scope = {
    name: 'item', value: undefined, known: false,
    parent: { name: 'item', value: { label: 'outer' }, known: true },
  };
  assert.equal(host.$resolveValue('item.label', scope), undefined);
  assert.equal(host.$hasStateRoot('item.label', scope), false);
});

for (const wildcard of [false, true]) {
  test(`an unindexed reactive write preserves batching with wildcard=${wildcard}`, async () => {
    const element = new TemplateElement();
    const targeted = ['derived'];
    const computed = ['computed'];
    const entry = (texts: string[]) => ({ texts, attrs: [], conds: [], repeats: [] });
    const applied: string[] = [];
    let fullRefreshes = 0;
    const root = {};
    Object.defineProperties(element, {
      $ready: { value: true },
      $root: { value: root },
      $pathIndex: { value: new Map([['derived', entry(targeted)]]) },
      $wildcardBindings: { value: wildcard ? entry(computed) : null },
      $updateInstance: { value(bindings: { texts: string[] }) {
        if (bindings === root) fullRefreshes++;
        else applied.push(...bindings.texts);
      } },
    });
    element[REQUEST_UPDATE]('derived');
    element[REQUEST_UPDATE]('value');
    assert.equal(fullRefreshes, 0);
    assert.deepEqual(applied, []);
    await Promise.resolve();
    assert.deepEqual(applied, wildcard ? ['derived', 'computed'] : ['derived']);
    element[REQUEST_UPDATE]('value');
    await Promise.resolve();
    assert.deepEqual(applied, wildcard ? ['derived', 'computed', 'computed'] : ['derived']);
    element[REQUEST_UPDATE]();
    assert.equal(fullRefreshes, 1, 'explicit refresh remains synchronous');
  });
}

test('binding arrays reserve their compiled size instead of push-grown capacity', () => {
  const originalArray = globalThis.Array;
  const commentDescriptor = Object.getOwnPropertyDescriptor(document, 'createComment');
  const allocations = new Map<object, unknown[]>();
  const root = {
    nodeType: 11,
    firstChild: null,
    childNodes: [] as Node[],
    insertBefore(node: Node): Node {
      this.childNodes.push(node);
      return node;
    },
  };
  try {
    Object.defineProperty(document, 'createComment', {
      configurable: true,
      value: () => ({ nodeType: 8, data: '', parentNode: root }),
    });
    globalThis.Array = new Proxy(originalArray, {
      construct(target, args) {
        const array = Reflect.construct(target, args);
        allocations.set(array, args);
        return array;
      },
    });
    const host = new TemplateElement() as unknown as {
      $wire(root: Node, meta: TemplateMeta): TemplateInstance;
    };
    const instance = host.$wire(root as unknown as Node, {
      v: 3, h: '', c: [[[() => false, []], 0, [0, 0]]],
    });
    assert.deepEqual(allocations.get(instance.conds), [1]);
    assert.equal(instance.conds.length, 1);
    assert.equal(instance.conds[0].owner, instance);
    assert.equal(instance.texts.length, 0);
    assert.equal(instance.attrs.length, 0);
    assert.equal(instance.repeats.length, 0);
  } finally {
    globalThis.Array = originalArray;
    if (commentDescriptor) Object.defineProperty(document, 'createComment', commentDescriptor);
    else Reflect.deleteProperty(document, 'createComment');
  }
});

test('path indexes share empty buckets without sharing populated binding arrays', () => {
  const firstText = { node: {} as CharacterData, path: 'first' };
  const secondText = { node: {} as CharacterData, parts: [['second'], ['second']] };
  const thirdText = { node: {} as CharacterData, path: 'second' };
  const localText = {
    node: {} as CharacterData,
    path: 'row.label',
    scope: { name: 'row', value: { label: 'local' } },
  };
  const binding = {
    element: {} as Element, name: 'title', kind: 0, path: 'first',
  };
  const unownedText = { node: {} as CharacterData, path: 'unowned' };
  const condition = { condition: [() => true, ['first', 'first']] };
  const repeat = { collection: 'second', instances: [] };
  function indexed() {
    const host = new TemplateElement();
    Object.defineProperties(host, {
      $root: { value: {
        texts: [firstText, secondText, thirdText, localText, unownedText],
        attrs: [binding], conds: [condition], repeats: [repeat],
      } },
      $observableNames: { value: () => new Set(['first', 'second']) },
    });
    host['$buildPathIndex']();
    return host;
  }
  const first = indexed();
  const second = indexed();
  const index = first['$pathIndex']!;
  const secondIndex = second['$pathIndex']!;
  assert.ok(index instanceof Map);
  assert.ok(secondIndex instanceof Map);
  const a = index.get('first')!;
  const b = index.get('second')!;
  assert.deepEqual(a.texts, [firstText]);
  assert.deepEqual(a.attrs, [binding]);
  assert.deepEqual(b.texts, [secondText, thirdText], 'duplicate parts are indexed once');
  assert.deepEqual(a.conds, [condition], 'duplicate condition paths are indexed once');
  assert.deepEqual(b.repeats, [repeat]);
  assert.equal(index.has('row'), false, 'local repeat paths never enter the host index');
  assert.deepEqual(first['$wildcardBindings']!.texts, [unownedText]);
  assert.equal(b.conds, b.attrs);
  assert.equal(b.conds, a.repeats);
  assert.equal(b.conds, first['$wildcardBindings']!.conds);
  assert.equal(b.conds, secondIndex.get('second')!.attrs);
  assert.notEqual(a.texts, secondIndex.get('first')!.texts);
  assert.notEqual(a.attrs, secondIndex.get('first')!.attrs);
  assert.equal(b.conds.length, 0, 'populating another bucket never mutates the shared empty array');
});

test('body-only writes can reach a text-only conditional with an empty SSR range', () => {
  const root: TemplateInstance = {
    container: null, nodes: [], texts: [], attrs: [], conds: [], repeats: [],
  };
  root.conds.push({
    condition: [() => true, ['show']], blockIndex: 0,
    anchor: null, owner: root, instance: undefined,
  }, {
    condition: [() => false, ['hidden']], blockIndex: 1,
    anchor: null, owner: root, instance: null,
  }, {
    condition: [() => false, ['hidden']], blockIndex: 2,
    anchor: null, owner: root, instance: undefined,
  }, {
    condition: [() => false, ['hidden']], blockIndex: 3,
    anchor: null, owner: root, instance: undefined,
  });
  const host = new TemplateElement();
  Object.defineProperties(host, {
    $root: { value: root },
    $observableNames: { value: () => new Set(['show', 'details', 'hidden', 'unmounted', 'probe', 'staticBody']) },
    $meta: { value: {
      v: 3, h: '', b: [
        { h: '', tx: [[[0, 0], [['details'], ['details']]]] },
        { h: '', tx: [[[0, 0], [['unmounted']]]] },
        { h: '', tx: [[[0, 0], [['probe']]]] },
        { h: '<span></span>', tx: [[[1, 0], [['staticBody']]]] },
      ],
    } satisfies TemplateMeta },
  });
  host['$buildPathIndex']();
  assert.deepEqual(host['$pathBindings']('details')?.conds, [root.conds[0]]);
  assert.equal(host['$pathBindings']('unmounted'), undefined);
  assert.equal(host['$pathBindings']('staticBody'), undefined);
  assert.deepEqual(host['$pathBindings']('probe')?.conds, [root.conds[2]]);
  host['$toggleCond'](root.conds[2]);
  assert.equal(root.conds[2].instance, null);
  host['$buildPathIndex']();
  assert.equal(host['$pathBindings']('probe'), undefined);
});

test('provisional body paths do not imply a conditional presence mismatch', async (t) => {
  const { reportHydrationMismatch } = await import('./hydration-mismatch.js');
  const root: TemplateInstance = {
    container: null, nodes: [], texts: [], attrs: [], conds: [], repeats: [],
  };
  root.conds.push({
    condition: [() => true, []], blockIndex: 0,
    anchor: null, owner: root, instance: undefined,
  }, {
    condition: [() => true, ['show.enabled']], blockIndex: 1,
    anchor: null, owner: root, instance: undefined,
  });
  const host = new TemplateElement();
  Object.defineProperties(host, {
    $root: { value: root },
    $observableNames: { value: () => new Set(['details', 'show']) },
    $meta: { value: {
      v: 3, h: '', b: [
        { h: '', tx: [[[0, 0], [['details']]]] },
        { h: '<span></span>' },
      ],
    } satisfies TemplateMeta },
  });
  host['$buildPathIndex']();
  assert.deepEqual(host['$pathBindings']('details')?.conds, [root.conds[0]]);
  const warnings: string[] = [];
  t.mock.method(console, 'warn', (message: string) => { warnings.push(message); });
  const ctx = { resolver: () => '', resolveValue: () => '', resolveParts: () => '' };
  reportHydrationMismatch('test-empty-ssr-body', new Set(['details']), host['$pathIndex']!, ctx);
  assert.deepEqual(warnings, [], 'an empty body write cannot prove a presence mismatch');
  assert.equal(root.conds[0].instance, undefined, 'diagnostics must not reconcile the view');

  reportHydrationMismatch('test-ssr-presence', new Set(['show']), host['$pathIndex']!, ctx);
  assert.equal(warnings.length, 1, 'writes to condition dependencies must still report missing content');
  assert.match(warnings[0], /"show"/);
  warnings.length = 0;
  root.texts.push({ node: { data: 'server' } as Text, path: 'details' });
  host['$buildPathIndex']();
  reportHydrationMismatch('test-ssr-text', new Set(['details']), host['$pathIndex']!, ctx);
  assert.equal(warnings.length, 1, 'a provisional condition must not hide a real text mismatch');
});

test('SSR text without a neighboring dynamic marker allocates no topology array', () => {
  const original = globalThis.Int32Array;
  const text = { nodeType: 3, previousSibling: null, nextSibling: null };
  const paragraph = {
    nodeType: 1, localName: 'p', firstChild: text, lastChild: text,
    childNodes: [text], nextSibling: null as unknown,
    hasAttribute: () => false,
  };
  const end = { nodeType: 8, data: '/wc', nextSibling: null };
  const start = { nodeType: 8, data: 'wc', nextSibling: end, parentNode: null as unknown };
  paragraph.nextSibling = start;
  const root = {
    nodeType: 11, firstChild: paragraph, childNodes: [paragraph, start, end],
    removeChild(node: unknown) {
      this.childNodes.splice(this.childNodes.indexOf(node as typeof end), 1);
    },
  };
  start.parentNode = root;
  Object.assign(end, { parentNode: root });
  let allocations = 0;
  try {
    globalThis.Int32Array = new Proxy(original, {
      construct(target, args) {
        allocations++;
        return Reflect.construct(target, args);
      },
    });
    const host = new TemplateElement() as unknown as {
      $hydrate(root: Node, meta: TemplateMeta): TemplateInstance;
    };
    const instance = host.$hydrate(root as unknown as Node, {
      v: 3, h: '<p></p>', l: [3, 1],
      tx: [[[1, 0], ['hello']]],
      c: [[[() => false, []], 0, [0, 1]]],
    });
    assert.equal(instance.texts[0].node, text);
    assert.equal(instance.conds.length, 1);
    assert.equal(allocations, 0);
  } finally {
    globalThis.Int32Array = original;
  }
});

test('registered constructors retain attribute names, not complete template metadata', () => {
  const previous = Object.getOwnPropertyDescriptor(globalThis, 'customElements');
  const constructors = new Map<string, CustomElementConstructor>();
  Object.defineProperty(globalThis, 'customElements', {
    configurable: true,
    value: {
      get: (tag: string) => constructors.get(tag),
      define: (tag: string, ctor: CustomElementConstructor) => constructors.set(tag, ctor),
    },
  });
  const tag = 'collectible-template';
  const metadata: TemplateMeta = { v: 3, h: '<span></span>', tr: ['title'], ta: ['title'] };
  window.__webui!.templates![tag] = metadata;
  class CollectibleTemplate extends TemplateElement {
    templateMetadata(): TemplateMeta | undefined {
      return this.$currentTemplateMetadata();
    }
  }
  try {
    CollectibleTemplate.define(tag);
    assert.deepEqual(Reflect.get(CollectibleTemplate, 'observedAttributes'), ['title']);
    assert.equal(Object.getOwnPropertyDescriptor(CollectibleTemplate, 'observedAttributes')?.get, undefined);
    const element = new CollectibleTemplate();
    let constructorReads = 0;
    Object.defineProperties(element, {
      localName: { value: tag },
      constructor: { get() { constructorReads++; return CollectibleTemplate; } },
    });
    assert.equal(element.templateMetadata(), metadata);
    assert.equal(constructorReads, 0, 'Native host identity replaces the constructor-to-tag registry');

    delete window.__webui!.templates![tag];
    assert.equal(element.templateMetadata(), undefined);
    assert.deepEqual(Reflect.get(CollectibleTemplate, 'observedAttributes'), ['title']);
  } finally {
    delete window.__webui!.templates![tag];
    if (previous) Object.defineProperty(globalThis, 'customElements', previous);
    else Reflect.deleteProperty(globalThis, 'customElements');
  }
});

for (const wildcard of [false, true]) {
  test(`unmounted template state preserves unrelated SSR bindings (wildcard: ${wildcard})`, () => {
    const element = new TemplateElement();
    const rootTexts: unknown[] = [];
    const wildcardTexts: unknown[] = [];
    const updated: unknown[][] = [];
    const bindings = (texts: unknown[]) => ({ texts, attrs: [], conds: [], repeats: [] });
    Object.defineProperties(element, {
      $ready: { value: true },
      $root: { value: bindings(rootTexts) },
      $meta: { value: { h: '', tr: ['deferredValue'] } },
      $pathIndex: { value: new Map() },
      $wildcardBindings: { value: wildcard ? bindings(wildcardTexts) : null },
      $updateInstance: { value({ texts }: { texts: unknown[] }) { updated.push(texts); } },
    });

    element.setState({ deferredValue: 'Loaded before the body mounts' });
    assert.equal(element.$resolveValue('deferredValue'), 'Loaded before the body mounts');
    assert.equal(updated.length, wildcard ? 1 : 0);
    if (wildcard) assert.equal(updated[0], wildcardTexts);

    updated.length = 0;
    element[REQUEST_UPDATE]('untracked.path');
    element[FLUSH_UPDATES]();
    assert.equal(updated.length, wildcard ? 1 : 0, 'unindexed writes avoid a full update');
    if (wildcard) assert.equal(updated[0], wildcardTexts);
    element[REQUEST_UPDATE]();
    assert.equal(updated.length, wildcard ? 2 : 1, 'an explicit full update still visits the root');
    assert.equal(updated[updated.length - 1], rootTexts);
  });
}

test('an observable without live bindings does not trigger a full template update', () => {
  const names = new Set(['deferredValue']);
  class DormantObservable extends TemplateElement {
    protected override $observableNames(): Set<string> { return names; }
  }
  const element = new DormantObservable();
  let updates = 0;
  Object.defineProperties(element, {
    $ready: { value: true },
    $root: { value: { texts: [], attrs: [], conds: [], repeats: [] } },
    $pathIndex: { value: new Map() },
    $updateInstance: { value() { updates++; } },
  });
  element[REQUEST_UPDATE]('deferredValue');
  element[FLUSH_UPDATES]();
  assert.equal(updates, 0);
});

test('a rejected binding releases the flush gate and preserves re-entrant updates', () => {
  const originalQueue = globalThis.queueMicrotask;
  const queued: VoidFunction[] = [];
  globalThis.queueMicrotask = callback => { queued.push(callback); };
  const element = new TemplateElement();
  const rejected: unknown[] = [];
  const valid: unknown[] = [];
  const entry = (texts: unknown[]) => ({ texts, attrs: [], conds: [], repeats: [] });
  let updates = 0;
  const failure = new TypeError('TrustedHTML required');
  Object.defineProperties(element, {
    $ready: { value: true },
    $root: { value: {} },
    $pathIndex: { value: new Map([['raw', entry(rejected)], ['count', entry(valid)]]) },
    $updateInstance: { value({ texts }: { texts: unknown[] }) {
      if (texts === rejected) {
        element[REQUEST_UPDATE]('count');
        throw failure;
      }
      assert.equal(texts, valid);
      updates++;
    } },
  });
  try {
    element[REQUEST_UPDATE]('raw');
    assert.equal(queued.length, 1);
    assert.throws(() => queued.shift()!(), failure);
    assert.equal(queued.length, 1);
    queued.shift()!();
    assert.equal(updates, 1);
    assert.equal(queued.length, 0);
    element[REQUEST_UPDATE]('count');
    assert.equal(queued.length, 1);
    queued.shift()!();
    assert.equal(updates, 2);
    assert.equal(queued.length, 0);
  } finally {
    globalThis.queueMicrotask = originalQueue;
  }
});

test('single dirty paths allocate no collection and promotion preserves reentrant writes', () => {
  const element = new TemplateElement();
  const applied: string[] = [];
  Object.defineProperties(element, {
    $ready: { value: true },
    $root: { value: {} },
    $pathIndex: { value: new Map(['first', 'second'].map(path =>
      [path, { texts: [path], attrs: [], conds: [], repeats: [] }])) },
    $updateInstance: { value({ texts: [path] }: { texts: string[] }) {
      applied.push(path);
      if (applied.length === 1) element[REQUEST_UPDATE]('first');
    } },
  });
  element[REQUEST_UPDATE]('first');
  element[REQUEST_UPDATE]('first');
  assert.equal(element['$dirtyPaths'], 'first');
  element[REQUEST_UPDATE]('second');
  assert.deepEqual(element['$dirtyPaths'], new Set(['first', 'second']));
  element[FLUSH_UPDATES]();
  assert.deepEqual(applied, ['first', 'second', 'first']);
  assert.equal(element['$dirtyPaths'], null);
});

test('flat reactive plans are shared without allocating instance indexes', () => {
  const meta: TemplateMeta = {
    v: 3, h: '', tx: [[[0, 0], [['value']]]], tr: ['value'], rp: 'value',
  };
  const make = (value: string) => {
    const element = Object.assign(new TemplateElement(), { value });
    const node = { data: '' } as CharacterData;
    Object.defineProperties(element, {
      $meta: { value: meta },
      $ready: { value: true },
      $root: { value: {
        container: null, nodes: [node], texts: [{ node, parts: meta.tx![0][1] }],
        attrs: [], conds: [], repeats: [],
      } },
      $observableNames: { value: () => new Set(['value']) },
    });
    element[REQUEST_UPDATE]('value');
    element[FLUSH_UPDATES]();
    return { element, node };
  };
  const OriginalMap = globalThis.Map;
  let allocations = 0;
  let first: ReturnType<typeof make>;
  let second: ReturnType<typeof make>;
  try {
    globalThis.Map = new Proxy(OriginalMap, {
      construct(target, args) {
        allocations++;
        return Reflect.construct(target, args);
      },
    });
    first = make('first');
    second = make('second');
  } finally {
    globalThis.Map = OriginalMap;
  }
  assert.equal(allocations, 0);
  assert.equal(first.element['$pathIndex'], second.element['$pathIndex']);
  assert.equal(first.node.data, 'first');
  assert.equal(second.node.data, 'second');
  second.element.value = 'changed';
  second.element[REQUEST_UPDATE]('value');
  second.element[FLUSH_UPDATES]();
  assert.equal(first.node.data, 'first');
  assert.equal(second.node.data, 'changed');
});

test('a compiled single-root plan preserves wildcard updates for an authored non-observable value', () => {
  const element = Object.assign(new TemplateElement(), { computed: 'derived' });
  const node = { data: '' } as CharacterData;
  const meta: TemplateMeta = {
    v: 3, h: '', tx: [[[0, 0], [['computed']]]], tr: ['computed'], rp: 'computed',
  };
  Object.defineProperties(element, {
    $meta: { value: meta },
    $ready: { value: true },
    $root: { value: {
      container: null, nodes: [node], texts: [{ node, parts: meta.tx![0][1] }],
      attrs: [], conds: [], repeats: [],
    } },
    $observableNames: { value: () => new Set(['trigger']) },
  });
  element[REQUEST_UPDATE]('trigger');
  element[FLUSH_UPDATES]();
  assert.equal(node.data, 'derived');
  assert.ok(element['$pathIndex'] instanceof Map);
  assert.equal(element['$wildcardBindings']?.texts.length, 1);
});

test('a shared plan stops when a binding destroys its current root', () => {
  const element = Object.assign(new TemplateElement(), { value: 'changed' });
  const first = { data: '' } as CharacterData;
  const second = { data: '' } as CharacterData;
  const parts: [string][] = [['value']];
  const root: TemplateInstance = {
    container: null, nodes: [first, second],
    texts: [{ node: first, parts }, { node: second, parts }],
    attrs: [], conds: [], repeats: [],
  };
  Object.defineProperties(element, {
    $meta: { value: { v: 3, h: '', tx: [[[0, 0], parts], [[0, 1], parts]], tr: ['value'], rp: 'value' } },
    $ready: { value: true },
    $root: { value: root, writable: true },
    $observableNames: { value: () => new Set(['value']) },
  });
  Object.defineProperty(first, 'data', {
    get: () => '',
    set() {
      element['$root'] = null;
      root.texts.length = 0;
    },
  });
  element[REQUEST_UPDATE]('value');
  assert.doesNotThrow(() => element[FLUSH_UPDATES]());
  assert.equal(second.data, '');
});

for (const paths of [
  ['raw', 'count'],
  ['before', 'raw', 'count'],
  ['raw', 'otherRaw', 'count'],
]) {
  test(`a rejected binding preserves the remaining queued paths: ${paths.join(', ')}`, () => {
    const originalQueue = globalThis.queueMicrotask;
    const queued: VoidFunction[] = [];
    globalThis.queueMicrotask = callback => { queued.push(callback); };
    const element = new TemplateElement();
    const applied: string[] = [];
    const failures: string[] = [];
    const failure = new TypeError('TrustedHTML required');
    Object.defineProperties(element, {
      $ready: { value: true },
      $root: { value: {} },
      $pathIndex: { value: new Map([...paths, 'reentrant'].map(path =>
        [path, { texts: [path], attrs: [], conds: [], repeats: [] }])) },
      $updateInstance: { value({ texts: [path] }: { texts: string[] }) {
        if (path === 'raw' || path === 'otherRaw') {
          failures.push(path);
          element[REQUEST_UPDATE]('count');
          element[REQUEST_UPDATE]('reentrant');
          throw failure;
        }
        applied.push(path);
      } },
    });
    try {
      for (const path of paths) element[REQUEST_UPDATE](path);
      assert.equal(queued.length, 1);
      assert.throws(() => queued.shift()!(), failure);
      if (paths.includes('otherRaw')) {
        assert.equal(queued.length, 1);
        assert.throws(() => queued.shift()!(), failure);
      }
      assert.equal(queued.length, 1);
      queued.shift()!();
      assert.deepEqual(failures, paths.filter(path => path.toLowerCase().includes('raw')));
      assert.deepEqual(applied.filter(path => path === 'before'), paths.filter(path => path === 'before'));
      assert.ok(applied.includes('count'));
      assert.ok(applied.includes('reentrant'));
      assert.equal(queued.length, 0);
    } finally {
      globalThis.queueMicrotask = originalQueue;
    }
  });
}

test('a rejected binding does not drop a later path queued before the flush', () => {
  const originalQueue = globalThis.queueMicrotask;
  const queued: VoidFunction[] = [];
  globalThis.queueMicrotask = callback => { queued.push(callback); };
  const element = new TemplateElement();
  const applied: string[] = [];
  const failure = new TypeError('TrustedHTML required');
  Object.defineProperties(element, {
    $ready: { value: true },
    $root: { value: {} },
    $pathIndex: { value: new Map(['raw', 'count'].map(path =>
      [path, { texts: [path], attrs: [], conds: [], repeats: [] }])) },
    $updateInstance: { value({ texts: [path] }: { texts: string[] }) {
      if (path === 'raw') throw failure;
      applied.push(path);
    } },
  });
  try {
    element[REQUEST_UPDATE]('raw');
    element[REQUEST_UPDATE]('count');
    assert.equal(queued.length, 1);
    assert.throws(() => queued.shift()!(), failure);
    assert.equal(queued.length, 1);
    queued.shift()!();
    assert.deepEqual(applied, ['count']);
    assert.equal(queued.length, 0);
  } finally {
    globalThis.queueMicrotask = originalQueue;
  }
});

describe('TemplateElement complex-property delivery', () => {
  test('queues an unresolved WebUI child without creating an own property', () => {
    const tag = 'test-pending-property';
    registerTemplate(tag);
    class PendingChild extends TemplateElement {
      protected override $observableNames(): Set<string> {
        return new Set(['payload']);
      }
    }
    Object.defineProperty(PendingChild.prototype, 'payload', {
      get(this: { _payload?: unknown }) {
        return this._payload;
      },
      set(this: { _payload?: unknown }, value: unknown) {
        this._payload = value;
      },
      configurable: true,
    });

    const child = new PendingChild() as PendingChild & {
      localName: string;
      _payload?: unknown;
    };
    child.localName = tag;

    writeComplexProperty(
      child,
      'payload',
      { label: 'from parent' },
      false,
    );

    assert.equal(Object.hasOwn(child, 'payload'), false);
    (child as unknown as PendingParentStateConsumer)
      .$applyPendingParentState(false);
    assert.deepEqual(child._payload, { label: 'from parent' });
    assert.equal(Object.hasOwn(child, 'payload'), false);
  });

  test('preserves direct assignment for an unresolved third-party element', () => {
    const child = { localName: 'external-property-target' } as unknown as
      Element & { payload?: unknown };

    writeComplexProperty(
      child,
      'payload',
      { label: 'direct' },
      false,
    );

    assert.deepEqual(child.payload, { label: 'direct' });
    assert.equal(Object.hasOwn(child, 'payload'), true);
  });
});

describe('TemplateElement.connectedCallback — streamed-host (data-ws) deferral', () => {
  test('defers a data-ws-marked streamed host without warning, even when metadata is missing', () => {
    resetStreamingModeForTests();

    const previousWarn = console.warn;
    let warned = false;
    console.warn = () => {
      warned = true;
    };

    try {
      // A streamed SSR component host carries the parser-emitted `data-ws`
      // marker. It connects at its opening tag (zero children, no shadow root)
      // before its boundary — and thus its template metadata — has streamed in.
      const el = new TemplateElement();
      (el as unknown as { tagName: string }).tagName = 'test-stream-widget';
      (el as unknown as { setAttribute(n: string, v: string): void }).setAttribute('data-ws', '');

      assert.equal((el as unknown as { childNodes: unknown[] }).childNodes.length, 0);

      el.connectedCallback();

      assert.equal(warned, false, 'a marked streamed host must never warn about in-flight metadata');
      assert.equal((el as unknown as { $deferredSSR: boolean }).$deferredSSR, true);
      assert.equal((el as unknown as { $ready: boolean }).$ready, true);
      // The marker stays until the boundary activates the instance.
      assert.equal((el as unknown as { hasAttribute(n: string): boolean }).hasAttribute('data-ws'), true);
    } finally {
      console.warn = previousWarn;
    }
  });

  test('lets a pending-definition resume own activation without replaying ordinary bootstrap state', () => {
    const tag = 'test-pending-resume-widget';
    registerTemplate(tag);
    let ordinaryDeferrals = 0;
    let received: Record<string, unknown> | undefined;

    class PendingResumeElement extends TemplateElement {
      protected override $didDeferSSRHydration(): void {
        ordinaryDeferrals++;
      }
    }

    const el = new PendingResumeElement();
    const raw = el as unknown as {
      tagName: string;
      $deferredSSR: boolean;
      $hydrated: boolean;
      setAttribute(name: string, value: string): void;
      removeAttribute(name: string): void;
      [STREAMING_BOUNDARY_ACTIVATE](
        state?: Record<string, unknown>,
      ): number;
    };
    raw.tagName = tag;
    raw.$hydrated = true;
    raw.setAttribute('data-ws', '');
    const previousResume = resumeStreamingRoot;
    registerStreamingRootResume((root) => {
      assert.equal(root, el);
      received = { status: 'ready' };
      assert.equal(
        raw[STREAMING_BOUNDARY_ACTIVATE](received),
        ACTIVATION_ACTIVATED,
      );
      raw.removeAttribute('data-ws');
      return true;
    });
    try {
      el.connectedCallback();
    } finally {
      registerStreamingRootResume(previousResume);
    }

    assert.deepEqual(received, { status: 'ready' });
    assert.equal(raw.$deferredSSR, false);
    assert.equal(
      ordinaryDeferrals,
      0,
      'ordinary deferral would replay older page bootstrap state after the queued update',
    );
  });

  test('warns for an UNMARKED client-created element with missing metadata (no silent defer)', () => {
    resetStreamingModeForTests();

    const previousWarn = console.warn;
    let warned = false;
    console.warn = () => {
      warned = true;
    };

    try {
      // No `data-ws`: a genuinely client-created empty element. With the old
      // empty-subtree heuristic removed, a missing template is now a real
      // authoring error surfaced immediately rather than an indefinite defer.
      const el = new TemplateElement();
      (el as unknown as { tagName: string }).tagName = 'test-unmarked-widget';

      el.connectedCallback();

      assert.equal(warned, true, 'an unmarked element with no metadata must warn, not defer');
      assert.notEqual((el as unknown as { $deferredSSR: boolean }).$deferredSSR, true);
    } finally {
      console.warn = previousWarn;
    }
  });

  test('does not reserve an authored data-ws attribute on an ordinary page', () => {
    const documentFake = document as unknown as {
      querySelector(selector: string): unknown;
    };
    const streamingQuery = documentFake.querySelector;
    documentFake.querySelector = () => null;
    resetStreamingModeForTests();

    const previousWarn = console.warn;
    let warned = false;
    console.warn = () => {
      warned = true;
    };

    try {
      const el = new TemplateElement();
      (el as unknown as { tagName: string }).tagName = 'test-ordinary-data-attribute';
      (el as unknown as { setAttribute(n: string, v: string): void }).setAttribute('data-ws', 'authored');

      el.connectedCallback();

      assert.equal(warned, true, 'ordinary lifecycle continues through metadata lookup');
      assert.notEqual((el as unknown as { $deferredSSR: boolean }).$deferredSSR, true);
    } finally {
      console.warn = previousWarn;
      documentFake.querySelector = streamingQuery;
      resetStreamingModeForTests();
    }
  });
});

describe('TemplateElement.connectedCallback — reused-template race', () => {
  test('defers a data-ws instance even when the tag metadata is already registered by an earlier boundary', () => {
    resetStreamingModeForTests();

    // Boundary 0 already committed and registered this tag's template metadata;
    // a later, not-yet-committed boundary connects its own <test-reused-widget>
    // instance whose SSR children have not been parsed yet. The `data-ws`
    // marker short-circuits the metadata lookup, so $mount() can never
    // misclassify the empty instance as client-created.
    registerTemplate('test-reused-widget');

    const el = new TemplateElement();
    (el as unknown as { tagName: string }).tagName = 'test-reused-widget';
    (el as unknown as { setAttribute(n: string, v: string): void }).setAttribute('data-ws', '');

    assert.equal((el as unknown as { childNodes: unknown[] }).childNodes.length, 0);

    el.connectedCallback();

    assert.equal((el as unknown as { $deferredSSR: boolean }).$deferredSSR, true);
    assert.equal((el as unknown as { $ready: boolean }).$ready, true);
    // $mount() must never have run: no client root wired, $meta still unset
    // (resolved lazily at activation).
    assert.equal((el as unknown as { $root: unknown }).$root, null);
    assert.equal((el as unknown as { $meta: unknown }).$meta, undefined);
  });
});

describe('TemplateElement — streamed-host activation ownership', () => {
  test('activation clears the deferral but leaves data-ws for the coordinator to strip', () => {
    resetStreamingModeForTests();
    registerTemplate('test-activate-widget');

    const el = new TemplateElement();
    (el as unknown as { tagName: string }).tagName = 'test-activate-widget';
    const raw = el as unknown as {
      setAttribute(n: string, v: string): void;
      hasAttribute(n: string): boolean;
      $deferredSSR: boolean;
      $hydrated: boolean;
      [STREAMING_BOUNDARY_ACTIVATE](state?: Record<string, unknown>): number;
    };
    raw.setAttribute('data-ws', '');

    // Reach the deferred state the coordinator would activate. Marking the
    // instance already-hydrated makes $mount() a clean no-op so this test
    // isolates the activation contract without a real DOM.
    el.connectedCallback();
    assert.equal(raw.$deferredSSR, true);
    assert.equal(raw.hasAttribute('data-ws'), true);
    raw.$hydrated = true;

    raw[STREAMING_BOUNDARY_ACTIVATE]();

    assert.equal(raw.$deferredSSR, false, 'activation clears the deferral');
    // Successful-path marker removal is centralized in the streaming
    // coordinator's `invokeActivationHook` (proved by the pipeline tests), NOT
    // duplicated in TemplateElement — so invoking the hook directly leaves the
    // marker in place. This guards against re-introducing the duplicate cleanup.
    assert.equal(raw.hasAttribute('data-ws'), true, 'TemplateElement itself does not strip the marker');
  });

  test('activation establishes deferral for a detached root upgraded before first connection', () => {
    let received: Record<string, unknown> | undefined;
    let wasDeferred = false;

    class DetachedUpgradeElement extends TemplateElement {
      protected override $activateDeferredSSR(state?: Record<string, unknown>): void {
        wasDeferred = (this as unknown as { $deferredSSR: boolean }).$deferredSSR;
        received = state;
      }
    }

    const el = new DetachedUpgradeElement();
    const raw = el as unknown as {
      $meta?: TemplateMeta;
      setAttribute(name: string, value: string): void;
      [STREAMING_BOUNDARY_ACTIVATE](state?: Record<string, unknown>): number;
    };
    raw.$meta = { v: 3, h: '<span></span>' };
    raw.setAttribute('data-ws', '');

    raw[STREAMING_BOUNDARY_ACTIVATE]({ detached: true });

    assert.equal(wasDeferred, true, 'the marker establishes dormant SSR state without connectedCallback');
    assert.deepEqual(received, { detached: true });
  });

  test('a coordinator-resolved bypass ancestor is skipped exactly once', () => {
    const parentTag = 'test-spanning-parent';
    const childTag = 'test-early-span-child';
    registerTemplate(parentTag);
    registerTemplate(childTag);

    const parent = new TemplateElement();
    const parentRaw = parent as unknown as {
      tagName: string;
      parentElement: Element | null;
      $deferredSSR: boolean;
    };
    parentRaw.tagName = parentTag;
    parentRaw.parentElement = null;
    parentRaw.$deferredSSR = true;

    const child = new TemplateElement();
    const childRaw = child as unknown as {
      tagName: string;
      parentElement: Element;
      $deferredSSR: boolean;
      $hydrated: boolean;
      setAttribute(name: string, value: string): void;
      [STREAMING_BOUNDARY_ACTIVATE](
        state?: Record<string, unknown>,
        bypassAncestor?: Element,
      ): number;
    };
    childRaw.tagName = childTag;
    childRaw.parentElement = parent as unknown as Element;
    childRaw.setAttribute('data-ws', '');
    child.connectedCallback();
    childRaw.$hydrated = true;

    assert.equal(
      childRaw[STREAMING_BOUNDARY_ACTIVATE](
        { child: true },
        parent as unknown as Element,
      ),
      ACTIVATION_ACTIVATED,
    );
    assert.equal(childRaw.$deferredSSR, false);
  });

  test('an unrelated bypass ancestor preserves the parent-first barrier', () => {
    const parentTag = 'test-mismatch-span-parent';
    const childTag = 'test-mismatch-span-child';
    registerTemplate(parentTag);
    registerTemplate(childTag);

    const parent = new TemplateElement();
    const parentRaw = parent as unknown as {
      tagName: string;
      parentElement: Element | null;
      $deferredSSR: boolean;
    };
    parentRaw.tagName = parentTag;
    parentRaw.parentElement = null;
    parentRaw.$deferredSSR = true;

    const unrelated = new TemplateElement();
    (unrelated as unknown as { tagName: string }).tagName = parentTag;

    const child = new TemplateElement();
    const childRaw = child as unknown as {
      tagName: string;
      parentElement: Element;
      $deferredSSR: boolean;
      setAttribute(name: string, value: string): void;
      [STREAMING_BOUNDARY_ACTIVATE](
        state?: Record<string, unknown>,
        bypassAncestor?: Element,
      ): number;
    };
    childRaw.tagName = childTag;
    childRaw.parentElement = parent as unknown as Element;
    childRaw.setAttribute('data-ws', '');
    child.connectedCallback();

    assert.equal(
      childRaw[STREAMING_BOUNDARY_ACTIVATE](
        { child: true },
        unrelated as unknown as Element,
      ),
      ACTIVATION_ANCESTOR_BARRIER,
    );
    assert.equal(childRaw.$deferredSSR, true);
  });

  test('only one barrier is bypassed when barriers nest', () => {
    const outerTag = 'test-nested-bypass-outer';
    const innerTag = 'test-nested-bypass-inner';
    const childTag = 'test-nested-bypass-child';
    registerTemplate(outerTag);
    registerTemplate(innerTag);
    registerTemplate(childTag);

    const outer = new TemplateElement();
    const outerRaw = outer as unknown as {
      tagName: string;
      parentElement: Element | null;
      $deferredSSR: boolean;
    };
    outerRaw.tagName = outerTag;
    outerRaw.parentElement = null;
    outerRaw.$deferredSSR = true;

    const inner = new TemplateElement();
    const innerRaw = inner as unknown as {
      tagName: string;
      parentElement: Element | null;
      $deferredSSR: boolean;
    };
    innerRaw.tagName = innerTag;
    innerRaw.parentElement = outer as unknown as Element;
    innerRaw.$deferredSSR = true;

    const child = new TemplateElement();
    const childRaw = child as unknown as {
      tagName: string;
      parentElement: Element;
      $deferredSSR: boolean;
      setAttribute(name: string, value: string): void;
      [STREAMING_BOUNDARY_ACTIVATE](
        state?: Record<string, unknown>,
        bypassAncestor?: Element,
      ): number;
    };
    childRaw.tagName = childTag;
    childRaw.parentElement = inner as unknown as Element;
    childRaw.setAttribute('data-ws', '');
    child.connectedCallback();

    // `inner` is stepped over, but `outer` is still an unfinished barrier.
    assert.equal(
      childRaw[STREAMING_BOUNDARY_ACTIVATE](
        { child: true },
        inner as unknown as Element,
      ),
      ACTIVATION_ANCESTOR_BARRIER,
    );
    assert.equal(childRaw.$deferredSSR, true);
  });

  test('authored components do not globally defer unmarked SSR-shaped light DOM', () => {
    const el = new TemplateElement() as unknown as {
      $shouldDeferSSRHydration(): boolean;
    };
    assert.equal(el.$shouldDeferSSRHydration(), false);
  });

  test('reports missing metadata numerically and explicitly abandons internal deferral', () => {
    const el = new TemplateElement();
    const raw = el as unknown as {
      tagName: string;
      $deferredSSR: boolean;
      setAttribute(name: string, value: string): void;
      [STREAMING_BOUNDARY_ACTIVATE](state?: Record<string, unknown>): number;
      [STREAMING_BOUNDARY_ABANDON](): void;
    };
    raw.tagName = 'test-missing-activation-meta';
    raw.setAttribute('data-ws', '');
    el.connectedCallback();

    assert.equal(raw[STREAMING_BOUNDARY_ACTIVATE](), ACTIVATION_MISSING_TEMPLATE);
    assert.equal(raw.$deferredSSR, true);

    raw[STREAMING_BOUNDARY_ABANDON]();
    assert.equal(raw.$deferredSSR, false);
  });

  test('caches metadata before static-host opt-out so a later state write can wake it', () => {
    let activationMeta: TemplateMeta | undefined;
    class OptOutElement extends TemplateElement {
      protected override $shouldActivateOnBoundaryCommit(): boolean {
        return false;
      }

      protected override $afterExternalStateWrite(applied: boolean): void {
        if (applied) this.$activateDeferredSSR();
      }

      protected override $activateDeferredSSR(): void {
        activationMeta = (this as unknown as { $meta?: TemplateMeta }).$meta;
      }
    }
    const el = new OptOutElement();
    const raw = el as unknown as {
      tagName: string;
      $meta?: TemplateMeta;
      setAttribute(name: string, value: string): void;
      [STREAMING_BOUNDARY_ACTIVATE](): number;
    };
    raw.tagName = 'test-static-opt-out';
    registerTemplate(raw.tagName);
    window.__webui!.templates![raw.tagName].tr = ['message'];
    raw.setAttribute('data-ws', '');
    el.connectedCallback();

    assert.equal(raw[STREAMING_BOUNDARY_ACTIVATE](), ACTIVATION_STATIC_HOST_OPT_OUT);
    assert.ok(raw.$meta, 'boundary commit caches metadata without mounting');
    el.setState({ message: 'wake' });
    assert.equal(activationMeta, raw.$meta, 'the later state write can activate from cached metadata');
  });
});

describe('TemplateElement.define — ordinary (non-streaming) authored definition', () => {
  test('defers an eagerly authored define() until later-registered metadata completes it, even outside streaming mode', () => {
    // Ordinary WebUI Router partial navigation eagerly imports an authored
    // nested component module — which calls the compiler-emitted
    // `MyComponent.define(tag)` at top level — *before* the router has
    // registered that route's compiled template metadata. Native
    // `customElements.define()` snapshots `observedAttributes` at call time,
    // so defining now (as the old streaming-only guard did) would forever
    // miss template-only attributes like `title` below.
    const documentFake = document as unknown as {
      querySelector(selector: string): unknown;
    };
    const previousQuerySelector = documentFake.querySelector;
    documentFake.querySelector = () => null; // no streaming meta tag present
    resetStreamingModeForTests();

    const customElementsFake = customElements as unknown as {
      get(name: string): CustomElementConstructor | undefined;
      define?(name: string, ctor: CustomElementConstructor): void;
    };
    const previousGet = customElementsFake.get;
    const previousDefine = customElementsFake.define;
    const registry = new Map<string, CustomElementConstructor>();
    customElementsFake.get = (name: string) => registry.get(name);
    customElementsFake.define = (name: string, ctor: CustomElementConstructor) => {
      registry.set(name, ctor);
    };

    const tag = `test-router-nested-widget-${Date.now()}`;

    try {
      // No `@attr`/`@observable` for `title` — a template-only binding
      // entirely owned by compiled metadata (`tr`/`ta`).
      class AuthoredNestedWidget extends TemplateElement {}
      const AuthoredNestedWidgetCtor = AuthoredNestedWidget as unknown as {
        define(tagName: string): void;
      };

      AuthoredNestedWidgetCtor.define(tag);

      assert.equal(
        customElements.get(tag),
        undefined,
        'must stay pending — an ordinary (non-streaming) page must not define an incomplete observer surface either',
      );

      // A second eager define() call for the same still-pending tag must
      // preserve the existing duplicate-definition diagnostic.
      assert.throws(
        () => AuthoredNestedWidgetCtor.define(tag),
        /already pending definition/,
      );

      // The router registers this route's compiled template metadata once
      // its partial navigation response resolves.
      registerTemplateData({
        [tag]: {
          v: 3,
          h: '<div><span></span></div>',
          tr: ['title'],
          ta: ['title'],
        },
      });

      const ctor = customElements.get(tag) as (CustomElementConstructor & {
        observedAttributes?: string[];
      }) | undefined;
      assert.ok(ctor, 'the deferred definition must complete once metadata registers');
      assert.deepEqual(
        ctor!.observedAttributes,
        ['title'],
        'observedAttributes must include the template-derived attribute the browser would otherwise have missed',
      );

      const el = new (ctor as CustomElementConstructor)() as unknown as {
        attributeChangedCallback(name: string, oldValue: string | null, newValue: string | null): void;
        $templateState: Record<string, unknown> | null;
      };
      Object.defineProperty(el, 'localName', { value: tag });

      // A client-created host now carries `title` in `observedAttributes`,
      // so the browser calls `attributeChangedCallback` for it.
      el.attributeChangedCallback('title', null, 'Hello from router nav');

      assert.equal(
        el.$templateState?.title,
        'Hello from router nav',
        'the template-only binding must receive and update from the client-created host attribute',
      );
    } finally {
      documentFake.querySelector = previousQuerySelector;
      resetStreamingModeForTests();
      customElementsFake.get = previousGet;
      customElementsFake.define = previousDefine;
    }
  });
});

describe('TemplateElement — scoped state availability', () => {
  test('uses scope knownness when an item value is explicitly undefined', () => {
    const el = new TemplateElement();

    assert.equal(
      el.$hasStateRoot('item', {
        name: 'item',
        value: undefined,
        known: true,
      }),
      true,
    );
    assert.equal(
      el.$hasStateRoot('item.label', {
        name: 'item',
        value: 'trusted SSR value',
        known: false,
      }),
      false,
    );
  });
});

describe('TemplateElement — hydration lifecycle exceptions', () => {
  test('a real streamed activation throw balances lifecycle and does not wedge terminal completion', () => {
    class ThrowingStateElement extends TemplateElement {
      protected override $shouldApplySSRBootstrapState(): boolean {
        throw new Error('state hydration failed');
      }
    }

    const tag = 'test-throwing-hydration-widget';
    registerTemplate(tag);
    __resetLifecycleForTests();
    dispatchedEvents.length = 0;
    beginStreamingGate();
    markBoundaryPending();

    const el = new ThrowingStateElement();
    const raw = el as unknown as {
      tagName: string;
      childNodes: unknown[];
      setAttribute(name: string, value: string): void;
      [STREAMING_BOUNDARY_ACTIVATE](state?: Record<string, unknown>): number;
    };
    raw.tagName = tag;
    raw.childNodes.push({});
    raw.setAttribute('data-ws', '');
    el.connectedCallback();

    assert.throws(
      () => raw[STREAMING_BOUNDARY_ACTIVATE]({ count: 1 }),
      /state hydration failed/,
      'TemplateElement must not swallow the hydration error',
    );
    let state = __getLifecycleStateForTests();
    assert.equal(state.pendingCount, 0, 'the throwing activation balances hydrationStart');
    assert.equal(state.completed, false, 'terminal has not committed yet');

    markBoundaryCommitted(true);
    state = __getLifecycleStateForTests();
    assert.equal(state.completed, true, 'balanced accounting allows terminal completion');
    assert.equal(dispatchedEvents.includes('webui:hydration-complete'), true);
  });

  test('a reconnect update throw also balances hydration lifecycle', () => {
    class ThrowingReconnectElement extends TemplateElement {
      override [REQUEST_UPDATE](): void {
        throw new Error('reconnect update failed');
      }
    }

    __resetLifecycleForTests();
    dispatchedEvents.length = 0;
    const el = new ThrowingReconnectElement();
    const raw = el as unknown as {
      tagName: string;
      $hydrated: boolean;
      $root: object;
    };
    raw.tagName = 'test-throwing-reconnect-widget';
    raw.$hydrated = true;
    raw.$root = {};

    assert.throws(() => el.connectedCallback(), /reconnect update failed/);
    const state = __getLifecycleStateForTests();
    assert.equal(state.pendingCount, 0);
    assert.equal(state.completed, true);
  });
});

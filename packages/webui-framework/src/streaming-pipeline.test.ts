// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { strict as assert } from 'node:assert';
import { beforeEach, describe, test } from 'node:test';

/**
 * Coordinator pipeline tests.
 *
 * `streaming.ts` drives real DOM APIs (comment/element traversal and
 * `customElements`) that jsdom would normally provide, but this package's
 * unit tests run under `node --test`
 * with no browser and no added dependencies. So this file hand-rolls a
 * minimal DOM — just the handful of node properties the coordinator reads —
 * and installs it as globals before importing the module, mirroring the
 * dynamic-import-after-mock pattern used by the other tests here.
 *
 * The coordinator keeps its queue/halt/sequence state in module singletons,
 * so `__resetStreamingCoordinatorForTests()` runs before every test.
 */

// ── Minimal DOM node model ───────────────────────────────────────────

const ACTIVATE = Symbol.for('microsoft.webui.boundaryActivate');
const RESUME_PENDING = Symbol.for('microsoft.webui.pendingRootConnected');

interface FakeNode {
  nodeType: number;
  parentNode: FakeNode | null;
  firstChild: FakeNode | null;
  nextSibling: FakeNode | null;
  previousSibling: FakeNode | null;
  remove(): void;
  readonly previousElementSibling: FakeNode | null;
  readonly nextElementSibling: FakeNode | null;
}

interface FakeElement extends FakeNode {
  tagName: string;
  shadowRoot: FakeNode | null;
  readonly isConnected: boolean;
  hasAttribute(name: string): boolean;
  setAttribute(name: string, value: string): void;
  removeAttribute(name: string): void;
  readonly textContent: string;
  [ACTIVATE]?: (state?: Record<string, unknown>) => void;
  [RESUME_PENDING]?: () => void;
}

let elementRegistry: FakeElement[] = [];

function detach(node: FakeNode): void {
  const parent = node.parentNode;
  const prev = node.previousSibling;
  const next = node.nextSibling;
  if (prev) prev.nextSibling = next;
  if (next) next.previousSibling = prev;
  if (parent && parent.firstChild === node) parent.firstChild = next;
  node.parentNode = null;
  node.previousSibling = null;
  node.nextSibling = null;
}

function addSiblingGetters(node: FakeNode): void {
  Object.defineProperty(node, 'previousElementSibling', {
    get(): FakeNode | null {
      let n = node.previousSibling;
      while (n && n.nodeType !== 1) n = n.previousSibling;
      return n ?? null;
    },
    configurable: true,
  });
  Object.defineProperty(node, 'nextElementSibling', {
    get(): FakeNode | null {
      let n = node.nextSibling;
      while (n && n.nodeType !== 1) n = n.nextSibling;
      return n ?? null;
    },
    configurable: true,
  });
}

function comment(data: string): FakeNode & { data: string } {
  const node = {
    nodeType: 8,
    data,
    parentNode: null,
    firstChild: null,
    nextSibling: null,
    previousSibling: null,
    remove(): void {
      detach(node);
    },
  } as FakeNode & { data: string };
  addSiblingGetters(node);
  return node;
}

interface ElementSpec {
  attrs?: Record<string, string>;
  text?: string;
  hook?: (state?: Record<string, unknown>) => void;
  children?: Array<FakeNode>;
  shadowChildren?: Array<FakeNode>;
}

function element(tagName: string, spec: ElementSpec = {}): FakeElement {
  const attrs = { ...(spec.attrs ?? {}) };
  const node = {
    nodeType: 1,
    tagName: tagName.toUpperCase(),
    parentNode: null,
    firstChild: null,
    nextSibling: null,
    previousSibling: null,
    shadowRoot: null,
    _children: spec.children ?? [],
    hasAttribute(name: string): boolean {
      return Object.prototype.hasOwnProperty.call(attrs, name);
    },
    setAttribute(name: string, value: string): void {
      attrs[name] = value;
    },
    removeAttribute(name: string): void {
      delete attrs[name];
    },
    get textContent(): string {
      return spec.text ?? '';
    },
    get isConnected(): boolean {
      return node.parentNode !== null;
    },
    remove(): void {
      detach(node);
    },
  } as unknown as FakeElement & { _children: FakeNode[] };
  addSiblingGetters(node);
  if (spec.hook) node[ACTIVATE] = spec.hook;
  if (spec.shadowChildren) {
    const shadowRoot = {
      nodeType: 11,
      host: node,
      parentNode: null,
      firstChild: null,
      nextSibling: null,
      previousSibling: null,
      remove(): void {
        /* shadow roots are not removable boundary scaffolding */
      },
    } as unknown as FakeNode;
    addSiblingGetters(shadowRoot);
    link(shadowRoot, spec.shadowChildren);
    node.shadowRoot = shadowRoot;
  }
  elementRegistry.push(node);
  return node;
}

/** Link a flat list of children under `parent`, recursively linking any
 *  element's own `_children` and setting its `firstChild`. */
function link(parent: FakeNode, nodes: FakeNode[]): void {
  parent.firstChild = nodes[0] ?? null;
  for (let i = 0; i < nodes.length; i++) {
    const n = nodes[i] as FakeNode & { _children?: FakeNode[] };
    n.parentNode = parent;
    n.previousSibling = nodes[i - 1] ?? null;
    n.nextSibling = nodes[i + 1] ?? null;
    if (n._children && n._children.length) link(n, n._children);
  }
}

function body(): FakeNode {
  return {
    nodeType: 1,
    parentNode: null,
    firstChild: null,
    nextSibling: null,
    previousSibling: null,
    remove(): void {
      /* body is never removed */
    },
  } as unknown as FakeNode;
}

// ── Custom-element + document fakes ──────────────────────────────────

let definedTags: Map<string, unknown>;
let whenDefinedResolvers: Map<string, () => void>;
let whenDefinedCalls: Map<string, number>;
let upgradeCalls: Map<string, number>;
let dispatchedEvents: Array<{ type: string; detail?: unknown }>;
let documentListeners: Map<string, Array<{ handler: () => void; once: boolean }>>;

function installGlobals(): void {
  definedTags = new Map();
  whenDefinedResolvers = new Map();
  whenDefinedCalls = new Map();
  upgradeCalls = new Map();
  dispatchedEvents = [];
  documentListeners = new Map();

  const win = {
    __webui: {} as Record<string, unknown>,
    __WEBUI_STREAMING_DEBUG__: false,
    dispatchEvent(event: { type: string; detail?: unknown }): boolean {
      dispatchedEvents.push(event);
      return true;
    },
  };

  const customElementsFake = {
    get(tag: string): unknown {
      return definedTags.get(tag);
    },
    define(tag: string, ctor: unknown): void {
      definedTags.set(tag, ctor);
    },
    whenDefined(tag: string): Promise<void> {
      whenDefinedCalls.set(tag, (whenDefinedCalls.get(tag) ?? 0) + 1);
      return new Promise<void>((resolve) => {
        whenDefinedResolvers.set(tag, resolve);
      });
    },
    upgrade(root: FakeElement): void {
      const tag = root.tagName.toLowerCase();
      upgradeCalls.set(tag, (upgradeCalls.get(tag) ?? 0) + 1);
      if (root[ACTIVATE]) return;
      const ctor = definedTags.get(tag) as { prototype?: { [ACTIVATE]?: (state?: Record<string, unknown>) => void } } | undefined;
      const hook = ctor?.prototype?.[ACTIVATE];
      if (hook) root[ACTIVATE] = hook;
    },
  };

  const documentFake = {
    readyState: 'loading',
    getElementsByTagName(tag: string): FakeElement[] {
      const upper = tag.toUpperCase();
      return elementRegistry.filter((el) => el.tagName === upper && el.parentNode !== null);
    },
    addEventListener(type: string, handler: () => void, opts?: { once?: boolean }): void {
      const list = documentListeners.get(type) ?? [];
      list.push({ handler, once: opts?.once === true });
      documentListeners.set(type, list);
    },
  };

  class FakeEvent {
    type: string;
    constructor(type: string) {
      this.type = type;
    }
  }
  class FakeCustomEvent {
    type: string;
    detail: unknown;
    constructor(type: string, init?: { detail?: unknown }) {
      this.type = type;
      this.detail = init?.detail;
    }
  }

  define('window', win);
  define('customElements', customElementsFake);
  define('document', documentFake);
  define('Event', FakeEvent);
  define('CustomEvent', FakeCustomEvent);
  define('HTMLElement', class HTMLElement {});
}

function define(name: string, value: unknown): void {
  Object.defineProperty(globalThis, name, { value, configurable: true, writable: true });
}

/** Resolve a tag's `whenDefined` promise, as `customElements.define` would. */
function defineTag(
  tag: string,
  supportsDetachedResume = true,
  hook: (state?: Record<string, unknown>) => void = () => {},
): void {
  class DefinedElement {}
  if (supportsDetachedResume) {
    (DefinedElement.prototype as unknown as { [ACTIVATE]?: (state?: Record<string, unknown>) => void })[ACTIVATE] = hook;
  }
  definedTags.set(tag, DefinedElement);
  const resolve = whenDefinedResolvers.get(tag);
  if (resolve) resolve();
}

/** Fire the fake document's `DOMContentLoaded`, honoring `{ once: true }`. */
function fireDomContentLoaded(): void {
  const list = documentListeners.get('DOMContentLoaded') ?? [];
  const survivors: Array<{ handler: () => void; once: boolean }> = [];
  for (const entry of list) {
    entry.handler();
    if (!entry.once) survivors.push(entry);
  }
  documentListeners.set('DOMContentLoaded', survivors);
}

function setDocumentReadyState(readyState: 'loading' | 'interactive' | 'complete'): void {
  (document as unknown as { readyState: string }).readyState = readyState;
}

async function flush(): Promise<void> {
  await new Promise<void>((r) => queueMicrotask(r));
  await new Promise<void>((r) => queueMicrotask(r));
}

function boundaryEvents(): Array<{ type: string; detail?: unknown }> {
  return dispatchedEvents.filter((e) => e.type === 'webui:boundary-hydrated');
}

installGlobals();

const {
  __resetStreamingCoordinatorForTests,
  __enqueueSentinelForTests,
  __isHaltedForTests,
  __pendingTagWaiterCountForTests,
  __pendingUndefinedRootCountForTests,
  __elementHasPendingStateForTests,
  __installTruncationGuardForTests,
} = await import('./streaming.js');

const {
  beginStreamingGate,
  __resetLifecycleForTests,
  __getLifecycleStateForTests,
} = await import('./lifecycle.js');

/** Drive a fake sentinel through the coordinator's real pump. */
function enqueue(sentinel: FakeElement): void {
  __enqueueSentinelForTests(sentinel as unknown as Element);
}

/** Introspect stashed pending-boundary state on a fake element. */
function hasPending(el: FakeElement): boolean {
  return __elementHasPendingStateForTests(el as unknown as Element);
}

/** Whether a fake element still advertises itself as a streamed SSR host. */
function hasWs(el: FakeElement): boolean {
  return el.hasAttribute('data-ws');
}

/** The live `window.__webui` object the coordinator writes its handoff into. */
function webuiGlobal(): Record<string, unknown> {
  return (globalThis as unknown as { window: { __webui: Record<string, unknown> } }).window.__webui;
}

/** Mark tags as already-defined so their roots activate synchronously. */
function predefine(...tags: string[]): void {
  for (const tag of tags) definedTags.set(tag, class {});
}

// ── Boundary DOM builders ────────────────────────────────────────────

interface BuiltBoundary {
  root: FakeNode;
  sentinel: FakeElement;
  scriptEl: FakeElement;
  startMarker: (FakeNode & { data: string }) | null;
  endMarker: (FakeNode & { data: string }) | null;
  roots: FakeElement[];
}

/** Mirror the server's root marker for fake WebUI components. A custom element
 * without the branded activation hook remains unmarked, modeling unrelated
 * third-party custom elements inside a boundary. */
function markStreamedRoots(nodes: FakeNode[]): void {
  const pending = [...nodes];
  while (pending.length !== 0) {
    const node = pending.pop() as FakeNode & { _children?: FakeNode[] };
    if (node.nodeType === 1) {
      const el = node as FakeElement;
      if (typeof el[ACTIVATE] === 'function') el.setAttribute('data-ws', '');
      let shadowChild = el.shadowRoot?.firstChild ?? null;
      while (shadowChild) {
        pending.push(shadowChild);
        shadowChild = shadowChild.nextSibling;
      }
    }
    if (node._children) {
      for (let i = 0; i < node._children.length; i++) pending.push(node._children[i]);
    }
  }
}

/** Build a boundary with a marker pair wrapping `roots`. */
function buildBoundary(sequence: number, terminal: number, roots: FakeElement[], bootstrap: object): BuiltBoundary {
  markStreamedRoots(roots);
  const start = comment(`wb:${sequence}`);
  const end = comment(`/wb:${sequence}`);
  const scriptEl = element('script', {
    attrs: { 'data-webui-boundary': '' },
    text: JSON.stringify([1, sequence, terminal, bootstrap]),
  });
  const sentinel = element('webui-hydrate');
  const root = body();
  link(root, [start, ...roots, end, scriptEl, sentinel]);
  return { root, sentinel, scriptEl, startMarker: start, endMarker: end, roots };
}

/** Build a markerless boundary (no new roots — tail/terminal resync). */
function buildMarkerless(sequence: number, terminal: number, bootstrap: object): BuiltBoundary {
  const scriptEl = element('script', {
    attrs: { 'data-webui-boundary': '' },
    text: JSON.stringify([1, sequence, terminal, bootstrap]),
  });
  const sentinel = element('webui-hydrate');
  const root = body();
  link(root, [scriptEl, sentinel]);
  return { root, sentinel, scriptEl, startMarker: null, endMarker: null, roots: [] };
}

/** Build a boundary whose payload script carries arbitrary (possibly
 *  malformed) text, while keeping a real marker pair around `roots`. Used to
 *  prove that a rejected boundary's scaffolding is still fully cleaned. */
function buildRawBoundary(sequence: number, rawText: string, roots: FakeElement[]): BuiltBoundary {
  markStreamedRoots(roots);
  const start = comment(`wb:${sequence}`);
  const end = comment(`/wb:${sequence}`);
  const scriptEl = element('script', { attrs: { 'data-webui-boundary': '' }, text: rawText });
  const sentinel = element('webui-hydrate');
  const root = body();
  link(root, [start, ...roots, end, scriptEl, sentinel]);
  return { root, sentinel, scriptEl, startMarker: start, endMarker: end, roots };
}

/** Assert a boundary left no discoverable scaffolding (sentinel, payload
 *  script, and marker pair all detached) while every SSR root remains in the
 *  tree — the invariant every reject path must uphold. */
function assertScaffoldCleaned(b: BuiltBoundary): void {
  assert.equal(b.sentinel.parentNode, null, 'sentinel removed');
  assert.equal(b.scriptEl.parentNode, null, 'payload script removed');
  assert.equal(b.startMarker?.parentNode ?? null, null, 'start marker removed');
  assert.equal(b.endMarker?.parentNode ?? null, null, 'end marker removed');
  for (const r of b.roots) {
    assert.equal(r.parentNode, b.root, 'SSR root retained');
  }
}

// ── Tests ────────────────────────────────────────────────────────────

describe('streaming coordinator pipeline', () => {
  beforeEach(() => {
    installGlobals();
    elementRegistry = [];
    // Reset coordinator first (it abandons any waiters, settling their
    // late-activation counts), then zero the shared lifecycle, then reopen
    // the streaming gate so completion is boundary/late-activation-driven.
    __resetStreamingCoordinatorForTests();
    __resetLifecycleForTests();
    beginStreamingGate();
  });

  test('commits a boundary: activates its root and removes all scaffolding', async () => {
    const activated: string[] = [];
    const counter = element('my-counter', { hook() { activated.push('my-counter'); } });
    const b = buildBoundary(0, 0, [counter], { state: { n: 1 } });

    predefine('my-counter');
    enqueue(b.sentinel);
    await flush();

    assert.deepEqual(activated, ['my-counter']);
    assert.equal(__isHaltedForTests(), false);
    // Scaffolding gone; the activated root itself stays in the tree.
    assert.equal(b.sentinel.parentNode, null, 'sentinel removed');
    assert.equal(b.scriptEl.parentNode, null, 'payload script removed');
    assert.equal(b.startMarker?.parentNode, null, 'start marker removed');
    assert.equal(b.endMarker?.parentNode, null, 'end marker removed');
    assert.equal(counter.parentNode, b.root, 'activated root retained');
  });

  test('merges inventory, css, and style checkpoint deltas cumulatively', async () => {
    const first = buildBoundary(0, 0, [], {
      inventory: '01',
      css: ['a.css'],
      styles: ['my-a'],
    });
    enqueue(first.sentinel);
    await flush();

    const second = buildBoundary(1, 0, [], {
      inventory: '02',
      css: ['b.css'],
      styles: ['my-b'],
    });
    enqueue(second.sentinel);
    await flush();

    assert.equal(webuiGlobal().inventory, '03', '01 OR 02 is retained as compact 03');
    assert.deepEqual(webuiGlobal().css, ['a.css', 'b.css']);
    assert.deepEqual(webuiGlobal().styles, ['my-a', 'my-b']);

    const repeated = buildBoundary(2, 0, [], {
      inventory: '01',
      css: ['a.css'],
      styles: ['my-a'],
    });
    enqueue(repeated.sentinel);
    await flush();
    assert.equal(webuiGlobal().inventory, '03', 'a repeated delta cannot erase prior bits');
    assert.deepEqual(webuiGlobal().css, ['a.css', 'b.css'], 'a repeated CSS delta is deduplicated');
    assert.deepEqual(webuiGlobal().styles, ['my-a', 'my-b'], 'a repeated style delta is deduplicated');

    const empty = buildMarkerless(3, 1, { inventory: '' });
    enqueue(empty.sentinel);
    await flush();
    assert.equal(webuiGlobal().inventory, '03', 'an empty delta cannot erase prior bits');
  });

  test('ORs inventory bytes by position when checkpoint lengths differ', async () => {
    const first = buildBoundary(0, 0, [], { inventory: '01' });
    const second = buildMarkerless(1, 1, { inventory: '0002' });
    enqueue(first.sentinel);
    await flush();
    enqueue(second.sentinel);
    await flush();

    assert.equal(webuiGlobal().inventory, '0102', 'short inventory strings are padded on the right');
  });

  test('accepts uppercase and lowercase ASCII inventory hex', async () => {
    const boundary = buildMarkerless(0, 1, { inventory: '0123456789abcdefABCDEF' });
    enqueue(boundary.sentinel);
    await flush();

    assert.equal(__isHaltedForTests(), false);
    assert.equal(webuiGlobal().inventory, '0123456789abcdefabcdef');
  });

  test('rejects non-ASCII-hex inventory characters deterministically', async () => {
    // Characters immediately around each accepted ASCII range, plus whitespace
    // and non-ASCII digits, guard the explicit char-code checks.
    const invalidInventories = ['0/', '0:', '0@', '0G', '0`', '0g', '0 ', '0é', '０0'];
    const previousError = console.error;
    console.error = () => {};
    try {
      for (let i = 0; i < invalidInventories.length; i++) {
        if (i > 0) {
          installGlobals();
          elementRegistry = [];
          __resetStreamingCoordinatorForTests();
          __resetLifecycleForTests();
          beginStreamingGate();
        }
        const boundary = buildMarkerless(0, 1, { inventory: invalidInventories[i] });
        enqueue(boundary.sentinel);
        await flush();
        assert.equal(__isHaltedForTests(), true, `${JSON.stringify(invalidInventories[i])} must be rejected`);
        assert.equal(__getLifecycleStateForTests().completed, false);
      }
    } finally {
      console.error = previousError;
    }
  });

  test('boundary-hydrated event is opt-in via the debug flag', async () => {
    const b1 = buildBoundary(0, 1, [element('my-counter', { hook() {} })], { state: {} });
    enqueue(b1.sentinel);
    await flush();
    assert.equal(boundaryEvents().length, 0, 'no event without the debug flag');

    // With the flag on, exactly one CustomEvent carrying {sequence, terminal}.
    __resetStreamingCoordinatorForTests();
    (globalThis as unknown as { window: { __WEBUI_STREAMING_DEBUG__: boolean } }).window.__WEBUI_STREAMING_DEBUG__ = true;
    const b2 = buildBoundary(0, 1, [element('my-counter', { hook() {} })], { state: {} });
    enqueue(b2.sentinel);
    await flush();

    const evts = boundaryEvents();
    assert.equal(evts.length, 1);
    assert.deepEqual(evts[0].detail, { sequence: 0, terminal: true });
  });

  test('a malformed envelope halts and fully cleans its scaffold, keeping roots', async () => {
    const root0 = element('my-root', { hook() {} });
    const b = buildRawBoundary(0, '[1,0,0,{', [root0]);

    enqueue(b.sentinel);
    await flush();
    assert.equal(__isHaltedForTests(), true, 'invalid JSON must halt');
    // The rejected boundary leaves no discoverable payload/markers, but its
    // SSR root stays in the tree.
    assertScaffoldCleaned(b);

    // A boundary enqueued after the halt is dropped and fully cleaned too.
    const later = buildBoundary(1, 0, [element('my-counter', { hook() {} })], { state: {} });
    enqueue(later.sentinel);
    await flush();
    assertScaffoldCleaned(later);
  });

  test('an out-of-order sequence halts and fully cleans its scaffold', async () => {
    // Expecting sequence 0, but the first boundary claims sequence 1.
    const b = buildBoundary(1, 0, [element('my-counter', { hook() {} })], { state: {} });
    enqueue(b.sentinel);
    await flush();
    assert.equal(__isHaltedForTests(), true);
    assertScaffoldCleaned(b);
  });

  test('a missing start marker halts and cleans the end marker + payload', async () => {
    // End marker present but no matching start marker: reject + clean.
    const end = comment('/wb:0');
    const scriptEl = element('script', { attrs: { 'data-webui-boundary': '' }, text: JSON.stringify([1, 0, 0, {}]) });
    const sentinel = element('webui-hydrate');
    const root = body();
    link(root, [end, scriptEl, sentinel]);

    enqueue(sentinel);
    await flush();
    assert.equal(__isHaltedForTests(), true, 'missing start marker must halt');
    assert.equal(sentinel.parentNode, null, 'sentinel removed');
    assert.equal(scriptEl.parentNode, null, 'payload script removed');
    assert.equal(end.parentNode, null, 'orphan end marker removed');
  });

  test('a missing end marker halts and releases roots instead of accepting a markerless checkpoint', async () => {
    const root0 = element('my-missing-end', { hook() {} });
    const b = buildBoundary(0, 0, [root0], { state: {} });
    b.endMarker?.remove();

    enqueue(b.sentinel);
    await flush();

    assert.equal(__isHaltedForTests(), true, 'a nonterminal checkpoint must have an end marker');
    assertScaffoldCleaned(b);
    assert.equal(hasWs(root0), false, 'the orphaned boundary root must not remain deferred');
  });

  test('one root activation failure is isolated; later roots still activate', async () => {
    const activated: string[] = [];
    const bad = element('my-bad', {
      hook() {
        throw new Error('mount failed');
      },
    });
    const good = element('my-good', { hook() { activated.push('my-good'); } });
    const b = buildBoundary(0, 0, [bad, good], { state: {} });

    predefine('my-bad', 'my-good');
    enqueue(b.sentinel);
    await flush();

    assert.deepEqual(activated, ['my-good'], 'later root activates despite earlier failure');
    assert.equal(__isHaltedForTests(), false, 'a component mount failure must not halt the stream');
    assert.equal(b.sentinel.parentNode, null, 'scaffolding still removed');
  });

  test('a markerless terminal boundary commits without activation', async () => {
    const b = buildMarkerless(0, 1, {});
    enqueue(b.sentinel);
    await flush();

    assert.equal(__isHaltedForTests(), false);
    assert.equal(b.sentinel.parentNode, null, 'sentinel removed');
    assert.equal(b.scriptEl.parentNode, null, 'payload script removed');
  });

  test('a nested island is activated exactly once in a single walk', async () => {
    const activated: string[] = [];
    const inner = element('my-inner', { hook() { activated.push('my-inner'); } });
    const outer = element('my-outer', {
      hook() {
        activated.push('my-outer');
      },
      children: [inner],
    });
    const b = buildBoundary(0, 0, [outer], { state: {} });

    predefine('my-outer', 'my-inner');
    enqueue(b.sentinel);
    await flush();

    assert.deepEqual(activated.sort(), ['my-inner', 'my-outer']);
    assert.equal(activated.length, 2, 'each nested root activated exactly once');
  });

  test('nested islands inside an open declarative shadow root activate exactly once', async () => {
    const activated: string[] = [];
    const inner = element('my-shadow-inner', { hook() { activated.push('my-shadow-inner'); } });
    const outer = element('my-shadow-outer', {
      hook() {
        activated.push('my-shadow-outer');
      },
      shadowChildren: [inner],
    });
    const b = buildBoundary(0, 0, [outer], { state: {} });

    predefine('my-shadow-outer', 'my-shadow-inner');
    enqueue(b.sentinel);
    await flush();

    assert.deepEqual(activated, ['my-shadow-outer', 'my-shadow-inner']);
    assert.equal(hasWs(outer), false);
    assert.equal(hasWs(inner), false);
  });

  test('an unmarked third-party custom element is ignored by streaming activation', async () => {
    const activated: string[] = [];
    const thirdParty = element('lazy-icon');
    const webuiRoot = element('my-owned', {
      hook() {
        activated.push('my-owned');
      },
      children: [thirdParty],
    });
    const b = buildBoundary(0, 1, [webuiRoot], { state: {} });

    predefine('my-owned');
    enqueue(b.sentinel);
    await flush();

    assert.deepEqual(activated, ['my-owned']);
    assert.equal(whenDefinedCalls.get('lazy-icon'), undefined, 'unmarked descendants create no waiter');
    assert.equal(__pendingTagWaiterCountForTests(), 0);
    assert.equal(__pendingUndefinedRootCountForTests(), 0);
    assert.equal(__getLifecycleStateForTests().completed, true);
  });

  test('undefined tags register one waiter per unique tag and activate on define', async () => {
    const activated: string[] = [];
    // Two instances of the same not-yet-defined tag.
    const a = element('my-late', { hook() { activated.push('a'); } });
    const c = element('my-late', { hook() { activated.push('c'); } });
    const b = buildBoundary(0, 1, [a, c], { state: {} });

    enqueue(b.sentinel);
    await flush();

    assert.deepEqual(activated, [], 'undefined tag is not activated until its class defines');
    assert.equal(whenDefinedCalls.get('my-late'), 1, 'exactly one waiter for the unique undefined tag');
    // The terminal boundary committed, but the late activation holds the gate
    // open, so hydration is not yet complete.
    let lc = __getLifecycleStateForTests();
    assert.equal(lc.completed, false, 'completion is gated on the late activation');
    assert.equal(lc.pendingLateActivations, 1, 'one late-activation wait outstanding');
    assert.equal(__pendingTagWaiterCountForTests(), 1, 'one tag waiter outstanding');

    defineTag('my-late');
    await flush();
    assert.deepEqual(activated.sort(), ['a', 'c'], 'both deferred instances activate once defined');
    lc = __getLifecycleStateForTests();
    assert.equal(lc.pendingLateActivations, 0, 'late-activation settled exactly once');
    assert.equal(__pendingTagWaiterCountForTests(), 0, 'waiter cleared');
    assert.equal(lc.completed, true, 'hydration completes once the late activation resolves');
    assert.equal(hasPending(a), false, 'stashed state consumed');
    assert.equal(hasPending(c), false, 'stashed state consumed');
  });

  test('one tag waiter activates more than 10k exact roots across boundaries', async () => {
    let activated = 0;
    const activate = () => { activated++; };
    const firstRoots: FakeElement[] = [];
    const secondRoots: FakeElement[] = [];
    for (let i = 0; i < 6_000; i++) {
      firstRoots.push(element('my-many', { hook: activate }));
      secondRoots.push(element('my-many', { hook: activate }));
    }

    const first = buildBoundary(0, 0, firstRoots, { state: { boundary: 0 } });
    enqueue(first.sentinel);
    await flush();
    const second = buildBoundary(1, 1, secondRoots, { state: { boundary: 1 } });
    enqueue(second.sentinel);
    await flush();

    assert.equal(whenDefinedCalls.get('my-many'), 1, 'one native promise is shared by the tag');
    assert.equal(__pendingUndefinedRootCountForTests(), 12_000, 'all exact roots remain tracked');

    defineTag('my-many');
    await flush();

    assert.equal(activated, 12_000, 'the waiter has no one-shot 10k truncation');
    assert.equal(__pendingUndefinedRootCountForTests(), 0);
    assert.equal(__pendingTagWaiterCountForTests(), 0);
    assert.equal(__getLifecycleStateForTests().completed, true);
  });

  test('definition upgrades and activates an exact root that remains detached', async () => {
    let received: Record<string, unknown> | undefined;
    const late = element('my-detached', { attrs: { 'data-ws': '' } });
    const b = buildBoundary(0, 1, [late], { state: { exact: true } });
    enqueue(b.sentinel);
    await flush();
    detach(late);

    defineTag('my-detached', true, (state) => {
      received = state;
    });
    await flush();

    assert.equal(late.isConnected, false, 'the root remains detached throughout definition');
    assert.equal(upgradeCalls.get('my-detached'), 1, 'the detached exact root is explicitly upgraded');
    assert.deepEqual(received, { exact: true }, 'the installed hook receives the stashed state');
    assert.equal(hasPending(late), false);
    assert.equal(hasWs(late), false);
    assert.equal(__pendingTagWaiterCountForTests(), 0);
    assert.equal(__pendingUndefinedRootCountForTests(), 0);
    assert.equal(__getLifecycleStateForTests().completed, true);
  });

  test('a detached root reattached before whenDefined reacts resumes synchronously', async () => {
    const late = element('my-reattach', { attrs: { 'data-ws': '' } });
    const b = buildBoundary(0, 1, [late], { state: { resumed: true } });
    enqueue(b.sentinel);
    await flush();
    detach(late);

    let received: Record<string, unknown> | undefined;
    defineTag('my-reattach', true, (state) => {
      received = state;
    });
    // Definition queues the whenDefined reaction. Reattachment can upgrade and
    // connect the element synchronously before that reaction gets its turn.
    const reattachedBody = body();
    link(reattachedBody, [late]);
    customElements.upgrade(late as unknown as Element);
    assert.equal(typeof late[RESUME_PENDING], 'function', 'one shared reconnect seam is installed');
    late[RESUME_PENDING]!();
    await flush();

    assert.deepEqual(received, { resumed: true });
    assert.equal(hasPending(late), false);
    assert.equal(hasWs(late), false);
    assert.equal(__pendingTagWaiterCountForTests(), 0);
    assert.equal(__pendingUndefinedRootCountForTests(), 0);
    assert.equal(__getLifecycleStateForTests().completed, true);
    assert.equal(upgradeCalls.get('my-reattach'), 1, 'the whenDefined reaction does not upgrade it again');
  });

  test('a detached root whose defined class cannot resume is cleaned as a stream failure', async () => {
    const late = element('my-no-resume', { attrs: { 'data-ws': '' } });
    const b = buildBoundary(0, 1, [late], { state: { abandoned: true } });
    enqueue(b.sentinel);
    await flush();
    detach(late);

    const previousError = console.error;
    console.error = () => {};
    try {
      defineTag('my-no-resume', false);
      await flush();
    } finally {
      console.error = previousError;
    }

    const lc = __getLifecycleStateForTests();
    assert.equal(__isHaltedForTests(), true);
    assert.equal(lc.streamingGateAborted, true);
    assert.equal(lc.completed, false);
    assert.equal(lc.pendingLateActivations, 0);
    assert.equal(__pendingTagWaiterCountForTests(), 0);
    assert.equal(__pendingUndefinedRootCountForTests(), 0);
    assert.equal(hasPending(late), false);
    assert.equal(hasWs(late), false);
    assert.equal(upgradeCalls.get('my-no-resume'), 1);
  });

  test('a halted stream removes a sentinel that arrives afterward', async () => {
    // Force halt via a missing payload script.
    const orphan = element('webui-hydrate');
    const root = body();
    link(root, [orphan]);
    enqueue(orphan);
    await flush();
    assert.equal(__isHaltedForTests(), true);

    const late = element('webui-hydrate');
    const root2 = body();
    link(root2, [late]);
    enqueue(late);
    await flush();
    assert.equal(late.parentNode, null, 'sentinel dropped and removed after halt');

    // A full boundary arriving after the halt has its adjacent payload/markers
    // cleaned too — not just the bare sentinel — while its root survives.
    const post = buildBoundary(1, 0, [element('my-post', { hook() {} })], { state: {} });
    enqueue(post.sentinel);
    await flush();
    assertScaffoldCleaned(post);
  });

  test('fail() abandons undefined-tag waiters without double-settling on later define', async () => {
    // A committed non-terminal boundary with an undefined tag, then a halt
    // from a malformed follow-up. The undefined tag must be abandoned: its
    // stashed state cleared, its late-activation settled exactly once, and a
    // later class definition must neither activate nor underflow the count.
    const activated: string[] = [];
    const late = element('my-abandon', { hook() { activated.push('x'); } });
    const b0 = buildBoundary(0, 0, [late], { state: {} });
    enqueue(b0.sentinel);
    await flush();
    assert.equal(__pendingTagWaiterCountForTests(), 1, 'one waiter registered');
    assert.equal(__getLifecycleStateForTests().pendingLateActivations, 1);

    // Halt via a malformed boundary at the next sequence.
    const bad = buildRawBoundary(1, '[1,1,0,{', []);
    enqueue(bad.sentinel);
    await flush();
    assert.equal(__isHaltedForTests(), true);

    // Abandonment balanced the lifecycle exactly once and cleared bookkeeping.
    const lc = __getLifecycleStateForTests();
    assert.equal(__pendingTagWaiterCountForTests(), 0, 'waiter set cleared on fail');
    assert.equal(lc.pendingLateActivations, 0, 'late-activation settled once on fail');
    assert.equal(hasPending(late), false, 'stashed state cleared on fail');

    // The uncancellable whenDefined promise resolves later: must not activate
    // the abandoned root nor underflow the settled count.
    defineTag('my-abandon');
    await flush();
    assert.deepEqual(activated, [], 'abandoned root is never activated');
    assert.equal(__getLifecycleStateForTests().pendingLateActivations, 0, 'no underflow after late define');
    assert.equal(__pendingTagWaiterCountForTests(), 0);
  });

  test('a failed terminal commit does not dispatch hydration-complete', async () => {
    // A terminal boundary whose bootstrap application throws: the stream halts
    // and, crucially, the terminal is NOT marked reached, so completion never
    // fires for a failed stream.
    const win = (globalThis as unknown as { window: { __webui: Record<string, unknown> } }).window;
    Object.defineProperty(win.__webui, 'boom', {
      configurable: true,
      set() {
        throw new Error('bootstrap merge failed');
      },
    });

    const b = buildMarkerless(0, 1, { boom: 1 });
    enqueue(b.sentinel);
    await flush();

    assert.equal(__isHaltedForTests(), true, 'a throwing terminal commit halts');
    const lc = __getLifecycleStateForTests();
    assert.equal(lc.terminalReached, false, 'terminal must not be marked reached on a failed commit');
    assert.equal(lc.completed, false, 'no hydration-complete for a failed stream');
    assert.equal(lc.pendingBoundaries, 0, 'the pending boundary still settles');
    assert.equal(dispatchedEvents.some((e) => e.type === 'webui:hydration-complete'), false);
    // Scaffolding is still removed even on the failed commit path.
    assert.equal(b.sentinel.parentNode, null, 'sentinel removed');
    assert.equal(b.scriptEl.parentNode, null, 'payload script removed');
  });

  test('a successful terminal commit dispatches hydration-complete', async () => {
    const b = buildMarkerless(0, 1, {});
    enqueue(b.sentinel);
    await flush();

    const lc = __getLifecycleStateForTests();
    assert.equal(__isHaltedForTests(), false);
    assert.equal(lc.terminalReached, true, 'terminal marked reached on success');
    assert.equal(lc.completed, true, 'hydration completes');
    assert.equal(dispatchedEvents.some((e) => e.type === 'webui:hydration-complete'), true);
  });

  test('queue overflow cleans the overflowing boundary and halts', async () => {
    // Saturate the queue without draining, then push one more real boundary:
    // the overflowing boundary's scaffold must be cleaned synchronously.
    const MAX = 512;
    for (let i = 0; i < MAX; i++) {
      const filler = element('webui-hydrate');
      const root = body();
      link(root, [filler]);
      // Enqueue directly without flushing so the queue never drains.
      __enqueueSentinelForTests(filler as unknown as Element);
    }
    const overflow = buildBoundary(0, 0, [element('my-overflow', { hook() {} })], { state: {} });
    __enqueueSentinelForTests(overflow.sentinel as unknown as Element);

    assert.equal(__isHaltedForTests(), true, 'overflow halts the coordinator');
    assertScaffoldCleaned(overflow);
  });

  test('an element-cap breach aborts instead of partially committing', async () => {
    const roots: FakeElement[] = [];
    for (let i = 0; i < 10_001; i++) {
      roots.push(element('my-capped', { attrs: { 'data-ws': '' }, hook() {} }));
    }
    const b = buildBoundary(0, 1, roots, { state: {} });
    (window as unknown as { __WEBUI_STREAMING_DEBUG__: boolean }).__WEBUI_STREAMING_DEBUG__ = true;
    const previousError = console.error;
    console.error = () => {};
    try {
      enqueue(b.sentinel);
      await flush();
    } finally {
      console.error = previousError;
    }

    const lc = __getLifecycleStateForTests();
    assert.equal(__isHaltedForTests(), true, 'the cap is a stream failure');
    assertScaffoldCleaned(b);
    assert.equal(hasPending(roots[0]), false, 'stashed state from the partial walk is released');
    assert.equal(hasWs(roots[10_000]), false, 'the first unactivated root is cleaned in the forward pass');
    assert.equal(__pendingTagWaiterCountForTests(), 0);
    assert.equal(__pendingUndefinedRootCountForTests(), 0);
    assert.equal(boundaryEvents().length, 0, 'a failed boundary emits no success event');
    assert.equal(dispatchedEvents.some((e) => e.type === 'webui:hydration-complete'), false);
    assert.equal(lc.streamingGateAborted, true);
    assert.equal(lc.terminalReached, false);
    assert.equal(lc.pendingBoundaries, 0, 'boundary accounting remains balanced');
    assert.equal(lc.pendingLateActivations, 0, 'partial late-activation accounting is balanced');
  });

  test('a marker-scan breach aborts and bounded forward cleanup reaches remaining roots', async () => {
    const children: FakeNode[] = [];
    for (let i = 0; i < 50_001; i++) children.push(comment('padding'));
    let activated = false;
    const late = element('my-after-scan-cap', {
      attrs: { 'data-ws': '' },
      hook() {
        activated = true;
      },
    });
    children.push(late);
    const wrapper = element('div', { children });
    const b = buildBoundary(0, 1, [wrapper], { state: {} });
    predefine('my-after-scan-cap');
    (window as unknown as { __WEBUI_STREAMING_DEBUG__: boolean }).__WEBUI_STREAMING_DEBUG__ = true;
    const previousError = console.error;
    console.error = () => {};
    try {
      enqueue(b.sentinel);
      await flush();
    } finally {
      console.error = previousError;
    }

    assert.equal(__isHaltedForTests(), true);
    assert.equal(activated, false, 'nodes after the scan cap are never activated');
    assert.equal(hasWs(late), false, 'bounded forward cleanup strips a remaining streamed marker');
    assertScaffoldCleaned(b);
    assert.equal(boundaryEvents().length, 0);
    assert.equal(dispatchedEvents.some((e) => e.type === 'webui:hydration-complete'), false);
    assert.equal(__getLifecycleStateForTests().pendingBoundaries, 0);
  });

  test('__resetStreamingCoordinatorForTests balances lifecycle without underflow', async () => {
    // Leave a live waiter, then reset: abandonment must settle the count to
    // exactly zero (never negative), and a stale later define must no-op.
    const activated: string[] = [];
    const late = element('my-reset', { hook() { activated.push('x'); } });
    const b = buildBoundary(0, 0, [late], { state: {} });
    enqueue(b.sentinel);
    await flush();
    assert.equal(__pendingTagWaiterCountForTests(), 1);
    assert.equal(__getLifecycleStateForTests().pendingLateActivations, 1);

    __resetStreamingCoordinatorForTests();
    // Reset settles the abandoned waiter; lifecycle count is cleared by the
    // companion lifecycle reset. Waiter set is empty.
    assert.equal(__pendingTagWaiterCountForTests(), 0, 'waiters cleared on reset');
    __resetLifecycleForTests();

    // A stale whenDefined resolution from before the reset must not mutate the
    // fresh generation's state.
    defineTag('my-reset');
    await flush();
    assert.deepEqual(activated, [], 'stale resolution does not activate after reset');
    const lc = __getLifecycleStateForTests();
    assert.equal(lc.pendingLateActivations, 0, 'no underflow from a stale resolution');
    assert.ok(lc.pendingLateActivations >= 0, 'late-activation count never negative');
  });

  test('an illegal record queued behind terminal aborts before hydration-complete', async () => {
    const terminal = buildMarkerless(0, 1, {});
    const post = buildRawBoundary(1, '[1,1,0,{}]', []);

    // Both records are present before the single pump runs. The terminal must
    // remain tentative until the queue validates the record behind it.
    enqueue(terminal.sentinel);
    enqueue(post.sentinel);
    await flush();

    const lc = __getLifecycleStateForTests();
    assert.equal(__isHaltedForTests(), true);
    assertScaffoldCleaned(post);
    assert.equal(lc.streamingGateAborted, true);
    assert.equal(lc.completed, false, 'terminal cannot complete before the queued tail is rejected');
    assert.equal(lc.pendingBoundaries, 0, 'the tentative terminal boundary settles on abort');
    assert.equal(dispatchedEvents.some((e) => e.type === 'webui:hydration-complete'), false);
  });

  test('a record after a committed terminal is rejected and never completes, even with an open waiter', async () => {
    // A terminal boundary commits successfully but still has an undefined-tag
    // root, so completion is gated on the late activation.
    const activated: string[] = [];
    const open = element('my-open', { hook() { activated.push('x'); } });
    const terminalB = buildBoundary(0, 1, [open], { state: {} });
    enqueue(terminalB.sentinel);
    await flush();

    let lc = __getLifecycleStateForTests();
    assert.equal(lc.terminalReached, true, 'terminal committed');
    assert.equal(lc.completed, false, 'completion gated on the open late activation');
    assert.equal(__pendingTagWaiterCountForTests(), 1);

    // A malformed record arrives after the terminal: it must be rejected (its
    // scaffold cleaned + stream halted/aborted), and settling the abandoned
    // waiter during failure must NOT dispatch hydration-complete.
    const post = buildRawBoundary(1, '[bad', []);
    enqueue(post.sentinel);
    await flush();

    assertScaffoldCleaned(post);
    lc = __getLifecycleStateForTests();
    assert.equal(__isHaltedForTests(), true, 'post-terminal record halts');
    assert.equal(lc.streamingGateAborted, true, 'the gate is aborted on failure');
    assert.equal(__pendingTagWaiterCountForTests(), 0, 'the open waiter is abandoned');
    assert.equal(hasPending(open), false, 'stashed state cleared on abandonment');
    assert.equal(lc.pendingLateActivations, 0, 'late-activation settled exactly once');
    assert.equal(lc.completed, false, 'a failed stream never completes');
    assert.equal(dispatchedEvents.some((e) => e.type === 'webui:hydration-complete'), false);

    // A later class definition must neither activate the abandoned root nor
    // resurrect completion.
    defineTag('my-open');
    await flush();
    assert.deepEqual(activated, [], 'abandoned root is never activated');
    assert.equal(__getLifecycleStateForTests().completed, false, 'still no completion after late define');
    assert.equal(dispatchedEvents.some((e) => e.type === 'webui:hydration-complete'), false);
  });

  test('a truncated stream (DOMContentLoaded before terminal) clears pending state and never completes', async () => {
    __installTruncationGuardForTests();

    // A committed non-terminal boundary with an undefined tag: the waiter
    // holds the gate open.
    const activated: string[] = [];
    const late = element('my-trunc', { hook() { activated.push('x'); } });
    const b = buildBoundary(0, 0, [late], { state: {} });
    enqueue(b.sentinel);
    await flush();
    assert.equal(__pendingTagWaiterCountForTests(), 1);
    assert.equal(__getLifecycleStateForTests().completed, false);

    // The document finishes parsing without a terminal boundary: truncated.
    fireDomContentLoaded();
    await flush();

    const lc = __getLifecycleStateForTests();
    assert.equal(__isHaltedForTests(), true, 'truncation halts the stream');
    assert.equal(lc.streamingGateAborted, true, 'gate aborted on truncation');
    assert.equal(__pendingTagWaiterCountForTests(), 0, 'waiter abandoned on truncation');
    assert.equal(hasPending(late), false, 'stashed state cleared on truncation');
    assert.equal(lc.pendingLateActivations, 0, 'late-activation settled exactly once');
    assert.equal(lc.completed, false, 'a truncated stream never completes');
    assert.equal(dispatchedEvents.some((e) => e.type === 'webui:hydration-complete'), false);

    // A class defined after truncation must not activate or complete.
    defineTag('my-trunc');
    await flush();
    assert.deepEqual(activated, [], 'root abandoned by truncation is never activated');
    assert.equal(dispatchedEvents.some((e) => e.type === 'webui:hydration-complete'), false);
  });

  test('a truncation guard installed after DOMContentLoaded halts and cleans immediately', async () => {
    const late = element('my-late-truncation', { attrs: { 'data-ws': '' }, hook() {} });
    const b = buildBoundary(0, 0, [late], { state: { pending: true } });
    enqueue(b.sentinel);
    await flush();
    assert.equal(hasPending(late), true);

    setDocumentReadyState('complete');
    __installTruncationGuardForTests();
    await flush();

    const lc = __getLifecycleStateForTests();
    assert.equal(__isHaltedForTests(), true);
    assert.equal(hasPending(late), false);
    assert.equal(hasWs(late), false);
    assert.equal(__pendingUndefinedRootCountForTests(), 0);
    assert.equal(lc.pendingLateActivations, 0);
    assert.equal(lc.streamingGateAborted, true);
    assert.equal(dispatchedEvents.some((e) => e.type === 'webui:hydration-complete'), false);
  });

  test('a complete late install lets an already-queued valid terminal commit first', async () => {
    const terminal = buildMarkerless(0, 1, {});
    enqueue(terminal.sentinel);
    setDocumentReadyState('complete');
    __installTruncationGuardForTests();
    await flush();

    const lc = __getLifecycleStateForTests();
    assert.equal(__isHaltedForTests(), false, 'the late guard must not abort a valid terminal');
    assert.equal(lc.streamingGateAborted, false);
    assert.equal(lc.terminalReached, true);
    assert.equal(lc.completed, true);
    assert.equal(dispatchedEvents.some((e) => e.type === 'webui:hydration-complete'), true);
  });

  test('a stale DOMContentLoaded guard no-ops after a coordinator reset', async () => {
    __installTruncationGuardForTests();
    // Reset bumps the coordinator generation; the already-registered guard
    // captured the prior generation, so firing it must be inert.
    __resetStreamingCoordinatorForTests();
    __resetLifecycleForTests();
    beginStreamingGate();

    fireDomContentLoaded();
    await flush();

    assert.equal(__isHaltedForTests(), false, 'a stale truncation guard must not halt fresh state');
    assert.equal(__getLifecycleStateForTests().streamingGateAborted, false, 'stale guard does not abort');
  });

  // ── Direct boundary state (no global swap, no per-root wrapper) ─────

  test('boundary state is handed to the activation hook, never published globally', async () => {
    let received: Record<string, unknown> | undefined | 'unset' = 'unset';
    const root = element('my-direct', { hook(state) { received = state; } });
    const b = buildBoundary(0, 1, [root], { state: { a: 1 } });

    predefine('my-direct');
    enqueue(b.sentinel);
    await flush();

    assert.deepEqual(received, { a: 1 }, 'hook receives its own boundary state directly');
    // applyBootstrapGlobals skips `state`, so a committed boundary retains no
    // state on the global handoff.
    assert.equal(webuiGlobal().state, undefined, 'boundary state is never published on window.__webui');
  });

  test('two boundaries activate with their own distinct state', async () => {
    const received: Array<Record<string, unknown> | undefined> = [];
    const r0 = element('my-two', { hook(state) { received.push(state); } });
    const b0 = buildBoundary(0, 0, [r0], { state: { which: 'first' } });
    predefine('my-two');
    enqueue(b0.sentinel);
    await flush();

    const r1 = element('my-two', { hook(state) { received.push(state); } });
    const b1 = buildBoundary(1, 1, [r1], { state: { which: 'second' } });
    enqueue(b1.sentinel);
    await flush();

    assert.deepEqual(received, [{ which: 'first' }, { which: 'second' }], 'each root sees its own boundary state');
    assert.equal(webuiGlobal().state, undefined, 'neither boundary leaks state globally');
  });

  test('a delayed (undefined-tag) root activates with its own stashed state, even after a later boundary', async () => {
    let received: Record<string, unknown> | undefined | 'unset' = 'unset';
    const late = element('my-delayed', { hook(state) { received = state; } });
    const b0 = buildBoundary(0, 0, [late], { state: { seq: 0 } });
    enqueue(b0.sentinel);
    await flush();
    assert.equal(hasPending(late), true, 'undefined-tag root stashed its own state');

    // A later boundary commits with different state; the global handoff still
    // never carries state, and the delayed root must not pick up this one.
    const other = element('my-other', { hook() {} });
    const b1 = buildBoundary(1, 1, [other], { state: { seq: 1 } });
    predefine('my-other');
    enqueue(b1.sentinel);
    await flush();

    defineTag('my-delayed');
    await flush();
    assert.deepEqual(received, { seq: 0 }, 'the delayed root activates with the state its own boundary carried');
  });

  test('a boundary carrying no state stashes presence and activates with undefined', async () => {
    let received: Record<string, unknown> | undefined | 'unset' = 'unset';
    const late = element('my-nostate', { hook(state) { received = state; } });
    // Bootstrap with no `state` key at all.
    const b = buildBoundary(0, 1, [late], {});
    enqueue(b.sentinel);
    await flush();

    // Presence is recorded even though the state is undefined (no wrapper).
    assert.equal(hasPending(late), true, 'a stateless boundary still records the stash by property presence');

    defineTag('my-nostate');
    await flush();
    assert.equal(received, undefined, 'a stateless boundary activates its root with undefined state');
    assert.equal(hasPending(late), false, 'stash consumed on activation');
  });

  // ── data-ws streamed-host marker cleanup on reject paths ───────────

  test('a rejected (malformed) boundary strips data-ws from its roots, keeping them', async () => {
    const root0 = element('my-ws', { attrs: { 'data-ws': '' }, hook() {} });
    assert.equal(hasWs(root0), true, 'root starts marked as a streamed host');
    const b = buildRawBoundary(0, '[1,0,0,{', [root0]);

    enqueue(b.sentinel);
    await flush();
    assert.equal(__isHaltedForTests(), true);
    assertScaffoldCleaned(b);
    assert.equal(hasWs(root0), false, 'data-ws stripped from a rejected boundary root');
    assert.equal(root0.parentNode, b.root, 'the root itself is preserved');
  });

  test('an out-of-order boundary strips data-ws from its roots', async () => {
    const root0 = element('my-ws-ooo', { attrs: { 'data-ws': '' }, hook() {} });
    // Expecting sequence 0, but this boundary claims 1: rejected + cleaned.
    const b = buildBoundary(1, 0, [root0], { state: {} });
    enqueue(b.sentinel);
    await flush();

    assert.equal(__isHaltedForTests(), true);
    assertScaffoldCleaned(b);
    assert.equal(hasWs(root0), false, 'data-ws stripped from an out-of-order boundary root');
  });

  test('fail() strips data-ws from abandoned undefined-tag roots', async () => {
    const late = element('my-ws-abandon', { attrs: { 'data-ws': '' }, hook() {} });
    const b0 = buildBoundary(0, 0, [late], { state: {} });
    enqueue(b0.sentinel);
    await flush();
    assert.equal(hasPending(late), true);
    assert.equal(hasWs(late), true, 'a deferred undefined-tag root keeps data-ws until activation');

    // Halt via a malformed follow-up boundary.
    const bad = buildRawBoundary(1, '[1,1,0,{', []);
    enqueue(bad.sentinel);
    await flush();

    assert.equal(__isHaltedForTests(), true);
    assert.equal(hasPending(late), false, 'stashed state cleared on abandonment');
    assert.equal(hasWs(late), false, 'data-ws stripped from an abandoned pending root');
  });

  test('truncation strips data-ws from abandoned undefined-tag roots', async () => {
    __installTruncationGuardForTests();
    const late = element('my-ws-trunc', { attrs: { 'data-ws': '' }, hook() {} });
    const b = buildBoundary(0, 0, [late], { state: {} });
    enqueue(b.sentinel);
    await flush();
    assert.equal(hasWs(late), true);

    fireDomContentLoaded();
    await flush();

    assert.equal(__isHaltedForTests(), true);
    assert.equal(hasPending(late), false, 'stashed state cleared on truncation');
    assert.equal(hasWs(late), false, 'data-ws stripped from a truncation-abandoned root');
  });

  // ── Centralized successful-path data-ws removal (invokeActivationHook) ──

  test('a committed defined root with NO activation hook still loses its data-ws marker', async () => {
    // A plain defined element with no boundary-activate hook: the coordinator
    // has nothing to call, but must still strip the compiler scaffolding.
    const root = element('my-nohook', { attrs: { 'data-ws': '' } });
    const b = buildBoundary(0, 1, [root], { state: {} });

    predefine('my-nohook');
    enqueue(b.sentinel);
    await flush();

    assert.equal(__isHaltedForTests(), false, 'a no-hook root is a clean commit, not a failure');
    assert.equal(hasWs(root), false, 'data-ws stripped from a committed no-hook root');
    assert.equal(root.parentNode, b.root, 'the root itself is retained');
    assert.equal(hasPending(root), false);
  });

  test('a committed root whose activation hook THROWS still loses its data-ws marker and is isolated', async () => {
    const survivor: string[] = [];
    const thrower = element('my-throw', {
      attrs: { 'data-ws': '' },
      hook() {
        throw new Error('activation boom');
      },
    });
    const ok = element('my-after', { attrs: { 'data-ws': '' }, hook() { survivor.push('my-after'); } });
    const b = buildBoundary(0, 1, [thrower, ok], { state: {} });

    predefine('my-throw', 'my-after');
    const previousError = console.error;
    console.error = () => {};
    try {
      enqueue(b.sentinel);
      await flush();
    } finally {
      console.error = previousError;
    }

    assert.equal(__isHaltedForTests(), false, 'one hook throwing does not halt the well-formed stream');
    assert.equal(hasWs(thrower), false, 'data-ws stripped even when the hook threw');
    assert.equal(thrower.parentNode, b.root, 'the throwing root is retained');
    assert.deepEqual(survivor, ['my-after'], 'a later root in the same boundary still activates');
    assert.equal(hasWs(ok), false, 'the surviving root also loses its marker');
  });

  test('a committed root whose hook OPTS OUT (static-host style) still loses its data-ws marker', async () => {
    // A compiler-owned static host defines the hook but returns without
    // activating ($shouldActivateOnBoundaryCommit() === false). The coordinator
    // must still strip its marker so the committed host is not left deferred.
    let called = false;
    const host = element('my-optout', {
      attrs: { 'data-ws': '' },
      hook() {
        called = true; // opts out: records the call but performs no activation
      },
    });
    const b = buildBoundary(0, 1, [host], { state: {} });

    predefine('my-optout');
    enqueue(b.sentinel);
    await flush();

    assert.equal(called, true, 'the opt-out hook was still invoked');
    assert.equal(__isHaltedForTests(), false);
    assert.equal(hasWs(host), false, 'data-ws stripped from a committed opt-out root');
    assert.equal(host.parentNode, b.root, 'the opt-out root is retained');
  });
});

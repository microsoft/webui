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
const ABANDON = Symbol.for('microsoft.webui.boundaryAbandon');
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
  setState?: (state: Record<string, unknown>) => void;
  [ACTIVATE]?: (state?: Record<string, unknown>) => number;
  [ABANDON]?: () => void;
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
  activationOutcome?: number;
  abandon?: () => void;
  children?: Array<FakeNode>;
  shadowChildren?: Array<FakeNode>;
  setState?: (state: Record<string, unknown>) => void;
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
  if (spec.hook) {
    node[ACTIVATE] = (state) => {
      spec.hook!(state);
      return spec.activationOutcome ?? 1;
    };
  }
  if (spec.abandon) node[ABANDON] = spec.abandon;
  if (spec.setState) node.setState = spec.setState;
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
let documentWalkCalls: number;

function installGlobals(): void {
  definedTags = new Map();
  whenDefinedResolvers = new Map();
  whenDefinedCalls = new Map();
  upgradeCalls = new Map();
  dispatchedEvents = [];
  documentListeners = new Map();
  documentWalkCalls = 0;

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
      const ctor = definedTags.get(tag) as { prototype?: { [ACTIVATE]?: (state?: Record<string, unknown>) => number } } | undefined;
      const hook = ctor?.prototype?.[ACTIVATE];
      if (hook) root[ACTIVATE] = hook;
    },
  };

  const documentFake = {
    readyState: 'loading',
    getElementsByTagName(tag: string): FakeElement[] {
      documentWalkCalls++;
      const upper = tag.toUpperCase();
      return elementRegistry.filter((el) =>
        (upper === '*' || el.tagName === upper) && el.parentNode !== null
      );
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
  outcome = 1,
): void {
  class DefinedElement {}
  if (supportsDetachedResume) {
    (DefinedElement.prototype as unknown as { [ACTIVATE]?: (state?: Record<string, unknown>) => number })[ACTIVATE] =
      (state) => {
        hook(state);
        return outcome;
      };
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

/** Drain macrotask turns, which a time-sliced drain yields across. */
async function settle(turns = 12): Promise<void> {
  for (let i = 0; i < turns; i++) {
    await new Promise<void>((r) => setTimeout(r, 0));
  }
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
  __streamingRetentionStateForTests,
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

/** Patches queued for late activation accumulate in a null-prototype object
 *  (network-supplied keys must never reach `Object.prototype`), so copy them
 *  into plain objects before strict deep-equality assertions. Keys are defined
 *  rather than assigned so a `__proto__` key stays an own data property instead
 *  of silently retargeting the copy's prototype and hiding a pollution bug. */
function plainPatches(
  patches: Array<Record<string, unknown>>,
): Array<Record<string, unknown>> {
  return patches.map((patch) => {
    const plain: Record<string, unknown> = {};
    for (const key of Object.keys(patch)) {
      Object.defineProperty(plain, key, {
        value: patch[key],
        writable: true,
        enumerable: true,
        configurable: true,
      });
    }
    return plain;
  });
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
    text: JSON.stringify([
      1,
      sequence,
      terminal === 1 ? 3 : 0,
      terminal === 1 ? 0 : sequence,
      bootstrap,
    ]),
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
    text: JSON.stringify([1, sequence, terminal === 1 ? 3 : 0, 0, bootstrap]),
  });
  const sentinel = element('webui-hydrate');
  const root = body();
  link(root, [scriptEl, sentinel]);
  return { root, sentinel, scriptEl, startMarker: null, endMarker: null, roots: [] };
}

function buildUpdatableBoundary(
  recordSequence: number,
  boundaryId: number,
  roots: FakeElement[],
  bootstrap: object,
): BuiltBoundary {
  markStreamedRoots(roots);
  const start = comment(`wb:${boundaryId}`);
  const end = comment(`/wb:${boundaryId}`);
  const scriptEl = element('script', {
    attrs: { 'data-webui-boundary': '' },
    text: JSON.stringify([1, recordSequence, 1, boundaryId, bootstrap]),
  });
  const sentinel = element('webui-hydrate');
  const root = body();
  link(root, [start, ...roots, end, scriptEl, sentinel]);
  return { root, sentinel, scriptEl, startMarker: start, endMarker: end, roots };
}

function buildStateUpdate(
  recordSequence: number,
  boundaryId: number,
  patch: Record<string, unknown>,
): BuiltBoundary {
  const scriptEl = element('script', {
    attrs: { 'data-webui-boundary': '' },
    text: JSON.stringify([1, recordSequence, 2, boundaryId, patch]),
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
    assert.equal(documentWalkCalls, 0, 'valid streams never scan the document');
  });

  test('applies state updates without reactivating an updatable boundary', async () => {
    const activations: Array<Record<string, unknown> | undefined> = [];
    const updates: Array<Record<string, unknown>> = [];
    const weather = element('weather-panel', {
      hook(state) {
        activations.push(state);
      },
      setState(state) {
        updates.push(state);
      },
    });
    predefine('weather-panel');

    const checkpoint = buildUpdatableBoundary(
      0,
      0,
      [weather],
      { state: { status: 'loading' } },
    );
    enqueue(checkpoint.sentinel);
    await flush();
    assert.deepEqual(__streamingRetentionStateForTests(), [1, 1]);
    const update = buildStateUpdate(1, 0, {
      status: 'ready',
      forecast: 'Sunny',
    });
    enqueue(update.sentinel);
    await flush();
    const terminal = buildMarkerless(2, 1, {});
    enqueue(terminal.sentinel);
    await flush();

    assert.deepEqual(activations, [{ status: 'loading' }]);
    assert.deepEqual(updates, [{ status: 'ready', forecast: 'Sunny' }]);
    assertScaffoldCleaned(checkpoint);
    assert.equal(update.sentinel.parentNode, null);
    assert.equal(update.scriptEl.parentNode, null);
    assert.equal(__isHaltedForTests(), false);
    assert.equal(__getLifecycleStateForTests().completed, true);
    assert.deepEqual(
      __streamingRetentionStateForTests(),
      [0, 0],
      'terminal releases retained boundary roots',
    );
  });

  test('a throwing setState degrades one root without halting the stream', async () => {
    const previousError = console.error;
    const errors: string[] = [];
    console.error = (message?: unknown) => {
      errors.push(String(message));
    };
    try {
      const updates: Array<Record<string, unknown>> = [];
      const activations: Array<Record<string, unknown> | undefined> = [];
      // An application component whose own change handler throws.
      const weather = element('weather-panel', {
        hook() {},
        setState() {
          throw new Error('boom');
        },
      });
      // A healthy sibling inside the same boundary.
      const stats = element('stat-panel', {
        hook() {},
        setState(state) {
          updates.push(state);
        },
      });
      const late = element('late-panel', {
        hook(state) {
          activations.push(state);
        },
      });
      predefine('weather-panel', 'stat-panel', 'late-panel');

      const checkpoint = buildUpdatableBoundary(
        0,
        0,
        [weather, stats],
        { state: { status: 'loading' } },
      );
      enqueue(checkpoint.sentinel);
      await flush();

      enqueue(buildStateUpdate(1, 0, { status: 'ready' }).sentinel);
      await flush();

      // A later boundary must still hydrate.
      const second = buildUpdatableBoundary(2, 1, [late], { state: { n: 1 } });
      enqueue(second.sentinel);
      await flush();
      assert.equal(
        __isHaltedForTests(),
        false,
        `stream halted after the failing update; errors=${errors.join('|')}`,
      );

      const terminal = buildMarkerless(3, 1, {});
      enqueue(terminal.sentinel);
      await flush();

      assert.equal(
        errors.some((message) =>
          message.includes('state update failed for <weather-panel>')
        ),
        true,
        'the failing root is reported',
      );
      assert.deepEqual(
        updates,
        [{ status: 'ready' }],
        'a healthy sibling in the same boundary still receives the patch',
      );
      assert.deepEqual(
        activations,
        [{ n: 1 }],
        'a later boundary still hydrates',
      );
      assert.equal(__isHaltedForTests(), false);
      assert.equal(__getLifecycleStateForTests().completed, true);
      assert.deepEqual(
        __streamingRetentionStateForTests(),
        [0, 0],
        'terminal still releases retained boundary roots',
      );
    } finally {
      console.error = previousError;
    }
  });

  test('replays updates that arrive before an island definition after activation', async () => {
    const activations: Array<Record<string, unknown> | undefined> = [];
    const updates: Array<Record<string, unknown>> = [];
    const weather = element('weather-panel', {
      hook(state) {
        activations.push(state);
      },
      setState(state) {
        updates.push(state);
      },
    });

    const checkpoint = buildUpdatableBoundary(
      0,
      0,
      [weather],
      { state: { status: 'loading', location: 'Seattle' } },
    );
    enqueue(checkpoint.sentinel);
    await flush();
    enqueue(buildStateUpdate(1, 0, { status: 'ready' }).sentinel);
    await flush();
    enqueue(buildStateUpdate(2, 0, { forecast: 'Rain' }).sentinel);
    await flush();

    assert.deepEqual(activations, []);
    assert.deepEqual(updates, []);
    assert.equal(hasPending(weather), true);

    defineTag('weather-panel');
    await flush();
    assert.deepEqual(activations, [{
      status: 'loading',
      location: 'Seattle',
    }]);
    assert.deepEqual(
      plainPatches(updates),
      [{
        status: 'ready',
        forecast: 'Rain',
      }],
    );

    enqueue(buildMarkerless(3, 1, {}).sentinel);
    await flush();
    assert.equal(__getLifecycleStateForTests().completed, true);
  });

  test('a root whose activation fails never receives later updates', async () => {
    const previousError = console.error;
    const errors: string[] = [];
    console.error = (message?: unknown) => {
      errors.push(String(message));
    };
    try {
      const brokenUpdates: Array<Record<string, unknown>> = [];
      const healthyUpdates: Array<Record<string, unknown>> = [];
      const broken = element('broken-panel', {
        hook() {
          throw new Error('activation exploded');
        },
        setState(state) {
          brokenUpdates.push(state);
        },
      });
      const healthy = element('healthy-panel', {
        hook() {},
        setState(state) {
          healthyUpdates.push(state);
        },
      });

      enqueue(buildUpdatableBoundary(
        0,
        0,
        [broken, healthy],
        { state: { status: 'loading' } },
      ).sentinel);
      await flush();

      defineTag('broken-panel');
      defineTag('healthy-panel');
      await flush();

      assert.equal(
        errors.some((message) =>
          message.includes('late activation failed for <broken-panel>')
        ),
        true,
        'the failing activation is reported',
      );

      enqueue(buildStateUpdate(1, 0, { status: 'ready' }).sentinel);
      await flush();

      // `data-ws` is stripped on the throwing path too, so inferring liveness
      // from its absence would deliver this update to an inert root forever.
      assert.deepEqual(
        brokenUpdates,
        [],
        'a root that never activated is not an update target',
      );
      assert.deepEqual(
        plainPatches(healthyUpdates),
        [{ status: 'ready' }],
        'its healthy sibling still receives the patch',
      );
      assert.equal(__isHaltedForTests(), false);

      enqueue(buildMarkerless(2, 1, {}).sentinel);
      await flush();
      assert.equal(__getLifecycleStateForTests().completed, true);
      assert.deepEqual(__streamingRetentionStateForTests(), [0, 0]);
    } finally {
      console.error = previousError;
    }
  });

  test('one boundary mixing live and deferred roots updates both exactly once', async () => {
    const liveUpdates: Array<Record<string, unknown>> = [];
    const lateUpdates: Array<Record<string, unknown>> = [];
    const live = element('live-panel', {
      hook() {},
      setState(state) {
        liveUpdates.push(state);
      },
    });
    const late = element('late-panel', {
      hook() {},
      setState(state) {
        lateUpdates.push(state);
      },
    });
    predefine('live-panel');

    enqueue(buildUpdatableBoundary(
      0,
      0,
      [live, late],
      { state: { status: 'loading' } },
    ).sentinel);
    await flush();

    enqueue(buildStateUpdate(1, 0, { status: 'ready' }).sentinel);
    await flush();
    assert.deepEqual(
      plainPatches(liveUpdates),
      [{ status: 'ready' }],
      'the already-defined root updates immediately',
    );
    assert.deepEqual(lateUpdates, [], 'the deferred root waits');

    defineTag('late-panel');
    await flush();
    assert.deepEqual(
      plainPatches(lateUpdates),
      [{ status: 'ready' }],
      'the deferred root replays the queued patch exactly once on activation',
    );
    assert.deepEqual(
      plainPatches(liveUpdates),
      [{ status: 'ready' }],
      'activating a sibling does not re-deliver the patch to a live root',
    );

    enqueue(buildStateUpdate(2, 0, { forecast: 'Sunny' }).sentinel);
    await flush();
    assert.deepEqual(
      plainPatches(liveUpdates),
      [{ status: 'ready' }, { forecast: 'Sunny' }],
    );
    assert.deepEqual(
      plainPatches(lateUpdates),
      [{ status: 'ready' }, { forecast: 'Sunny' }],
      'both roots converge on the same final state',
    );

    enqueue(buildMarkerless(3, 1, {}).sentinel);
    await flush();
    assert.deepEqual(__streamingRetentionStateForTests(), [0, 0]);
  });

  test('updates nested roots retained behind an undefined outer island', async () => {
    const outerActivations: Array<Record<string, unknown> | undefined> = [];
    const innerActivations: Array<Record<string, unknown> | undefined> = [];
    const outerUpdates: Array<Record<string, unknown>> = [];
    const innerUpdates: Array<Record<string, unknown>> = [];
    const inner = element('inner-panel', {
      hook(state) {
        innerActivations.push(state);
      },
      setState(state) {
        innerUpdates.push(state);
      },
    });
    const outer = element('outer-panel', {
      children: [inner],
      hook(state) {
        outerActivations.push(state);
      },
      setState(state) {
        outerUpdates.push(state);
      },
    });
    predefine('inner-panel');

    enqueue(buildUpdatableBoundary(
      0,
      0,
      [outer],
      { state: { status: 'loading' } },
    ).sentinel);
    await flush();
    assert.deepEqual(
      __streamingRetentionStateForTests(),
      [1, 2],
      'the outer barrier does not hide nested roots from bounded retention',
    );

    enqueue(buildStateUpdate(1, 0, { status: 'ready' }).sentinel);
    await flush();
    defineTag('outer-panel');
    await flush();

    assert.deepEqual(outerActivations, [{ status: 'loading' }]);
    assert.deepEqual(innerActivations, [{ status: 'loading' }]);
    assert.deepEqual(
      __streamingRetentionStateForTests(),
      [1, 2],
      'definition preserves the complete retained update set',
    );

    enqueue(buildStateUpdate(2, 0, { forecast: 'Sunny' }).sentinel);
    await flush();
    assert.deepEqual(
      plainPatches(outerUpdates),
      [{ status: 'ready' }, { forecast: 'Sunny' }],
    );
    assert.deepEqual(
      plainPatches(innerUpdates),
      [{ status: 'ready' }, { forecast: 'Sunny' }],
    );

    enqueue(buildMarkerless(3, 1, {}).sentinel);
    await flush();
    assert.deepEqual(__streamingRetentionStateForTests(), [0, 0]);
  });

  test('a queued patch that throws degrades one root without stranding its retained descendants', async () => {
    const previousError = console.error;
    const errors: string[] = [];
    console.error = (message?: unknown) => {
      errors.push(String(message));
    };
    try {
      const innerActivations: Array<Record<string, unknown> | undefined> = [];
      const innerUpdates: Array<Record<string, unknown>> = [];
      const inner = element('inner-panel', {
        hook(state) {
          innerActivations.push(state);
        },
        setState(state) {
          innerUpdates.push(state);
        },
      });
      // The late-defining outer island's own change handler throws while
      // replaying the patch queued during its deferral.
      const outer = element('outer-panel', {
        children: [inner],
        hook() {},
        setState() {
          throw new Error('boom');
        },
      });
      predefine('inner-panel');

      enqueue(buildUpdatableBoundary(
        0,
        0,
        [outer],
        { state: { status: 'loading' } },
      ).sentinel);
      await flush();
      enqueue(buildStateUpdate(1, 0, { status: 'ready' }).sentinel);
      await flush();

      defineTag('outer-panel');
      await flush();

      assert.equal(
        errors.some((message) =>
          message.includes('state update failed for <outer-panel>')
        ),
        true,
        'the throwing root is reported as a state-update failure',
      );
      assert.deepEqual(
        innerActivations,
        [{ status: 'loading' }],
        'the retained descendant still activates behind the failed patch',
      );
      assert.deepEqual(
        plainPatches(innerUpdates),
        [{ status: 'ready' }],
        'the retained descendant still receives the queued patch',
      );
      assert.equal(hasWs(inner), false);
      assert.equal(__isHaltedForTests(), false);

      enqueue(buildMarkerless(2, 1, {}).sentinel);
      await flush();
      assert.equal(__getLifecycleStateForTests().completed, true);
    } finally {
      console.error = previousError;
    }
  });

  test('a dormant compiler-owned host still receives its queued patch', async () => {
    const updates: Array<Record<string, unknown>> = [];
    // A static host opts out of boundary-commit activation but must stay a
    // valid update target: an explicit state write is exactly what wakes it,
    // and the immediate commit path already writes to it.
    const staticHost = element('static-panel', {
      hook() {},
      activationOutcome: 2,
      setState(state) {
        updates.push(state);
      },
    });

    enqueue(buildUpdatableBoundary(
      0,
      0,
      [staticHost],
      { state: { status: 'loading' } },
    ).sentinel);
    await flush();
    enqueue(buildStateUpdate(1, 0, { status: 'ready' }).sentinel);
    await flush();
    assert.deepEqual(updates, [], 'the patch waits while the tag is undefined');

    defineTag('static-panel');
    await flush();
    assert.deepEqual(plainPatches(updates), [{ status: 'ready' }]);

    enqueue(buildMarkerless(2, 1, {}).sentinel);
    await flush();
    assert.equal(__getLifecycleStateForTests().completed, true);
  });

  test('rejects state updates for final boundaries', async () => {
    const previousError = console.error;
    console.error = () => {};
    try {
      const root = element('my-final', { hook() {}, setState() {} });
      predefine('my-final');
      enqueue(buildBoundary(0, 0, [root], { state: {} }).sentinel);
      await flush();
      const update = buildStateUpdate(1, 0, { value: 1 });
      enqueue(update.sentinel);
      await flush();

      assert.equal(__isHaltedForTests(), true);
      assert.equal(update.sentinel.parentNode, null);
      assert.equal(update.scriptEl.parentNode, null);
    } finally {
      console.error = previousError;
    }
  });

  test('halts when updatable-boundary retention exceeds its fixed limit', async () => {
    const previousError = console.error;
    console.error = () => {};
    try {
      for (let boundary = 0; boundary <= 128; boundary++) {
        enqueue(buildUpdatableBoundary(boundary, boundary, [], {}).sentinel);
      }
      await flush();

      assert.equal(__isHaltedForTests(), true);
      assert.deepEqual(
        __streamingRetentionStateForTests(),
        [0, 0],
        'failure releases every retained response reference',
      );
    } finally {
      console.error = previousError;
    }
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

    const empty = buildMarkerless(3, 1, {});
    enqueue(empty.sentinel);
    await flush();
    assert.equal(webuiGlobal().inventory, '03', 'an empty delta cannot erase prior bits');
  });

  test('ORs inventory bytes by position when checkpoint lengths differ', async () => {
    const first = buildBoundary(0, 0, [], { inventory: '01' });
    const second = buildBoundary(1, 0, [], { inventory: '0002' });
    enqueue(first.sentinel);
    await flush();
    enqueue(second.sentinel);
    await flush();

    assert.equal(webuiGlobal().inventory, '0102', 'short inventory strings are padded on the right');
  });

  test('accepts uppercase and lowercase ASCII inventory hex', async () => {
    const boundary = buildBoundary(0, 0, [], { inventory: '0123456789abcdefABCDEF' });
    enqueue(boundary.sentinel);
    await flush();

    assert.equal(__isHaltedForTests(), false);
    assert.equal(webuiGlobal().inventory, '0123456789abcdefabcdef');
  });

  test('halts the stream when a checkpoint payload is not an object', async () => {
    // The envelope parser no longer re-validates fields written by our own
    // serializer, so a structurally impossible payload is caught by the commit
    // path's error boundary instead. The stream must still fail closed.
    const previousError = console.error;
    console.error = () => {};
    try {
      const boundary = buildBoundary(0, 0, [], null as unknown as object);
      enqueue(boundary.sentinel);
      await flush();

      assert.equal(__isHaltedForTests(), true);
      assert.equal(__getLifecycleStateForTests().completed, false);
    } finally {
      console.error = previousError;
    }
  });

  test('boundary-hydrated event is opt-in via the debug flag', async () => {
    const b1 = buildMarkerless(0, 1, {});
    enqueue(b1.sentinel);
    await flush();
    assert.equal(boundaryEvents().length, 0, 'no event without the debug flag');

    // With the flag on, exactly one CustomEvent carrying {sequence, terminal}.
    __resetStreamingCoordinatorForTests();
    (globalThis as unknown as { window: { __WEBUI_STREAMING_DEBUG__: boolean } }).window.__WEBUI_STREAMING_DEBUG__ = true;
    const b2 = buildMarkerless(0, 1, {});
    enqueue(b2.sentinel);
    await flush();

    const evts = boundaryEvents();
    assert.equal(evts.length, 1);
    assert.deepEqual(evts[0].detail, {
      sequence: 0,
      terminal: true,
      kind: 'terminal',
    });
  });

  test('a slice budget yields between boundaries instead of hydrating in one task', async () => {
    (globalThis as unknown as { window: { __WEBUI_STREAMING_DEBUG__: boolean } })
      .window.__WEBUI_STREAMING_DEBUG__ = true;
    // Sub-millisecond, so the deadline is passed after every boundary and the
    // drain yields at each one. `performance.now()` is what makes this
    // expressible; `Date.now()`'s 1 ms resolution could never trip it.
    (globalThis as unknown as { window: { __WEBUI_STREAMING_SLICE_MS__: number } })
      .window.__WEBUI_STREAMING_SLICE_MS__ = 0.0001;

    const roots = ['a', 'b', 'c'].map((n) => element(`feed-${n}`, { hook() {} }));
    for (const n of ['a', 'b', 'c']) predefine(`feed-${n}`);

    // All four arrive together, as they would behind a coalescing proxy.
    for (let i = 0; i < 3; i++) {
      enqueue(buildBoundary(i, 0, [roots[i]], { state: {} }).sentinel);
    }
    enqueue(buildMarkerless(3, 1, {}).sentinel);

    await flush();
    assert.ok(
      boundaryEvents().length < 4,
      'the drain released the thread rather than committing all four in one task',
    );
    assert.equal(
      dispatchedEvents.some((e) => e.type === 'webui:hydration-complete'),
      false,
      'the terminal does not settle while the queue is still draining',
    );

    await settle();

    assert.deepEqual(
      boundaryEvents().map((e) => (e.detail as { sequence: number }).sequence),
      [0, 1, 2, 3],
      'slicing preserves record order',
    );
    assert.equal(
      dispatchedEvents.filter((e) => e.type === 'webui:hydration-complete').length,
      1,
      'completion still fires exactly once, after the last boundary',
    );
  });

  test('a coordinator reset abandons an in-flight sliced drain', async () => {
    (globalThis as unknown as { window: { __WEBUI_STREAMING_SLICE_MS__: number } })
      .window.__WEBUI_STREAMING_SLICE_MS__ = 0.0001;

    const roots = ['x', 'y', 'z'].map((n) => element(`feed-${n}`, { hook() {} }));
    for (const n of ['x', 'y', 'z']) predefine(`feed-${n}`);
    for (let i = 0; i < 3; i++) {
      enqueue(buildBoundary(i, 0, [roots[i]], { state: {} }).sentinel);
    }
    enqueue(buildMarkerless(3, 1, {}).sentinel);

    await flush();
    __resetStreamingCoordinatorForTests();
    await settle();

    assert.equal(
      dispatchedEvents.some((e) => e.type === 'webui:hydration-complete'),
      false,
      'the abandoned drain does not complete the superseded document',
    );
  });
  test('a state update commit is observable, not just checkpoints', async () => {
    (globalThis as unknown as { window: { __WEBUI_STREAMING_DEBUG__: boolean } })
      .window.__WEBUI_STREAMING_DEBUG__ = true;
    const panel = element('weather-panel', { hook() {}, setState() {} });
    predefine('weather-panel');

    const checkpoint = buildUpdatableBoundary(0, 0, [panel], {
      state: { status: 'loading' },
    });
    enqueue(checkpoint.sentinel);
    await flush();
    enqueue(buildStateUpdate(1, 0, { status: 'ready' }).sentinel);
    await flush();
    enqueue(buildMarkerless(2, 1, {}).sentinel);
    await flush();

    assert.deepEqual(
      boundaryEvents().map((e) => (e.detail as { kind: string }).kind),
      ['checkpoint', 'update', 'terminal'],
      'the update commit reports its own signal',
    );
  });

  test('every commit is marked in the performance timeline without a listener', async () => {
    // No debug flag: marks must not be gated on it, because a consumer that
    // loads after hydration cannot have registered a listener in time.
    performance.clearMarks();
    const panel = element('weather-panel', { hook() {}, setState() {} });
    predefine('weather-panel');

    const checkpoint = buildUpdatableBoundary(0, 0, [panel], {
      state: { status: 'loading' },
    });
    enqueue(checkpoint.sentinel);
    await flush();
    enqueue(buildStateUpdate(1, 0, { status: 'ready' }).sentinel);
    await flush();
    enqueue(buildMarkerless(2, 1, {}).sentinel);
    await flush();

    assert.equal(boundaryEvents().length, 0, 'marks are not the debug event');
    assert.deepEqual(
      performance.getEntriesByType('mark').map((entry) => entry.name),
      ['webui:boundary:0', 'webui:boundary:0:update', 'webui:streaming:terminal'],
      'checkpoint, update, and terminal are each readable after the fact',
    );
    performance.clearMarks();
  });

  test('a malformed envelope halts and fully cleans its scaffold, keeping roots', async () => {
    let abandoned = 0;
    const root0 = element('my-root', {
      hook() {},
      abandon() { abandoned++; },
    });
    const b = buildRawBoundary(0, '[1,0,0,{', [root0]);

    enqueue(b.sentinel);
    await flush();
    assert.equal(__isHaltedForTests(), true, 'invalid JSON must halt');
    // The rejected boundary leaves no discoverable payload/markers, but its
    // SSR root stays in the tree.
    assertScaffoldCleaned(b);
    assert.equal(abandoned, 1, 'fatal cleanup clears element-owned deferral');
    assert.equal(documentWalkCalls > 0, true, 'failure may perform a bounded document sweep');

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
    const scriptEl = element('script', { attrs: { 'data-webui-boundary': '' }, text: JSON.stringify([1, 0, 0, 0, {}]) });
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

  test('a marker-bearing terminal boundary is rejected', async () => {
    const b = buildBoundary(0, 1, [], {});
    enqueue(b.sentinel);
    await flush();

    assert.equal(__isHaltedForTests(), true);
    assertScaffoldCleaned(b);
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

    assert.deepEqual(activated, ['my-outer', 'my-inner']);
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

  test('an undefined outer blocks its defined child while siblings continue', async () => {
    const activated: string[] = [];
    const child = element('my-barrier-child', { hook() { activated.push('child'); } });
    const outer = element('my-barrier-outer', {
      hook() { activated.push('outer'); },
      children: [child],
    });
    const sibling = element('my-barrier-sibling', { hook() { activated.push('sibling'); } });
    const b = buildBoundary(0, 0, [outer, sibling], { state: {} });

    predefine('my-barrier-child', 'my-barrier-sibling');
    enqueue(b.sentinel);
    await flush();
    const terminal = buildMarkerless(1, 1, {});
    enqueue(terminal.sentinel);
    await flush();

    assert.deepEqual(activated, ['sibling']);
    assert.equal(whenDefinedCalls.get('my-barrier-outer'), 1);
    assert.equal(whenDefinedCalls.get('my-barrier-child'), undefined);

    defineTag('my-barrier-outer');
    await flush();

    assert.deepEqual(activated, ['sibling', 'outer', 'child']);
    assert.equal(__getLifecycleStateForTests().completed, true);
  });

  test('an undefined child gets no waiter until its undefined outer activates', async () => {
    const activated: string[] = [];
    const child = element('my-late-child', { hook() { activated.push('child'); } });
    const outer = element('my-late-outer', {
      hook() { activated.push('outer'); },
      children: [child],
    });
    const b = buildBoundary(0, 0, [outer], { state: {} });

    enqueue(b.sentinel);
    await flush();
    const terminal = buildMarkerless(1, 1, {});
    enqueue(terminal.sentinel);
    await flush();
    assert.deepEqual(activated, []);
    assert.equal(whenDefinedCalls.get('my-late-outer'), 1);
    assert.equal(whenDefinedCalls.get('my-late-child'), undefined);

    defineTag('my-late-outer');
    await flush();
    assert.deepEqual(activated, ['outer']);
    assert.equal(whenDefinedCalls.get('my-late-child'), 1);

    defineTag('my-late-child');
    await flush();
    assert.deepEqual(activated, ['outer', 'child']);
    assert.equal(__getLifecycleStateForTests().completed, true);
  });

  test('an undefined shadow host retains its shadow child behind the barrier', async () => {
    const activated: string[] = [];
    const child = element('my-late-shadow-child', { hook() { activated.push('child'); } });
    const outer = element('my-late-shadow-outer', {
      hook() { activated.push('outer'); },
      shadowChildren: [child],
    });
    const b = buildBoundary(0, 0, [outer], { state: {} });

    predefine('my-late-shadow-child');
    enqueue(b.sentinel);
    await flush();
    assert.deepEqual(activated, []);

    defineTag('my-late-shadow-outer');
    await flush();
    assert.deepEqual(activated, ['outer', 'child']);
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
    const b = buildBoundary(0, 0, [webuiRoot], { state: {} });

    predefine('my-owned');
    enqueue(b.sentinel);
    await flush();
    const terminal = buildMarkerless(1, 1, {});
    enqueue(terminal.sentinel);
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
    const b = buildBoundary(0, 0, [a, c], { state: {} });

    enqueue(b.sentinel);
    await flush();
    const terminal = buildMarkerless(1, 1, {});
    enqueue(terminal.sentinel);
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
    assert.deepEqual(activated, ['a', 'c'], 'both deferred instances activate once defined');
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
    const second = buildBoundary(1, 0, secondRoots, { state: { boundary: 1 } });
    enqueue(second.sentinel);
    await flush();
    const terminal = buildMarkerless(2, 1, {});
    enqueue(terminal.sentinel);
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
    const b = buildBoundary(0, 0, [late], { state: { exact: true } });
    enqueue(b.sentinel);
    await flush();
    const terminal = buildMarkerless(1, 1, {});
    enqueue(terminal.sentinel);
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
    const b = buildBoundary(0, 0, [late], { state: { resumed: true } });
    enqueue(b.sentinel);
    await flush();
    const terminal = buildMarkerless(1, 1, {});
    enqueue(terminal.sentinel);
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
    const b = buildBoundary(0, 0, [late], { state: { abandoned: true } });
    enqueue(b.sentinel);
    await flush();
    const terminal = buildMarkerless(1, 1, {});
    enqueue(terminal.sentinel);
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

  test('failure abandons a detached undefined outer and its retained child', async () => {
    let outerAbandoned = 0;
    let childAbandoned = 0;
    const child = element('my-retained-child', {
      hook() {},
      abandon() { childAbandoned++; },
    });
    const outer = element('my-retained-outer', {
      hook() {},
      abandon() { outerAbandoned++; },
      children: [child],
    });
    const b = buildBoundary(0, 0, [outer], { state: {} });
    enqueue(b.sentinel);
    await flush();
    detach(outer);

    const bad = buildRawBoundary(1, '[1,1,0,{', []);
    enqueue(bad.sentinel);
    await flush();

    assert.equal(outerAbandoned, 1);
    assert.equal(childAbandoned, 1);
    assert.equal(hasWs(outer), false);
    assert.equal(hasWs(child), false);
  });

  test('a terminal record with an unexpected payload still completes', async () => {
    // The terminal path reads no payload fields, so an unrecognized one is
    // ignored rather than halting a page that has already rendered and
    // hydrated. Incompatible terminal semantics must bump the envelope
    // version instead of relying on a per-field check here.
    const b = buildMarkerless(0, 1, { boom: 1 });
    enqueue(b.sentinel);
    await flush();

    assert.equal(__isHaltedForTests(), false, 'an unknown terminal field is tolerated');
    const lc = __getLifecycleStateForTests();
    assert.equal(lc.terminalReached, true, 'terminal is reached');
    assert.equal(lc.completed, true, 'hydration still completes');
    assert.equal(lc.pendingBoundaries, 0, 'the pending boundary still settles');
    assert.equal(dispatchedEvents.some((e) => e.type === 'webui:hydration-complete'), true);
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
    const b = buildBoundary(0, 0, roots, { state: {} });
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
    const b = buildBoundary(0, 0, [wrapper], { state: {} });
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
    // A non-terminal boundary opens an undefined-tag waiter. The following
    // canonical empty terminal commits, so completion is gated on activation.
    const activated: string[] = [];
    const open = element('my-open', { hook() { activated.push('x'); } });
    const openBoundary = buildBoundary(0, 0, [open], { state: {} });
    enqueue(openBoundary.sentinel);
    await flush();
    const terminal = buildMarkerless(1, 1, {});
    enqueue(terminal.sentinel);
    await flush();

    let lc = __getLifecycleStateForTests();
    assert.equal(lc.terminalReached, true, 'terminal committed');
    assert.equal(lc.completed, false, 'completion gated on the open late activation');
    assert.equal(__pendingTagWaiterCountForTests(), 1);

    // A malformed record arrives after the terminal: it must be rejected (its
    // scaffold cleaned + stream halted/aborted), and settling the abandoned
    // waiter during failure must NOT dispatch hydration-complete.
    const post = buildRawBoundary(2, '[bad', []);
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

  test('pre-sentinel truncation abandons roots found only by the failure walk', async () => {
    __installTruncationGuardForTests();
    let abandoned = 0;
    const orphan = element('my-pre-sentinel', {
      attrs: { 'data-ws': '' },
      hook() {},
      abandon() { abandoned++; },
    });
    const root = body();
    link(root, [orphan]);

    fireDomContentLoaded();
    await flush();

    assert.equal(__isHaltedForTests(), true);
    assert.equal(abandoned, 1);
    assert.equal(hasWs(orphan), false);
    assert.equal(documentWalkCalls > 0, true);
    assert.equal(__getLifecycleStateForTests().completed, false);
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
    const b = buildBoundary(0, 0, [root], { state: { a: 1 } });

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
    const b1 = buildBoundary(1, 0, [r1], { state: { which: 'second' } });
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
    const b1 = buildBoundary(1, 0, [other], { state: { seq: 1 } });
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
    const b = buildBoundary(0, 0, [late], {});
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

  test('a committed defined root with no activation hook is a fatal missing-template failure', async () => {
    const root = element('my-nohook', { attrs: { 'data-ws': '' } });
    const b = buildBoundary(0, 0, [root], { state: {} });

    predefine('my-nohook');
    const previousError = console.error;
    console.error = () => {};
    try {
      enqueue(b.sentinel);
      await flush();
    } finally {
      console.error = previousError;
    }

    assert.equal(__isHaltedForTests(), true);
    assert.equal(__getLifecycleStateForTests().completed, false);
    assert.equal(hasWs(root), false, 'fatal cleanup strips data-ws');
    assert.equal(root.parentNode, b.root, 'the root itself is retained');
    assert.equal(hasPending(root), false);
  });

  test('an explicit missing-template activation outcome halts without completion', async () => {
    const root = element('my-missing-meta', {
      attrs: { 'data-ws': '' },
      activationOutcome: 3,
      hook() {},
    });
    const b = buildBoundary(0, 0, [root], { state: {} });
    predefine('my-missing-meta');

    const previousError = console.error;
    console.error = () => {};
    try {
      enqueue(b.sentinel);
      await flush();
    } finally {
      console.error = previousError;
    }

    assert.equal(__isHaltedForTests(), true);
    assert.equal(hasWs(root), false);
    assert.equal(__getLifecycleStateForTests().completed, false);
    assert.equal(dispatchedEvents.some((e) => e.type === 'webui:hydration-complete'), false);
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
    const b = buildBoundary(0, 0, [thrower, ok], { state: {} });

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
      activationOutcome: 2,
      hook() {
        called = true; // opts out: records the call but performs no activation
      },
    });
    const b = buildBoundary(0, 0, [host], { state: {} });

    predefine('my-optout');
    enqueue(b.sentinel);
    await flush();

    assert.equal(called, true, 'the opt-out hook was still invoked');
    assert.equal(__isHaltedForTests(), false);
    assert.equal(hasWs(host), false, 'data-ws stripped from a committed opt-out root');
    assert.equal(host.parentNode, b.root, 'the opt-out root is retained');
  });
});

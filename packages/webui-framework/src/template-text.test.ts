// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { strict as assert } from 'node:assert';
import { test } from 'node:test';
import type { TemplateBlockMeta } from './template.js';
import type { TemplateInstance } from './element/types.js';

class TestNode {
  parentNode: TestNode | null = null;
  childNodes: TestNode[] = [];
  data = '';
  constructor(readonly nodeType: number, readonly localName = '') {}
  get ownerDocument(): Document { return document; }
  get firstChild(): TestNode | null { return this.childNodes[0] ?? null; }
  get lastChild(): TestNode | null { return this.childNodes[this.childNodes.length - 1] ?? null; }
  get nextSibling(): TestNode | null {
    const siblings = this.parentNode?.childNodes;
    return siblings ? siblings[siblings.indexOf(this) + 1] ?? null : null;
  }
  get previousSibling(): TestNode | null {
    const siblings = this.parentNode?.childNodes;
    return siblings ? siblings[siblings.indexOf(this) - 1] ?? null : null;
  }
  hasAttribute(): boolean { return false; }
  insertBefore(node: TestNode, before: TestNode | null): TestNode {
    node.parentNode?.removeChild(node);
    const index = before === null ? this.childNodes.length : this.childNodes.indexOf(before);
    assert.ok(index >= 0, 'insertion reference belongs to its parent');
    this.childNodes.splice(index, 0, node);
    node.parentNode = this;
    return node;
  }
  removeChild(node: TestNode): TestNode {
    const index = this.childNodes.indexOf(node);
    assert.ok(index >= 0, 'only owned children are removed');
    this.childNodes.splice(index, 1);
    node.parentNode = null;
    return node;
  }
  replaceChild(node: TestNode, previous: TestNode): TestNode {
    this.insertBefore(node, previous);
    this.removeChild(previous);
    return previous;
  }
  remove(): void { this.parentNode?.removeChild(this); }
  replaceWith(node: TestNode): void { this.parentNode?.replaceChild(node, this); }
}

Object.defineProperties(globalThis, {
  HTMLElement: { configurable: true, value: class extends TestNode {
    constructor() { super(1, 'text-binding-owner'); }
  } },
  document: { configurable: true, value: {
    nodeType: 9,
    readyState: 'loading',
    querySelector: () => null,
    querySelectorAll: () => [],
    getElementById: () => null,
    createTextNode(data: string) { return Object.assign(new TestNode(3), { data }); },
    createComment(data: string) { return Object.assign(new TestNode(8), { data }); },
    createElement(tag: string) { return new TestNode(1, tag); },
  } },
  window: { configurable: true, value: { __webui: { templates: {} } } },
  customElements: { configurable: true, value: { get: () => undefined } },
});

const { TemplateElement } = await import('./template-element.js');

function node(value: TestNode): Node { return value as unknown as Node; }

function fixture(hydrate: boolean, value?: string, tag = 'details-card') {
  const root = new TestNode(11);
  const receiver = new TestNode(1, tag);
  const tail = new TestNode(1, 'span');
  root.insertBefore(receiver, null);
  receiver.insertBefore(tail, null);
  const adopted = value === undefined ? undefined : Object.assign(new TestNode(3), { data: value });
  if (adopted) receiver.insertBefore(adopted, tail);
  const host = new TemplateElement();
  const meta: TemplateBlockMeta = {
    h: `<${tag}><span></span></${tag}>`, l: [3, 3, 1],
    tx: [[[1, 0], [['details']], 0, 1]],
  };
  const instance = hydrate
    ? host['$hydrate'](node(root), meta)
    : host['$wire'](node(root), meta);
  const set = (details: unknown) => {
    Object.assign(host, { details });
    host.$updateInstance(instance);
  };
  const assignedText = () => receiver.childNodes.filter(child => child.nodeType === 3);
  return { host, root, receiver, tail, adopted, instance, set, assignedText };
}

test('empty hydrated slot text has no assigned Text before client state arrives', () => {
  const f = fixture(true);
  assert.deepEqual(f.assignedText(), []);
  f.host.$updateInstance(f.instance, true);
  assert.deepEqual(f.assignedText(), []);
  f.set('Details');
  const text = f.assignedText()[0];
  assert.equal(text.data, 'Details');
  assert.equal(text.nextSibling, f.tail);
  f.set('');
  assert.deepEqual(f.assignedText(), []);
  f.set('Again');
  assert.equal(f.assignedText()[0], text, 'detached text is reused');
});

test('hydration trusts absent state and retains the adopted Text through empty transitions', () => {
  const f = fixture(true, 'Server details');
  f.host.$updateInstance(f.instance, true);
  assert.equal(f.adopted?.data, 'Server details');
  assert.equal(f.assignedText()[0], f.adopted);
  f.set('');
  assert.deepEqual(f.assignedText(), []);
  f.set('Client details');
  assert.equal(f.assignedText()[0], f.adopted);
  assert.equal(f.adopted?.data, 'Client details');
});

for (const hydrate of [false, true]) {
  test(`slot text preserves conversion and static runs (hydrate=${hydrate})`, () => {
    const f = fixture(hydrate);
    for (const empty of ['', null, undefined]) {
      f.set(empty);
      assert.deepEqual(f.assignedText(), []);
    }
    for (const value of [0, false, ' ', '\n']) {
      f.set(value);
      assert.equal(f.assignedText()[0]?.data, String(value));
    }
    f.instance.texts[0].parts = [' Before ', ['details'], ' after '];
    f.set(null);
    assert.equal(f.assignedText()[0]?.data, ' Before  after ');
    f.instance.texts[0].parts = [' ', ['details'], '\n'];
    f.set('');
    assert.equal(f.assignedText()[0]?.data, ' \n');
  });
}

test('ordinary non-slot text retains its anchor-free representation', () => {
  const f = fixture(false, undefined, 'p');
  f.set('');
  const text = f.assignedText()[0];
  assert.equal(text.data, '');
  assert.equal(f.receiver.childNodes.some(child => child.nodeType === 8), false);
  f.set('Text');
  f.set('');
  assert.equal(f.assignedText()[0], text);
  assert.equal(f.receiver.childNodes.some(child => child.nodeType === 8), false);
});

test('adjacent client text slots retain insertion order through independent empty transitions', () => {
  const host = new TemplateElement();
  const root = new TestNode(1, 'details-card');
  const tail = new TestNode(1, 'span');
  root.insertBefore(tail, null);
  const instance = host['$wire'](node(root), {
    h: '<span></span>',
    tx: [
      [[0, 0], [['first']]],
      [[0, 0, 1], [['second']]],
      [[0, 0, 2], [['third']]],
    ],
  });
  instance.nodes = [...root.childNodes] as unknown as Node[];
  const set = (first: string, second: string, third: string) => {
    Object.assign(host, { first, second, third });
    host.$updateInstance(instance);
  };
  set('', '', '');
  assert.equal(root.childNodes.some(child => child.nodeType === 3), false);
  set('', 'B', '');
  set('A', 'B', 'C');
  assert.deepEqual(root.childNodes.map(child => child === tail ? 'tail' : child.data), ['A', 'B', 'C', 'tail']);
  set('', 'B', '');
  set('A2', '', 'C2');
  assert.deepEqual(root.childNodes.filter(child => child.nodeType === 3).map(child => child.data), ['A2', 'C2']);
  assert.deepEqual(instance.nodes, root.childNodes, 'owned node snapshots follow text/anchor swaps');
  host.$removeInstance(instance);
  assert.deepEqual(root.childNodes, [], 'block disposal removes attached nodes, not cached detached text');
});

test('hydrated block-root text updates ancestor ownership before removal', () => {
  const host = new TemplateElement();
  const root = new TestNode(1, 'details-card');
  const start = Object.assign(new TestNode(8), { data: 'wc' });
  const text = Object.assign(new TestNode(3), { data: 'Server details' });
  const end = Object.assign(new TestNode(8), { data: '/wc' });
  for (const child of [start, text, end]) root.insertBefore(child, null);
  const instance = host['$hydrate'](node(root), {
    h: '', l: [1], tx: [[[0, 0], [['details']]]],
  }, undefined, node(start), node(end));
  const parent: TemplateInstance = {
    container: node(root) as ParentNode & Node,
    nodes: [node(start), node(text), node(end)],
    texts: [], attrs: [], conds: [], repeats: [],
  };
  instance.parent = parent;
  Object.assign(host, { details: '' });
  host.$updateInstance(instance);
  assert.equal(root.childNodes.some(child => child.nodeType === 3), false);
  assert.deepEqual(parent.nodes, root.childNodes);
  Object.assign(host, { details: 'Restored' });
  host.$updateInstance(instance);
  assert.equal(root.childNodes[1], text);
  host.$removeInstance(instance);
  assert.deepEqual(root.childNodes, [start, end]);
});

test('hydration owns projected markers inside a template-empty receiver without sharing adjacent text', () => {
  const host = new TemplateElement();
  const root = new TestNode(11);
  const receiver = new TestNode(1, 'details-card');
  const start = Object.assign(new TestNode(8), { data: 'wc' });
  const middle = Object.assign(new TestNode(3), { data: 'Middle' });
  const end = Object.assign(new TestNode(8), { data: '/wc' });
  root.insertBefore(receiver, null);
  for (const child of [start, middle, end]) receiver.insertBefore(child, null);
  const meta: TemplateBlockMeta = {
    h: '<details-card></details-card>', l: [3, 0],
    tx: [[[1, 0], [['first']]], [[1, 0, 2], [['last']]]],
    c: [[[() => true, ['showMiddle']], 0, [1, 0, 1]]],
  };
  host['$meta'] = {
    ...meta, v: 3, b: [{ h: '', l: [1], tx: [[[0, 0], [['middle']]]] }],
  };
  const instance = host['$hydrate'](node(root), meta);
  assert.notEqual(instance.texts[0].node, instance.texts[1].node);
  assert.equal(instance.conds[0].instance?.texts[0].node, middle);
  Object.assign(host, { first: 'A', last: 'C' });
  host['$patchText'](instance.texts[1]);
  host['$patchText'](instance.texts[0]);
  assert.deepEqual(receiver.childNodes.filter(child => child.nodeType === 3).map(child => child.data), ['A', 'Middle', 'C']);
  Object.assign(host, { first: '', last: '' });
  host['$patchText'](instance.texts[0]);
  host['$patchText'](instance.texts[1]);
  assert.deepEqual(receiver.childNodes.filter(child => child.nodeType === 3), [middle]);
});

for (const known of [true, false, undefined]) {
  test(`body-only writes activate empty SSR text conditionals only when true (${known})`, () => {
    const host = new TemplateElement();
    Object.assign(host, { middle: 'Unpainted client default' });
    if (known !== undefined) Object.assign(host, { show: known });
    const receiver = new TestNode(1, 'details-card');
    const start = Object.assign(new TestNode(8), { data: 'wc' });
    const end = Object.assign(new TestNode(8), { data: '/wc' });
    receiver.insertBefore(start, null);
    receiver.insertBefore(end, null);
    const meta: TemplateBlockMeta = {
      h: '', l: [1],
      c: [[[(resolve) => resolve('show') === true, ['show']], 0, [0, 0]]],
    };
    host['$meta'] = {
      ...meta, v: 3, b: [{ h: '', l: [1], tx: [[[0, 0], [['middle']]]] }],
    };
    const instance = host['$hydrate'](node(receiver), meta);
    assert.equal(receiver.childNodes.some(child => child.nodeType === 3), false);
    host['$root'] = instance;
    host['$stageTemplate'] = (_wrapper, block) => { assert.equal(block.h, ''); };
    host['$buildPathIndex']();
    const bindings = host['$wildcardBindings'];
    assert.ok(bindings);
    assert.ok(bindings.conds.includes(instance.conds[0]), 'body-only writes are indexed before the empty view exists');
    Object.assign(host, { middle: 'Middle' });
    host.$updateInstance(bindings);
    const child = instance.conds[0].instance;
    if (known !== true) {
      assert.equal(child, null, 'false or absent state must not activate SSR-absent content');
      Object.assign(host, { show: true });
      host.$updateInstance(bindings);
      assert.equal(receiver.childNodes.find(node => node.nodeType === 3)?.data, 'Middle',
        'a later true condition mounts the latest body value, not its pre-hydration default');
      const activated = instance.conds[0].instance;
      assert.ok(activated);
      host.$removeInstance(activated);
      assert.deepEqual(receiver.childNodes, []);
      return;
    }
    assert.ok(child, 'a body-only write must create the active empty text view');
    assert.equal(receiver.childNodes.find(node => node.nodeType === 3)?.data, 'Middle');
    Object.assign(host, { middle: '' });
    host.$updateInstance(child);
    assert.equal(receiver.childNodes.some(child => child.nodeType === 3), false);
    host.$removeInstance(child);
    assert.deepEqual(receiver.childNodes, []);
  });
}

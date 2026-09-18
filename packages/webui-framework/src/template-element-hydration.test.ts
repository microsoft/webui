// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { strict as assert } from 'node:assert';
import { describe, test } from 'node:test';
import type { SSRIndex } from './element/hydration.js';
import { createFragmentWork } from './element/fragment-work.js';
import { EMPTY_BINDINGS, type RepeatBinding, type ScopeFrame, type TemplateInstance } from './element/types.js';
import type { TemplateBlockMeta } from './template-types.js';

function text(data: string): Text {
  return { nodeType: 3, data } as Text;
}

function comment(data: string): Comment {
  return { nodeType: 8, data } as Comment;
}

function parent(children: ChildNode[]): DocumentFragment {
  const root = { nodeType: 11 } as DocumentFragment;
  const link = (): void => {
    Object.assign(root, {
      childNodes: children, firstChild: children[0] ?? null, lastChild: children.at(-1) ?? null,
    });
    for (let i = 0; i < children.length; i++) {
      Object.assign(children[i], {
        parentNode: root, previousSibling: children[i - 1] ?? null, nextSibling: children[i + 1] ?? null,
      });
    }
  };
  Object.defineProperty(root, 'insertBefore', {
    value(node: ChildNode, before: ChildNode | null): ChildNode {
      const offset = before ? children.indexOf(before) : children.length;
      assert.ok(offset >= 0);
      children.splice(offset, 0, node);
      link();
      return node;
    },
  });
  link();
  return root;
}

Object.defineProperty(globalThis, 'HTMLElement', {
  value: class HTMLElement {},
  configurable: true,
});
Object.defineProperty(globalThis, 'document', {
  value: {
    createElement(name: string) {
      assert.equal(name, 'template');
      return {
        content: parent([]),
        set innerHTML(html: string) { assert.equal(html, ''); },
      };
    },
    createTextNode: text,
  },
  configurable: true,
});

const { TemplateElement } = await import('./template-element.js');

class TextHost extends TemplateElement {
  known = true;
  value: unknown = '<b>known</b>';
  resolveCalls = 0;
  onResolve?: () => unknown;
  readonly finalized: TemplateInstance[] = [];

  override $hasStateRoot(_path: string, _scope?: ScopeFrame): boolean {
    return this.known;
  }

  override $resolveValue(_path: string, _scope?: ScopeFrame): unknown {
    this.resolveCalls++;
    return this.onResolve ? this.onResolve() : this.value;
  }

  protected override $finalize(instance: TemplateInstance): void {
    for (let i = 0; i < instance.texts.length; i++) {
      assert.ok(Object.hasOwn(instance.texts, i), 'finalization cannot observe a hole');
    }
    this.finalized.push(instance);
  }
}

function section(children: ChildNode[], scope?: ScopeFrame, range = false) {
  const container = parent(children);
  const instance: TemplateInstance = {
    container, scope, nodes: children.slice(), texts: EMPTY_BINDINGS,
    attrs: EMPTY_BINDINGS, conds: EMPTY_BINDINGS, repeats: EMPTY_BINDINGS,
  };
  if (range) instance.range = true;
  const index: SSRIndex = {
    elements: [container], conds: [], repeats: [], renders: [], raws: [],
    start: null, end: null, comments: [],
  };
  return { instance, index, container };
}

function textThenRaw(scope?: ScopeFrame) {
  const before = text('trusted prefix'), start = comment('w0'), end = comment('/w0');
  const fixture = section([before, start, text('trusted raw'), end], scope, true);
  fixture.index.raws.push([start, end]);
  const meta: TemplateBlockMeta = {
    h: '', tx: [[[0, 0], [['prefix']], 5], [[0, 0, 1], [['html']], 1]],
  };
  return { ...fixture, meta, before, start, end };
}

test('outlet-only range ownership preserves the ordinary repeat mismatch diagnostic', t => {
  const host = new TextHost();
  const fixture = section([]);
  const binding: RepeatBinding = {
    markerId: 0, collection: 'items', itemVar: 'item', blockIndex: 0,
    container: fixture.container, start: comment('wr'), end: null,
    owner: fixture.instance, instances: [section([]).instance],
  };
  Reflect.set(host, '$meta', { h: '' });
  Reflect.set(host, '$fragmentWork', createFragmentWork());
  const warning = t.mock.method(console, 'warn', () => {});
  host.$hydratedRepeat(binding, [], true);
  assert.equal(warning.mock.callCount(), 1);
  assert.match(warning.mock.calls[0].arguments[0], /repeat marker count \(1\)/);
});

describe('SSR text binding storage', () => {
  test('empty root text wiring keeps omitted ownership and publishes the real bound node', () => {
    const { instance, index, container } = section([]);
    instance.nodes = EMPTY_BINDINGS;
    const host = new TextHost();
    host.$wireHydrationSection(instance, { h: '', tx: [[[0, 0], [['value']]]] }, index);
    assert.equal(instance.nodes, EMPTY_BINDINGS);
    assert.equal(instance.texts.length, 1);
    assert.equal(instance.texts[0].node, container.firstChild);
    assert.equal(instance.texts[0].node.data, '');
    assert.deepEqual(host.finalized, [instance]);
    instance.texts[0].node.data = 'updated';
    assert.equal((container.firstChild as Text).data, 'updated');
  });

  for (const kind of [2, 3, 4, 5, 6, 7]) {
    for (const offset of [0, 1]) {
      test(`directly resolves successor kind ${kind}, index ${offset} without parsing or reconstruction`, () => {
        const before = text('trusted');
        const ref = kind === 6 ? { nodeType: 1, localName: 'i' } as Element : comment('boundary');
        const { instance, index } = section([before, ref]);
        switch (kind) {
          case 2: index.conds[offset] = ref as Comment; break;
          case 3: index.repeats[offset] = ref as Comment; break;
          case 4: index.renders[offset] = ref as Comment; break;
          case 5: index.raws[offset] = [ref as Comment, comment('/w0')]; break;
          case 6: index.elements[offset + 1] = ref; break;
          case 7: index.comments[0] = []; index.comments[0][offset] = ref as Comment; break;
        }
        const meta: TemplateBlockMeta = {
          h: '<i></i>', tx: [[[0, 0], [['value']], offset * 8 + kind]],
        };
        const host = new TextHost();
        const originalMap = globalThis.Map, originalWeakMap = globalThis.WeakMap;
        const originalArray = globalThis.Int32Array;
        const fail = { construct(): never { throw new Error('successor reconstruction is forbidden'); } };
        try {
          globalThis.Map = new Proxy(originalMap, fail);
          globalThis.WeakMap = new Proxy(originalWeakMap, fail);
          globalThis.Int32Array = new Proxy(originalArray, fail);
          // The document stub also rejects parsing nonempty template HTML.
          host.$wireHydrationSection(instance, meta, index);
        } finally {
          globalThis.Map = originalMap;
          globalThis.WeakMap = originalWeakMap;
          globalThis.Int32Array = originalArray;
        }
        assert.equal(instance.texts[0].node, before);
        assert.equal(before.nextSibling, ref);
        assert.equal(host.resolveCalls, 0);
      });
    }
  }

  for (const range of [false, true]) {
    for (const after of ['', 'after SSR']) {
      test(`authored empty comments separate ${after ? 'nonempty' : 'empty'} text in ${range ? 'fragment' : 'ordinary'} sections`, () => {
        const before = text('before SSR'), authored = comment('');
        const afterNode = after ? text(after) : undefined;
        const { instance, index, container } = section(
          afterNode ? [before, authored, afterNode] : [before, authored], undefined, range,
        );
        index.comments[0] = [authored];
        const meta: TemplateBlockMeta = {
          h: '<!---->', tx: [[[0, 0], [['before']], 7], [[0, 1], [['after']]]],
        };
        new TextHost().$wireHydrationSection(instance, meta, index);
        const [first, second] = instance.texts;
        assert.equal(first.node, before);
        assert.notEqual(second.node, before, 'an authored blank comment is not an erased text marker');
        assert.equal(second.node.data, after);
        assert.equal(second.node.previousSibling, authored);
        first.node.data = 'first update';
        second.node.data = 'second update';
        assert.deepEqual(Array.from(container.childNodes, node => (node as CharacterData).data),
          ['first update', '', 'second update']);
        if (afterNode) assert.equal(second.node, afterNode);
      });
    }
  }

  test('omitted successors mean exact root section end or nested parent end, not a sibling search', () => {
    const earlier = text('outside'), start = comment('wf'), end = comment('/wf');
    const { instance, index, container } = section([earlier, start, end], undefined, true);
    index.start = start;
    index.end = end;
    instance.nodes = [];
    new TextHost().$wireHydrationSection(instance, { h: '', tx: [[[0, 0], [['empty']]]] }, index);
    assert.notEqual(instance.texts[0].node, earlier);
    assert.equal(instance.texts[0].node.previousSibling, start);
    assert.equal(instance.texts[0].node.nextSibling, end);
    assert.equal(container.lastChild, end);

    const inner = text('nested');
    const nested = parent([inner]) as unknown as Element;
    const outer = section([nested, comment('/wf')], undefined, true);
    outer.index.elements.push(nested);
    outer.index.end = outer.container.lastChild as Comment;
    new TextHost().$wireHydrationSection(outer.instance,
      { h: '<span></span>', tx: [[[1, 0], [['nested']]]] }, outer.index);
    assert.equal(outer.instance.texts[0].node, inner);
  });

  test('fills the known two-binding group by index without push growth or SSR writes', () => {
    const first = text('first SSR'), marker = comment('wc'), second = text('second SSR');
    const { instance, index } = section([first, marker, comment('/wc'), second]);
    index.conds.push(marker);
    const meta: TemplateBlockMeta = {
      h: '', c: [[[() => true, []], 0, [0, 0, 1]]],
      tx: [[[0, 0], [['first']], 2], [[0, 0, 2], [['second']]]],
    };
    const host = new TextHost();
    let bindingPushes = 0;
    const originalPush = Array.prototype.push;
    try {
      Array.prototype.push = function<T>(this: T[], ...values: T[]): number {
        for (const value of values) {
          if (typeof value === 'object' && value !== null && 'node' in value && 'parts' in value) {
            bindingPushes++;
          }
        }
        return originalPush.apply(this, values);
      };
      host.$wireHydrationSection(instance, meta, index);
    } finally {
      Array.prototype.push = originalPush;
    }
    assert.equal(bindingPushes, 0);
    assert.equal(instance.texts.length, 2);
    assert.deepEqual(instance.texts.map(binding => binding.node), [first, second]);
    assert.notEqual(instance.texts, EMPTY_BINDINGS);
    assert.deepEqual([first.data, second.data], ['first SSR', 'second SSR']);
    assert.equal(host.resolveCalls, 0);
    assert.deepEqual(host.finalized, [instance]);
  });

  for (const tx of [undefined, []]) {
    test(`retains shared storage for ${tx ? 'explicitly empty' : 'absent'} text metadata`, () => {
      const { instance, index } = section([]);
      const host = new TextHost();
      const original = globalThis.Int32Array;
      let emptyArrays = 0;
      try {
        globalThis.Int32Array = new Proxy(original, {
          construct(target, args, newTarget) {
            if (args[0] === 0) emptyArrays++;
            return Reflect.construct(target, args, newTarget);
          },
        });
        host.$wireHydrationSection(instance, { h: '', tx }, index);
      } finally {
        globalThis.Int32Array = original;
      }
      assert.equal(emptyArrays, 0, 'empty marker boundaries must reuse shared storage');
      assert.equal(instance.texts, EMPTY_BINDINGS);
      assert.equal(Object.isFrozen(instance.texts), true);
      assert.deepEqual(host.finalized, [instance]);
    });
  }

  for (const allMissing of [false, true]) {
    test(`compacts ${allMissing ? 'all' : 'some'} missing parents without exposing holes`, () => {
      const node = text('trusted');
      const { instance, index } = section([node]);
      const meta: TemplateBlockMeta = {
        h: '', tx: [
          [[8, 0], [['missing']]],
          [[allMissing ? 9 : 0, 0], [['present']]],
          [[10, 0], [['alsoMissing']]],
        ],
      };
      const host = new TextHost();
      host.$wireHydrationSection(instance, meta, index);
      assert.equal(instance.texts.length, allMissing ? 0 : 1);
      if (allMissing) assert.equal(instance.texts, EMPTY_BINDINGS);
      else assert.equal(instance.texts[0].node, node);
      assert.deepEqual(host.finalized, [instance]);
    });
  }

  for (const range of [false, true]) {
    for (const known of [false, true]) {
      test(`preserves ${known ? 'known' : 'unknown'} raw values and ${range ? 'fragment' : 'ordinary'} ownership`, () => {
        const start = comment('w0'), end = comment('/w0'), after = text('trusted suffix');
        const { instance, index } = section([start, text('trusted raw'), end, after], undefined, range);
        index.raws.push([start, end]);
        const meta: TemplateBlockMeta = {
          h: '', tx: [[[0, 0], [['html']], 1], [[0, 0, 1], [['suffix']]]],
        };
        const host = new TextHost();
        host.known = known;
        host.$wireHydrationSection(instance, meta, index);
        const [raw, plain] = instance.texts;
        assert.equal(instance.texts.length, 2);
        assert.equal(raw.node, start);
        assert.equal(raw.rawEnd, end);
        assert.equal(raw.rawOwner, instance);
        assert.equal(raw.owner, range ? instance : undefined);
        assert.equal(raw.rawValue, known ? host.value : undefined);
        assert.equal(host.resolveCalls, known ? 1 : 0);
        assert.equal(plain.node, after);
        assert.equal(plain.owner, range ? instance : undefined);
        assert.deepEqual([start.data, end.data, after.data], ['w0', '/w0', 'trusted suffix']);
      });
    }
  }

  test('retains unknown caller scopes without resolving their raw inputs', () => {
    const scope: ScopeFrame = { name: 'row', value: undefined, known: false };
    const { instance, index, meta } = textThenRaw(scope);
    const host = new TextHost();
    host.onResolve = () => { throw new Error('unknown input must not be read'); };
    host.$wireHydrationSection(instance, meta, index);
    assert.equal(host.resolveCalls, 0);
    assert.equal(instance.texts[1].rawValue, undefined);
    for (const binding of instance.texts) assert.equal(binding.scope, scope);
  });

  test('keeps the unfinished group private during a reentrant raw-value read', () => {
    const outer = textThenRaw();
    const inner = section([text('inner SSR')]);
    const host = new TextHost(), nested = new TextHost();
    host.onResolve = () => {
      assert.equal(outer.instance.texts, EMPTY_BINDINGS);
      nested.$wireHydrationSection(inner.instance, { h: '', tx: [[[0, 0], [['inner']]]] }, inner.index);
      assert.equal(inner.instance.texts.length, 1);
      assert.equal(outer.instance.texts, EMPTY_BINDINGS);
      return '<b>outer</b>';
    };
    host.$wireHydrationSection(outer.instance, outer.meta, outer.index);
    assert.equal(outer.instance.texts.length, 2);
    assert.equal(outer.instance.texts[1].rawValue, '<b>outer</b>');
    assert.notEqual(outer.instance.texts, inner.instance.texts);
    assert.deepEqual(host.finalized, [outer.instance]);
    assert.deepEqual(nested.finalized, [inner.instance]);
  });

  test('a failed raw-value read publishes no partial group and permits a clean retry', () => {
    const { instance, index, meta, before, start, end } = textThenRaw();
    const host = new TextHost();
    host.onResolve = () => { throw new Error('raw read failed'); };
    assert.throws(() => host.$wireHydrationSection(instance, meta, index), /raw read failed/);
    const failedGroup = instance.texts;
    assert.equal(failedGroup, EMPTY_BINDINGS);
    assert.equal(host.finalized.length, 0);
    assert.deepEqual([before.data, start.data, end.data], ['trusted prefix', 'w0', '/w0']);
    host.onResolve = undefined;
    host.$wireHydrationSection(instance, meta, index);
    assert.equal(instance.texts.length, 2);
    assert.equal(instance.texts[0].node, before);
    assert.deepEqual(host.finalized, [instance]);
  });

  test('creates an absent plain text node without resolving its initial value', () => {
    const { instance, index, container } = section([]);
    const host = new TextHost();
    host.$wireHydrationSection(instance, { h: '', tx: [[[0, 0], [['value']]]] }, index);
    assert.equal(instance.texts.length, 1);
    assert.equal(instance.texts[0].node.data, '');
    assert.equal(instance.texts[0].node, container.lastChild);
    assert.deepEqual(instance.nodes, [instance.texts[0].node]);
    assert.equal(host.resolveCalls, 0);
  });

  test('publishes the complete group before a known complex-property write can reenter', () => {
    const { instance, index, container } = section([text('SSR')]);
    Object.assign(container, { nodeType: 1, localName: 'div' });
    instance.attrs = [];
    const value = { label: 'known' };
    let writes = 0;
    Object.defineProperty(container, 'payload', {
      set(received: unknown) {
        writes++;
        assert.equal(received, value);
        assert.equal(instance.texts.length, 1);
        assert.ok(Object.hasOwn(instance.texts, 0));
      },
    });
    const meta: TemplateBlockMeta = {
      h: '', tx: [[[0, 0], [['label']]]],
      a: [['payload', 1, 'payload']], ag: [[0, 0, 1]],
    };
    const host = new TextHost();
    host.value = value;
    host.$wireHydrationSection(instance, meta, index);
    assert.equal(writes, 1);
    assert.deepEqual(host.finalized, [instance]);
  });
});

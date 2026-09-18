// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { strict as assert } from 'node:assert';
import { describe, test } from 'node:test';
import type {
  CompiledCondition,
  CompiledRenderMeta,
  TemplateBlockMeta,
  TemplateMeta,
} from '../template-types.js';
import { dotWalk } from './diff.js';
import {
  hydrateTemplate,
  type HydrationHost,
  type SSRIndex,
} from './hydration.js';
import { MAX_FRAGMENT_DEPTH, MAX_FRAGMENT_VISITS } from './fragment-work.js';
import type { ScopeFrame, TemplateInstance } from './types.js';
import { appendBinding, bindingArray, EMPTY_BINDINGS } from './types.js';

// Only removal is allowed: SSR hydration must never reparent existing content.
class MockNode {
  head: MockNode | null = null;
  after: MockNode | null = null;
  parentNode: MockNode | null = null;
  firstReads = 0;
  nextReads = 0;
  dataReads = 0;
  attributes = new Map<string, string>();

  constructor(
    readonly nodeType: number,
    readonly localName = '',
    private label = '',
  ) {}

  get firstChild(): MockNode | null {
    this.firstReads++;
    return this.head;
  }

  get childNodes(): MockNode[] {
    const children: MockNode[] = [];
    for (let node = this.head; node; node = node.after) children.push(node);
    return children;
  }

  get nextSibling(): MockNode | null {
    this.nextReads++;
    return this.after;
  }

  get data(): string {
    this.dataReads++;
    return this.label;
  }

  set data(value: string) {
    this.label = value;
  }

  getAttribute(name: string): string | null {
    return this.attributes.get(name) ?? null;
  }

  hasAttribute(name: string): boolean {
    return this.attributes.has(name);
  }

  removeChild(node: MockNode): MockNode {
    assert.equal(node.parentNode, this);
    let previous: MockNode | null = null;
    for (let child = this.head; child !== node; child = child.after) {
      assert.ok(child);
      previous = child;
    }
    if (previous) previous.after = node.after;
    else this.head = node.after;
    node.parentNode = null;
    node.after = null;
    return node;
  }
}

function attach(parent: MockNode, children: MockNode[]): MockNode {
  parent.head = children[0] ?? null;
  for (let i = 0; i < children.length; i++) {
    children[i].parentNode = parent;
    children[i].after = children[i + 1] ?? null;
  }
  return parent;
}

function element(name: string, ...children: MockNode[]): MockNode {
  return attach(new MockNode(1, name), children);
}

function root(...children: MockNode[]): MockNode {
  return attach(new MockNode(11), children);
}

function comment(data: string): MockNode {
  return new MockNode(8, '', data);
}

function text(data: string): MockNode {
  return new MockNode(3, '', data);
}

function dom(node: MockNode): Node {
  return node as unknown as Node;
}

interface Wired {
  instance: TemplateInstance;
  meta: TemplateBlockMeta;
  index: SSRIndex;
}

function hydrateFragmentGraph(root: Node, meta: TemplateMeta, host: HydrationHost): TemplateInstance {
  return hydrateTemplate(root, meta, host, true);
}

function harness(
  templates: Array<[TemplateBlockMeta, MockNode]>,
  state: Record<string, unknown> = {},
) {
  const tables = new Map<TemplateBlockMeta, Array<Node | undefined>>();
  for (const [meta, template] of templates) {
    const elements: Array<Node | undefined> = [dom(template)];
    const stack: Array<MockNode | null> = [template.head];
    while (stack.length > 0) {
      let node = stack.pop() ?? null;
      while (node) {
        if (node.nodeType === 1) {
          elements.push(dom(node));
          stack.push(node.after);
          node = node.head;
        } else {
          node = node.after;
        }
      }
    }
    tables.set(meta, elements);
  }
  const wired: Wired[] = [];
  const reads = new Map<TemplateBlockMeta, number>();
  const sourceIds: Array<number | undefined> = [];
  let calls = 0;
  const host: HydrationHost = {
    $templateElements(meta) {
      reads.set(meta, (reads.get(meta) ?? 0) + 1);
      const elements = tables.get(meta);
      assert.ok(elements, 'template must be provided by the host cache');
      return elements;
    },
    $resolveValue(path, scope) {
      const dot = path.indexOf('.');
      const name = dot < 0 ? path : path.slice(0, dot);
      for (let frame = scope; frame; frame = frame.parent) {
        if (frame.name === name) {
          return dot < 0 ? frame.value : dotWalk(frame.value, path, dot + 1);
        }
      }
      return dotWalk(state, path, 0);
    },
    $hasStateRoot(path, scope) {
      const name = path.split('.')[0];
      for (let frame = scope; frame; frame = frame.parent) {
        if (frame.name === name) return frame.known !== false;
      }
      return Object.hasOwn(state, name);
    },
    $createHydrationRender(owner, meta, anchor, end, sourceId) {
      calls++;
      sourceIds.push(sourceId);
      let alias: ScopeFrame | undefined;
      if (meta.length === 4) {
        const known = host.$hasStateRoot(meta[2], owner.scope);
        alias = {
          name: meta[3],
          value: known ? host.$resolveValue(meta[2], owner.scope) : undefined,
          known,
        };
      }
      return {
        owner, blockIndex: meta[0], anchor, end, path: meta[2],
        scope: owner.scope, alias, instance: null,
      };
    },
    $wireHydrationSection(instance, meta, index) {
      assert.equal(instance.texts, bindingArray(0), 'text storage is allocated by the wiring host');
      instance.texts = bindingArray(meta.tx?.length ?? 0);
      wired.push({ instance, meta, index });
    },
  };
  return { host, wired, reads, sourceIds, calls: () => calls };
}

const unevaluated: CompiledCondition = [
  () => { throw new Error('SSR conditions must not be evaluated'); },
  ['visible'],
];

/** Exercise the production section index for the former marker-helper cases. */
function indexed(
  template: MockNode,
  ssr: MockNode,
  metadata: TemplateMeta = { h: '' },
  bodies: Array<[TemplateBlockMeta, MockNode]> = [],
  retainRanges = true,
) {
  const rig = harness([[metadata, template], ...bodies]);
  const instance = hydrateTemplate(dom(ssr), metadata, rig.host, retainRanges);
  return { ...rig, instance, index: rig.wired[0].index };
}

describe('SSR marker indexing through the shared hydrator', () => {
  test('ordinary conditions never acquire a temporary end property during hydration', () => {
    for (const retained of [false, true]) {
      for (const present of [false, true]) {
        const body: TemplateBlockMeta = { h: '' };
        const meta: TemplateMeta = { h: '', b: [body], c: [[unevaluated, 0, [0, 0]]] };
        const start = comment('wc'), end = comment('/wc');
        const tree = root(start, ...(present ? [element('span')] : []), end);
        const rig = harness([[meta, root()], [body, root(element('span'))]]);
        const wire = rig.host.$wireHydrationSection;
        rig.host.$wireHydrationSection = (instance, metadata, index) => {
          for (const condition of instance.conds) {
            assert.equal(Object.hasOwn(condition, 'end'), retained);
          }
          wire(instance, metadata, index);
        };
        const instance = hydrateTemplate(dom(tree), meta, rig.host, retained);
        assert.equal(Object.hasOwn(instance.conds[0], 'end'), retained);
        assert.equal(end.parentNode, retained ? tree : null);
        assert.equal(start.parentNode, retained || !present ? tree : null);
      }
    }
  });

  test('outlet ranges preserve static indexes for empty and multiple opaque route children', () => {
    for (const retained of [false, true]) {
      for (const count of [0, 1, 3]) {
        const start = comment('wo'), end = comment('/wo');
        const before = element('span'), after = element('button');
        const routes = Array.from({ length: count }, () =>
          element('webui-route', element('test-child', comment('wc'), element('div'))));
        const { index } = indexed(
          root(element('main', element('span'), element('outlet'), element('button'))),
          root(element('main', before, start, ...routes, end, after)),
          { h: '' }, [], retained,
        );
        assert.equal(index.elements.length, 5);
        assert.equal(index.elements[2], before);
        assert.equal(index.elements[3], start);
        assert.equal(index.elements[4], after);
        assert.equal(index.raws.length, 0);
        assert.equal(start.data, 'wo');
        assert.equal(end.data, '/wo');
        for (const route of routes) assert.equal(route.firstReads, 0);
      }
    }
  });

  test('outlets inside fragment calls do not consume raw indexes or invocation budgets', () => {
    const body: TemplateBlockMeta = {
      h: '', tx: [[[0, 1], [['html']], 1]],
    };
    const meta: TemplateMeta = { h: '', b: [body], u: [[0, [0, 0]]] };
    const outletStart = comment('wo'), outletEnd = comment('/wo');
    const rawStart = comment('w0'), rawEnd = comment('/w0');
    const nested = element('webui-route', comment('wf'), element('test-nested'));
    const result = indexed(root(),
      root(comment('wf'), outletStart, nested, outletEnd, rawStart, element('i'), rawEnd, comment('/wf')),
      meta, [[body, root(element('outlet'))]]);
    assert.equal(result.calls(), 1);
    assert.equal(result.wired[1].index.elements[1], outletStart);
    assert.deepEqual(result.wired[1].index.raws, [[rawStart, rawEnd]]);
    assert.equal(nested.firstReads, 0);
  });

  test('outlet validation rejects missing, misplaced and unpaired markers before wiring', () => {
    for (const [template, ssr] of [
      [root(element('outlet')), root(element('webui-route'))],
      [root(element('outlet')), root(comment('wo'))],
      [root(element('main', element('outlet'))), root(comment('wo'), comment('/wo'))],
      [root(), root(comment('/wo'))],
    ]) {
      const meta: TemplateMeta = { h: '' };
      const rig = harness([[meta, template]]);
      assert.throws(() => hydrateTemplate(dom(ssr), meta, rig.host), /Invalid template SSR/);
      assert.equal(rig.wired.length, 0);
    }
  });

  test('pairs elements while ignoring whitespace the server dropped', () => {
    const a = element('a'), b = element('b');
    const { index } = indexed(
      root(text(' '), element('a'), text('\n'), element('b'), text(' ')), root(a, b),
    );
    assert.equal(index.elements[1], a);
    assert.equal(index.elements[2], b);
  });

  for (const name of ['style', 'importmap']) {
    test(`skips a compiler-emitted ${name} fallback the template never contained`, () => {
      const resource = element(name === 'style' ? 'style' : 'script');
      resource.attributes.set('data-webui-resource', 'card');
      resource.attributes.set(name === 'style' ? 'data-webui-strategy' : 'type', name);
      const a = element('a'), b = element('b');
      const { index } = indexed(root(element('a'), element('b')), root(resource, a, b), {
        h: '', tx: [[[0, 0], [['value']], 6]],
      });
      assert.equal(index.elements[1], a);
      assert.equal(index.elements[2], b);
      assert.deepEqual(index.elements.slice(1), [a, b]);
      assert.deepEqual(index.comments, []);
    });
  }

  test('indexes authored elements that use data-webui-resource', () => {
    const authored = element('div'), button = element('button');
    authored.attributes.set('data-webui-resource', 'authored');
    const { index } = indexed(root(element('div'), element('button')), root(authored, button), {
      h: '', tx: [[[0, 0], [['value']], 6]],
    });
    assert.equal(index.elements[1], authored);
    assert.equal(index.elements[2], button);
    assert.deepEqual(index.elements.slice(1), [authored, button]);
    assert.deepEqual(index.comments, []);
  });

  test('collects block markers in document order across depths', () => {
    const body: TemplateBlockMeta = { h: '' };
    const a = comment('wc'), b = comment('wc'), c = comment('wc');
    const section = element('section', c, comment('/wc'), element('inner'));
    const { index } = indexed(
      root(element('section', element('inner'))),
      root(a, comment('/wc'), section, b, comment('/wc')),
      { h: '', b: [body], c: [
        [unevaluated, 0, [0, 0]], [unevaluated, 0, [1, 0]], [unevaluated, 0, [0, 1]],
      ] }, [[body, root()]],
    );
    assert.deepEqual(index.conds, [a, c, b]);
  });

  for (const withComment of [false, true]) {
    test(withComment
      ? 'ignores legacy-looking comments inside an indexed raw range'
      : 'collects raw ranges and excludes their elements from static pairing', () => {
      const start = comment('w0'), end = comment('/w0'), span = element('span');
      const contents = withComment
        ? [element('b'), comment('/wh'), element('i')]
        : [element('b'), element('i')];
      const { index } = indexed(root(element('span')), root(start, ...contents, end, span), {
        h: '', tx: [[[0, 0], [['html']], 1]],
      });
      assert.deepEqual(index.raws, [[start, end]]);
      assert.equal(index.elements[1], span);
      assert.equal(start.data, 'w0');
      assert.equal(end.data, '/w0');
    });
  }

  test('does not pair elements inside a structural range', () => {
    const body: TemplateBlockMeta = { h: '<p></p>' };
    const div = element('div');
    const { index } = indexed(root(element('div')),
      root(comment('wc'), element('p'), comment('/wc'), div),
      { h: '', b: [body], c: [[unevaluated, 0, [0, 0]]] }, [[body, root(element('p'))]]);
    assert.equal(index.elements[1], div);
  });

  test('descends into a template-empty element to find its block marker', () => {
    const body: TemplateBlockMeta = { h: '' };
    const start = comment('wr');
    const { index } = indexed(root(element('ul')), root(element('ul', start, comment('/wr'))),
      { h: '', b: [body], r: [['items', 'item', 0, [1, 0]]] }, [[body, root()]]);
    assert.deepEqual(index.repeats, [start]);
    const flat = indexed(root(element('ul')), root(element('ul', text('owned text'))), { h: '' }, [], false);
    assert.deepEqual(flat.index.repeats, []);
  });

  test('stops at a child component that contributes no template children', () => {
    const { index } = indexed(root(element('my-child')),
      root(element('my-child', comment('wc'), comment('/wc'))));
    assert.deepEqual(index.conds, []);
  });

  test('pairs slotted children the parent template owns', () => {
    const span = element('span');
    const { index } = indexed(root(element('my-child', element('span'))), root(element('my-child', span)));
    assert.equal(index.elements[2], span);
  });
});

describe('bounded repeat items replace sibling marker scans', () => {
  test('collects item boundaries and captures the repeat end marker', () => {
    const body: TemplateBlockMeta = { h: '<span></span>' };
    const first = comment('wi'), second = comment('wi'), end = comment('/wr');
    const { wired, instance } = indexed(root(), root(
      comment('wr'), first, element('span'), second, element('span'), end,
    ), { h: '', b: [body], r: [['items', 'item', 0, [0, 0]]] }, [[body, root(element('span'))]]);
    assert.equal(instance.repeats[0].instances.length, 2);
    assert.deepEqual(wired.slice(1).map(entry => entry.index.start), [first, second]);
    assert.equal(instance.repeats[0].end, end);
  });

  test('returns empty items for an empty repeat', () => {
    const body: TemplateBlockMeta = { h: '' }, end = comment('/wr');
    const { instance } = indexed(root(), root(comment('wr'), end),
      { h: '', b: [body], r: [['items', 'item', 0, [0, 0]]] }, [[body, root()]]);
    assert.equal(instance.repeats[0].instances.length, 0);
    assert.equal(instance.repeats[0].end, end);
  });

  test('rejects a missing repeat end instead of adopting a partial item', () => {
    const body: TemplateBlockMeta = { h: '<span></span>' };
    const meta: TemplateMeta = { h: '', b: [body], r: [['items', 'item', 0, [0, 0]]] };
    const rig = harness([[meta, root()], [body, root(element('span'))]]);
    assert.throws(() => hydrateTemplate(
      dom(root(comment('wr'), comment('wi'), element('span'))), meta, rig.host, true,
    ), /missing closing marker/);
    assert.equal(rig.wired.length, 0);
  });

  test('ignores item and end markers from nested repeats', () => {
    const leaf: TemplateBlockMeta = { h: '' };
    const body: TemplateBlockMeta = { h: '', r: [['item.children', 'child', 1, [0, 0]]] };
    const first = comment('wi'), second = comment('wi'), end = comment('/wr');
    const { instance, wired } = indexed(root(), root(
      comment('wr'), first, comment('wr'), comment('wi'), comment('/wr'),
      second, comment('wr'), comment('wi'), comment('/wr'), end,
    ), { h: '', b: [body, leaf], r: [['items', 'item', 0, [0, 0]]] },
    [[body, root()], [leaf, root()]]);
    assert.deepEqual(wired.filter(entry => entry.meta === body).map(entry => entry.index.start), [first, second]);
    assert.equal(instance.repeats[0].end, end);
  });

  test('skips non-comment siblings within an item', () => {
    const body: TemplateBlockMeta = { h: '<span></span>' };
    const item = comment('wi'), end = comment('/wr');
    const { wired, instance } = indexed(root(),
      root(comment('wr'), item, text('hello'), element('span'), text('world'), end),
      { h: '', b: [body], r: [['items', 'item', 0, [0, 0]]] }, [[body, root(element('span'))]]);
    assert.equal(instance.repeats[0].instances.length, 1);
    assert.equal(wired[1].index.start, item);
    assert.equal(instance.repeats[0].end, end);
  });

  test('conditional markers inside an item do not become item boundaries', () => {
    const leaf: TemplateBlockMeta = { h: '' };
    const body: TemplateBlockMeta = { h: '', c: [[unevaluated, 1, [0, 0]]] };
    const item = comment('wi'), end = comment('/wr');
    const { instance, wired } = indexed(root(),
      root(comment('wr'), item, comment('wc'), comment('/wc'), end),
      { h: '', b: [body, leaf], r: [['items', 'item', 0, [0, 0]]] }, [[body, root()], [leaf, root()]]);
    assert.equal(instance.repeats[0].instances.length, 1);
    assert.equal(wired[1].index.start, item);
    assert.equal(instance.repeats[0].end, end);
  });

  for (const whitespace of [false, true]) {
    test(whitespace ? 'skips whitespace text nodes before the item element' : 'finds the element after an item marker', () => {
      const body: TemplateBlockMeta = { h: '<span></span>' }, span = element('span');
      const children = whitespace ? [text(' '), text('\n'), span] : [span];
      const { wired } = indexed(root(),
        root(comment('wr'), comment('wi'), ...children, comment('/wr')),
        { h: '', b: [body], r: [['items', 'item', 0, [0, 0]]] }, [[body, root(element('span'))]]);
      assert.equal(wired[1].index.elements[1], span);
    });
  }

  test('conditional item elements belong only to the conditional section', () => {
    const leaf: TemplateBlockMeta = { h: '<span></span>' }, span = element('span');
    const body: TemplateBlockMeta = { h: '', c: [[unevaluated, 1, [0, 0]]] };
    const { wired } = indexed(root(),
      root(comment('wr'), comment('wi'), comment('wc'), span, comment('/wc'), comment('/wr')),
      { h: '', b: [body, leaf], r: [['items', 'item', 0, [0, 0]]] },
      [[body, root()], [leaf, root(element('span'))]]);
    assert.equal(wired[1].index.elements[1] ?? null, null);
    assert.equal(wired[2].index.elements[1], span);
  });

  for (const ending of ['repeat-end', 'next-item', 'whitespace']) {
    test(`empty items cannot claim the next element across ${ending}`, () => {
      const body: TemplateBlockMeta = { h: '' }, footer = element('footer');
      const content = ending === 'next-item' ? [comment('wi')] :
        ending === 'whitespace' ? [text('  \n  ')] : [];
      const { wired, index } = indexed(root(element('footer')),
        root(comment('wr'), comment('wi'), ...content, comment('/wr'), footer),
        { h: '', b: [body], r: [['items', 'item', 0, [0, 0]]] }, [[body, root()]]);
      assert.equal(wired[1].index.elements[1] ?? null, null);
      assert.equal(index.elements[1], footer);
    });
  }

  test('empty conditions cannot claim the element after their end', () => {
    const body: TemplateBlockMeta = { h: '<span></span>' }, footer = element('footer');
    const { instance, index } = indexed(root(element('footer')),
      root(comment('wc'), comment('/wc'), footer),
      { h: '', b: [body], c: [[unevaluated, 0, [0, 0]]] }, [[body, root(element('span'))]]);
    assert.equal(instance.conds[0].instance, null);
    assert.equal(index.elements[1], footer);
  });
});

describe('direct section indexes replace repeated range skipping', () => {
  test('finds section elements without structural blocks and bounds missing indexes', () => {
    const link = element('link'), div = element('div');
    const { index } = indexed(root(element('link'), element('div')), root(link, div),
      { h: '', tx: [[[0, 0], [['value']], 6]] });
    assert.equal(index.elements[1], link);
    assert.equal(index.elements[2], div);
    assert.equal(index.elements[6] ?? null, null);
    assert.deepEqual(index.comments, []);
    const empty = indexed(root(), root());
    assert.equal(empty.index.elements[1] ?? null, null);
  });

  test('skips conditional block content when counting elements', () => {
    const body: TemplateBlockMeta = { h: '<p></p>' };
    const link = element('link'), div = element('div');
    const { index } = indexed(root(element('link'), element('div')),
      root(link, comment('wc'), element('p'), comment('/wc'), div),
      { h: '', b: [body], c: [[unevaluated, 0, [0, 1]]], tx: [[[0, 0], [['value']], 6]] },
      [[body, root(element('p'))]]);
    assert.equal(index.elements[1], link);
    assert.equal(index.elements[2], div);
  });

  test('retains own text without indexing text or conditional contents', () => {
    const body: TemplateBlockMeta = { h: 'inside' };
    const hello = text('hello'), world = text('world');
    const { instance, index } = indexed(root(),
      root(hello, comment('wc'), text('inside'), comment('/wc'), world),
      { h: '', b: [body], c: [[unevaluated, 0, [0, 0, 1]]],
        tx: [[[0, 0], [['hello']], 2], [[0, 0, 2], [['world']]]] },
      [[body, root(text('inside'))]]);
    assert.deepEqual(instance.nodes.filter(node => node.nodeType === 3), [hello, world]);
    assert.equal(index.elements.length, 1);
    assert.deepEqual(index.comments, []);
    assert.equal('ordinals' in index, false);
  });

  for (const kind of ['repeat', 'nested-condition', 'sequential-conditions', 'empty-condition', 'interleaved']) {
    test(`skips ${kind} content and returns the following static sibling`, () => {
      const leaf: TemplateBlockMeta = { h: '<span></span>' };
      const nested: TemplateBlockMeta = { h: '', c: [[unevaluated, 0, [0, 0]]] };
      const target = element('target');
      const meta: TemplateMeta = {
        h: '', b: [leaf, nested],
      };
      let contents: MockNode[];
      if (kind === 'repeat') {
        meta.r = [['items', 'item', 0, [0, 0]]];
        contents = [comment('wr'), comment('wi'), element('span'), comment('wi'), element('span'), comment('/wr')];
      } else if (kind === 'nested-condition') {
        meta.c = [[unevaluated, 1, [0, 0]]];
        contents = [comment('wc'), comment('wc'), element('span'), comment('/wc'), comment('/wc')];
      } else if (kind === 'sequential-conditions') {
        meta.c = [[unevaluated, 0, [0, 0]], [unevaluated, 0, [0, 0, 1]]];
        contents = [comment('wc'), element('span'), comment('/wc'), comment('wc'), element('span'), comment('/wc')];
      } else if (kind === 'empty-condition') {
        meta.c = [[unevaluated, 0, [0, 0]]];
        contents = [comment('wc'), comment('/wc')];
      } else {
        meta.c = [[unevaluated, 0, [0, 0]]];
        meta.r = [['items', 'item', 0, [0, 0, 1]]];
        contents = [comment('wc'), element('span'), comment('/wc'),
          comment('wr'), comment('wi'), element('span'), comment('/wr')];
      }
      meta.tx = [[[0, 0, (meta.c?.length ?? 0) + (meta.r?.length ?? 0)], [['value']], 6]];
      const { index } = indexed(root(element('target')), root(...contents, target), meta,
        [[leaf, root(element('span'))], [nested, root()]]);
      assert.deepEqual(index.comments, []);
      assert.equal(index.elements[1], target);
    });
  }

  test('rejects unterminated conditional ranges before wiring any section', () => {
    const body: TemplateBlockMeta = { h: '<span></span>' };
    const meta: TemplateMeta = { h: '', b: [body], c: [[unevaluated, 0, [0, 0]]] };
    const rig = harness([[meta, root()], [body, root(element('span'))]]);
    assert.throws(() => hydrateTemplate(
      dom(root(comment('wc'), element('span'))), meta, rig.host,
    ), /missing closing marker/);
    assert.equal(rig.wired.length, 0);
  });
});

describe('ordinary SSR uses the shared section engine', () => {
  for (const count of [0, 1, 2, 16]) {
    for (const kind of ['text', 'comment', 'element']) {
      test(`flat root omits ownership before wiring for ${count} ${kind} nodes`, () => {
        const make = () => kind === 'text' ? text('trusted') : kind === 'comment' ? comment('authored') : element('span');
        const children = Array.from({ length: count }, make);
        const ssr = root(...children), template = root(...Array.from({ length: count }, make));
        const meta: TemplateMeta = { h: '' };
        const rig = harness([[meta, template]]);
        Object.defineProperty(ssr, 'childNodes', {
          get(): never { throw new Error('flat roots must not snapshot or reserve node storage'); },
        });
        const wire = rig.host.$wireHydrationSection;
        rig.host.$wireHydrationSection = (instance, block, index) => {
          assert.equal(instance.nodes, EMPTY_BINDINGS, 'omission precedes all host wiring callbacks');
          wire(instance, block, index);
        };
        const result = hydrateTemplate(dom(ssr), meta, rig.host);
        assert.equal(result.nodes, EMPTY_BINDINGS);
        assert.equal(ssr.head, children[0] ?? null);
        assert.equal(rig.wired.length, 1);
        for (let i = 0; i < count; i++) {
          assert.equal(children[i].parentNode, ssr);
          assert.equal(children[i].after, children[i + 1] ?? null);
          assert.equal(children[i].nextReads, 1, 'the same validation walker still visits each root node once');
        }
      });
    }
  }

  for (const kind of ['condition', 'repeat', 'render']) {
    test(`flat metadata retains explicit members when reused as a ${kind} section`, () => {
      const leaf: TemplateBlockMeta = { h: '<span></span>' };
      const flat = harness([[leaf, root(element('span'))]]);
      assert.equal(hydrateTemplate(dom(root(element('span'))), leaf, flat.host).nodes, EMPTY_BINDINGS);
      const meta: TemplateMeta = { h: '', b: [leaf] };
      const member = element('span');
      const start = comment(kind === 'condition' ? 'wc' : kind === 'repeat' ? 'wr' : 'wf');
      const end = comment(kind === 'condition' ? '/wc' : kind === 'repeat' ? '/wr' : '/wf');
      if (kind === 'condition') meta.c = [[unevaluated, 0, [0, 0]]];
      else if (kind === 'repeat') meta.r = [['items', 'item', 0, [0, 0]]];
      else meta.u = [[0, [0, 0]]];
      const item = comment('wi');
      const ssr = kind === 'repeat' ? root(start, item, member, end) : root(start, member, end);
      const rig = harness([[meta, root()], [leaf, root(element('span'))]], { items: ['one'] });
      const result = hydrateTemplate(dom(ssr), meta, rig.host, kind === 'render');
      const child = kind === 'condition' ? result.conds[0].instance!
        : kind === 'repeat' ? result.repeats[0].instances[0] : result.renders![0].instance!;
      assert.notEqual(result.nodes, EMPTY_BINDINGS);
      assert.deepEqual(child.nodes, [member]);
      assert.equal(member.parentNode, ssr);
      assert.deepEqual(rig.wired.map(entry => entry.instance), [result, child]);
    });
  }

  test('raw-only and explicit graph-mode roots retain their original ownership arrays', () => {
    const start = comment('w0'), content = text('trusted raw'), end = comment('/w0');
    const meta: TemplateMeta = { h: '', tx: [[[0, 0], [['html']], 1]] };
    const ssr = root(start, content, end);
    const rig = harness([[meta, root()]]);
    const result = hydrateTemplate(dom(ssr), meta, rig.host);
    assert.deepEqual(result.nodes, [start, content, end]);
    assert.equal(content.parentNode, ssr);
    const flat: TemplateMeta = { h: '' };
    const graphHost = harness([[flat, root()]]);
    const member = text('graph');
    const graph = hydrateTemplate(dom(root(member)), flat, graphHost.host, true);
    assert.deepEqual(graph.nodes, [member]);
    assert.notEqual(hydrateTemplate(dom(root()), flat, graphHost.host, true).nodes, EMPTY_BINDINGS);
  });

  for (const label of ['wc', 'wr', 'wf', 'w0', '/wc', '/wr', '/wf', '/w0']) {
    test(`flat ownership omission still rejects unexpected ${label} before wiring`, () => {
      const meta: TemplateMeta = { h: '' }, marker = comment(label);
      const ssr = root(marker), rig = harness([[meta, root()]]);
      assert.throws(() => hydrateTemplate(dom(ssr), meta, rig.host), /Invalid template SSR/);
      assert.equal(rig.wired.length, 0);
      assert.equal(marker.data, label);
      assert.equal(marker.parentNode, ssr);
    });
  }

  for (const retainRanges of [false, true]) {
    for (const emptyBody of [false, true]) {
      test(`paired empty conditions prepare but do not wire their child template (${retainRanges ? 'fragment' : 'ordinary'}, ${emptyBody ? 'empty body' : 'hidden body'})`, () => {
        const body: TemplateBlockMeta = { h: emptyBody ? '' : '<strong>hidden</strong>' };
        const meta: TemplateMeta = {
          h: '<footer></footer>', b: [body], c: [[unevaluated, 0, [0, 0]]],
        };
        const start = comment('wc'), end = comment('/wc'), footer = element('footer');
        const ssr = root(start, end, footer);
        const rig = harness([
          [meta, root(element('footer'))],
          [body, emptyBody ? root() : root(element('strong', text('hidden')))],
        ], { visible: true });
        const instance = hydrateTemplate(dom(ssr), meta, rig.host, retainRanges);
        assert.equal(rig.reads.get(body), 1);
        assert.equal(rig.reads.get(meta), 1);
        assert.equal(rig.wired.length, 1);
        assert.equal(instance.conds[0].instance, null);
        assert.equal(instance.conds[0].anchor, start);
        assert.equal(instance.conds[0].end, retainRanges ? end : undefined);
        assert.deepEqual(instance.nodes, retainRanges ? [start, end, footer] : [start, footer]);
        assert.equal(rig.wired[0].index.elements[1], footer);
        for (const node of [start, end, footer]) assert.equal(node.nextReads, 1);
        assert.equal(start.dataReads, retainRanges ? 1 : 2);
        assert.equal(end.dataReads, 1);
        assert.equal(start.data, retainRanges ? '' : 'wc');
        assert.equal(end.parentNode, retainRanges ? ssr : null);
      });
    }

    test(`missing conditional end rejects before wiring (${retainRanges ? 'fragment' : 'ordinary'})`, () => {
      const body: TemplateBlockMeta = { h: '<strong>hidden</strong>' };
      const meta: TemplateMeta = { h: '', b: [body], c: [[unevaluated, 0, [0, 0]]] };
      const rig = harness([[meta, root()], [body, root(element('strong', text('hidden')))]]);
      const start = comment('wc'), ssr = root(start);
      assert.throws(() => hydrateTemplate(dom(ssr), meta, rig.host, retainRanges),
        /missing closing marker for cond/);
      assert.equal(rig.wired.length, 0);
      assert.equal(rig.reads.get(body), 1);
      assert.equal(start.data, 'wc');
    });

    for (const content of ['whitespace', 'authored-comment']) {
      test(`a ${content} before /wc enters its body (${retainRanges ? 'fragment' : 'ordinary'})`, () => {
        const body: TemplateBlockMeta = { h: content === 'whitespace' ? ' ' : '<!--/wc-extra-->' };
        const meta: TemplateMeta = { h: '', b: [body], c: [[unevaluated, 0, [0, 0]]] };
        const node = content === 'whitespace' ? text(' ') : comment('/wc-extra');
        const templateNode = content === 'whitespace' ? text(' ') : comment('/wc-extra');
        const rig = harness([[meta, root()], [body, root(templateNode)]]);
        const instance = hydrateTemplate(dom(root(comment('wc'), node, comment('/wc'))),
          meta, rig.host, retainRanges);
        assert.equal(rig.reads.get(body), 1);
        assert.equal(rig.wired.length, 2);
        assert.deepEqual(instance.conds[0].instance!.nodes, [node]);
        assert.equal(node.nextReads, 1);
      });
    }
  }

  test('absent conditions retain relative section preorder without wiring their bodies', () => {
    const hidden: TemplateBlockMeta = { h: '<strong>hidden</strong>' };
    const body: TemplateBlockMeta = { h: '<span></span>', u: [[2, [1, 0]]] };
    const leaf: TemplateBlockMeta = { h: '' };
    const meta: TemplateMeta = {
      h: '', b: [hidden, body, leaf],
      c: [[unevaluated, 0, [0, 0]], [unevaluated, 1, [0, 0, 1]]],
      u: [[2, [0, 0, 2]]],
    };
    const rig = harness([
      [meta, root()], [hidden, root(element('strong', text('hidden')))],
      [body, root(element('span'))], [leaf, root()],
    ]);
    hydrateTemplate(dom(root(
      comment('wc'), comment('/wc'), comment('wc'),
      element('span', comment('wf'), comment('/wf')), comment('/wc'),
      comment('wf'), comment('/wf'),
    )), meta, rig.host, true);
    assert.equal(rig.reads.get(hidden), 1);
    assert.deepEqual(rig.wired.map(entry => entry.meta), [meta, body, leaf, leaf]);
    assert.deepEqual(rig.wired.map(entry => entry.instance.order), [0, 2, 3, 4]);
    assert.equal(rig.calls(), 2);
  });

  test('flat cleanup does not request iterators for empty bindings or owned nodes', () => {
    const meta: TemplateMeta = { h: '<span></span>' };
    const rig = harness([[meta, root(element('span'))]]);
    const absent = bindingArray(0);
    let nodes: Node[] | undefined;
    let iterations = 0;
    const wire = rig.host.$wireHydrationSection;
    rig.host.$wireHydrationSection = (instance, block, index) => {
      nodes = instance.nodes;
      wire(instance, block, index);
    };
    const originalIterator = Array.prototype[Symbol.iterator];
    try {
      Array.prototype[Symbol.iterator] = function<T>(this: T[]): ArrayIterator<T> {
        if (this === absent || this === nodes) iterations++;
        return originalIterator.call(this);
      };
      hydrateTemplate(dom(root(element('span'))), meta, rig.host);
      assert.equal(iterations, 0);
    } finally {
      Array.prototype[Symbol.iterator] = originalIterator;
    }
  });

  test('flat roots fill exact element tables without allocating owned-node or traversal arrays', () => {
    const meta: TemplateMeta = {
      h: '<span></span><em></em>',
      tx: [[[1, 0], [['label']]], [[2, 0], [['note']]]],
    };
    const span = element('span', text('label')), em = element('em', text('note'));
    const ssr = root(span, em);
    const rig = harness([[meta, root(element('span'), element('em'))]]);
    const pushes = new WeakMap<object, number>();
    let traversalPushes = 0;
    const originalPush = Array.prototype.push;
    try {
      Array.prototype.push = function<T>(this: T[], ...values: T[]): number {
        pushes.set(this, (pushes.get(this) ?? 0) + values.length);
        for (const value of values) {
          if (typeof value === 'object' && value !== null &&
            ('section' in value || 'shape' in value)) traversalPushes++;
        }
        return originalPush.apply(this, values);
      };
      const instance = hydrateTemplate(dom(ssr), meta, rig.host);
      assert.equal(traversalPushes, 0, 'the root frame/section do not enter backing arrays');
      assert.equal(pushes.get(instance.nodes) ?? 0, 0, 'flat roots never append managed nodes');
      assert.equal(pushes.get(rig.wired[0].index.elements) ?? 0, 0, 'the compiled element table is exact-sized');
      // Representation migration: preserve exact DOM order and element identities.
      assert.equal(instance.nodes, EMPTY_BINDINGS);
      assert.equal(rig.wired[0].index.elements.length, 3);
      assert.deepEqual(ssr.childNodes, [span, em]);
      assert.deepEqual(rig.wired[0].index.elements, [ssr, span, em]);
      assert.equal(rig.wired.length, 1);
    } finally {
      Array.prototype.push = originalPush;
    }
  });

  test('pre-sized element storage cannot hide a missing SSR element during validation', () => {
    const meta: TemplateMeta = { h: '<span></span><em></em>' };
    const rig = harness([[meta, root(element('span'), element('em'))]]);
    assert.throws(() => hydrateTemplate(dom(root(element('span'))), meta, rig.host),
      /section nodes do not match compiled metadata/);
    assert.equal(rig.wired.length, 0);
  });

  test('scalar root traversal resumes after nested DOM and structural continuations', () => {
    const body: TemplateBlockMeta = { h: '<strong></strong>' };
    const meta: TemplateMeta = {
      h: '<div><span></span></div><footer></footer>', b: [body],
      c: [[unevaluated, 0, [1, 1]]],
    };
    const span = element('span'), strong = element('strong'), footer = element('footer');
    const div = element('div', span, comment('wc'), strong, comment('/wc'));
    const ssr = root(div, footer);
    const { instance, index, wired } = indexed(
      root(element('div', element('span')), element('footer')), ssr, meta,
      [[body, root(element('strong'))]], false,
    );
    assert.deepEqual(index.elements, [ssr, div, span, footer]);
    assert.deepEqual(instance.nodes, [div, footer]);
    assert.deepEqual(instance.conds[0].instance!.nodes, [strong]);
    assert.equal(wired.length, 2);
    assert.equal(div.nextReads, 1);
    assert.equal(footer.nextReads, 1);
    assert.equal(strong.nextReads, 1);
  });

  test('keeps only repeat starts and absent-condition starts after wiring', () => {
    const body: TemplateBlockMeta = { h: '<span></span>' }, textBody: TemplateBlockMeta = { h: '' };
    const meta: TemplateMeta = {
      h: '', b: [body, textBody],
      c: [[unevaluated, 0, [0, 0]], [unevaluated, 0, [0, 0, 1]]],
      r: [['items', 'item', 1, [0, 0, 2]]],
    };
    const absent = comment('wc'), absentEnd = comment('/wc'), visible = comment('wc'), visibleEnd = comment('/wc');
    const start = comment('wr'), item = comment('wi'), end = comment('/wr');
    const span = element('span'), value = text('item');
    const ssr = root(absent, absentEnd, visible, span, visibleEnd, start, item, value, end);
    const rig = harness([[meta, root()], [body, root(element('span'))], [textBody, root()]], { items: ['item'] });
    const wire = rig.host.$wireHydrationSection;
    rig.host.$wireHydrationSection = (instance, block, index) => {
      assert.equal(end.parentNode, ssr, 'cleanup is deferred until every binding is wired');
      wire(instance, block, index);
    };
    const instance = hydrateTemplate(dom(ssr), meta, rig.host);
    assert.deepEqual(ssr.childNodes, [absent, span, start, value]);
    assert.equal(instance.conds[0].anchor, absent);
    assert.equal(instance.conds[0].instance, null);
    assert.equal(instance.conds[1].anchor, null);
    assert.equal(instance.conds[1].end, undefined);
    assert.equal(instance.repeats[0].start, start);
    assert.equal(instance.repeats[0].end, null);
    assert.deepEqual(instance.nodes, [absent, span, start, value]);
    assert.deepEqual(instance.repeats[0].instances[0].nodes, [value]);
    assert.equal(span.parentNode, ssr);
    assert.equal(value.parentNode, ssr);
    for (const entry of rig.wired) {
      assert.equal(Object.hasOwn(entry.instance, 'range'), false);
      assert.equal(Object.hasOwn(entry.instance, 'renders'), false);
      assert.equal(Object.hasOwn(entry.instance, 'callDepth'), false);
      assert.equal(Object.hasOwn(entry.instance, 'generation'), false);
      assert.equal(entry.instance.attrs, bindingArray(0));
    }
  });

  test('preserves flattened ordinary ownership for nested same-parent blocks and raw nodes', () => {
    const leaf: TemplateBlockMeta = { h: '', tx: [[[0, 0], [['html']], 1]] };
    const body: TemplateBlockMeta = { h: '', c: [[unevaluated, 1, [0, 0]]] };
    const raw = comment('w0'), rawEnd = comment('/w0'), value = element('strong');
    const ssr = root(comment('wc'), comment('wc'), raw, value, rawEnd, comment('/wc'), comment('/wc'));
    const { instance } = indexed(root(), ssr,
      { h: '', b: [body, leaf], c: [[unevaluated, 0, [0, 0]]] }, [[body, root()], [leaf, root()]], false);
    const child = instance.conds[0].instance!;
    const nested = child.conds[0].instance!;
    for (const owner of [instance, child, nested]) assert.deepEqual(owner.nodes, [raw, value, rawEnd]);
    assert.equal(raw.data, 'w0');
    assert.equal(rawEnd.data, '/w0');
    assert.equal(value.parentNode, ssr);
  });

  test('ordinary conditional depth is iterative and does not consume invocation depth', () => {
    const body: TemplateBlockMeta = { h: '', c: [[unevaluated, 0, [0, 0]]] };
    const meta: TemplateMeta = { ...body, b: [body] };
    const count = MAX_FRAGMENT_DEPTH + 64;
    const children: MockNode[] = [];
    for (let i = 0; i < count; i++) children.push(comment('wc'));
    for (let i = 0; i < count; i++) children.push(comment('/wc'));
    const ssr = root(...children);
    const rig = harness([[meta, root()], [body, root()]]);
    rig.host.$visitHydrationInvocation = () => { throw new Error('conditions are not calls'); };
    const instance = hydrateTemplate(dom(ssr), meta, rig.host);
    assert.equal(rig.calls(), 0);
    assert.equal(rig.wired.length, count);
    assert.equal(ssr.childNodes.length, 1);
    assert.deepEqual(instance.nodes, ssr.childNodes);
  });

  test('ordinary element depth needs no fragment depth allowance', () => {
    let template = element('span'), ssr = element('span');
    const count = MAX_FRAGMENT_DEPTH * 4;
    for (let i = 1; i < count; i++) {
      template = element('div', template);
      ssr = element('div', ssr);
    }
    const result = indexed(root(template), root(ssr), { h: '' }, [], false);
    assert.equal(result.index.elements.length, count + 1);
    assert.equal(result.wired.length, 1);
    assert.equal(result.calls(), 0);
  });

  test('ordinary repeat item count does not consume the fragment visit budget', () => {
    const body: TemplateBlockMeta = { h: '' };
    const meta: TemplateMeta = { h: '', b: [body], r: [['items', 'item', 0, [0, 0]]] };
    const count = MAX_FRAGMENT_VISITS + 1;
    const nodes = [comment('wr')];
    for (let i = 0; i < count; i++) nodes.push(comment('wi'));
    nodes.push(comment('/wr'));
    const ssr = attach(root(), nodes);
    const rig = harness([[meta, root()], [body, root()]]);
    rig.host.$visitHydrationInvocation = () => { throw new Error('items are not calls'); };
    const instance = hydrateTemplate(dom(ssr), meta, rig.host);
    assert.equal(instance.repeats[0].instances.length, count);
    assert.equal(rig.calls(), 0);
    assert.deepEqual(ssr.childNodes, [nodes[0]]);
    assert.equal(ssr.firstReads, 1);
    for (const node of nodes) assert.equal(node.nextReads, 1);
  });
});

describe('bindingArray', () => {
  test('shares immutable absent groups and allocates only present groups', () => {
    const absent = bindingArray<string>(0);
    assert.equal(absent, bindingArray<number>(0));
    assert.equal(Object.isFrozen(absent), true);
    assert.throws(() => absent.push('unexpected'), TypeError);
    const first = bindingArray<string>(1);
    const second = bindingArray<string>(1);
    first.push('first');
    assert.deepEqual(second, []);
    assert.notEqual(first, second);
    assert.notEqual(first, absent);
  });

  test('replaces shared storage once and preserves later order and duplicates', () => {
    const empty = bindingArray<string>(0);
    const first = appendBinding(empty, 'first');
    const separate = appendBinding(empty, 'separate');
    assert.deepEqual(empty, []);
    assert.deepEqual(first, ['first']);
    assert.deepEqual(separate, ['separate']);
    assert.notEqual(first, separate);
    assert.equal(appendBinding(first, 'first'), first);
    assert.equal(appendBinding(first, 'last'), first);
    assert.deepEqual(first, ['first', 'first', 'last']);
  });

  test('reuses mutable empty groups rather than treating them as the sentinel', () => {
    const owned: string[] = [];
    assert.equal(appendBinding(owned, 'first'), owned);
    owned.length = 0;
    assert.equal(appendBinding(owned, 'later'), owned);
    assert.deepEqual(owned, ['later']);
  });
});

describe('hydrateTemplate fragment ranges', () => {
  test('isolates reentrant hosts while sharing metadata shapes', () => {
    const body: TemplateBlockMeta = { h: '<span></span>' };
    const meta: TemplateMeta = {
      h: '', u: [[0, [0, 0], 'source', 'row']], b: [body],
    };
    const outerValue = { label: 'Outer' };
    const innerValue = { label: 'Inner' };
    const templates: Array<[TemplateBlockMeta, MockNode]> = [
      [meta, root()], [body, root(element('span'))],
    ];
    const outer = harness(templates, { source: outerValue });
    const inner = harness(templates, { source: innerValue });
    const outerStart = comment('wf');
    const innerStart = comment('wf');
    const outerRoot = root(outerStart, element('span', text('Outer')), comment('/wf'));
    const innerRoot = root(innerStart, element('span', text('Inner')), comment('/wf'));
    const nested: TemplateInstance[] = [];
    let started = false;
    const wireOuter = outer.host.$wireHydrationSection;
    outer.host.$wireHydrationSection = (instance, block, index) => {
      wireOuter(instance, block, index);
      if (!started) {
        started = true;
        nested.push(hydrateFragmentGraph(dom(innerRoot), meta, inner.host));
      }
    };

    const result = hydrateFragmentGraph(dom(outerRoot), meta, outer.host);
    assert.equal(nested.length, 1);
    const innerResult = nested[0];
    assert.equal(result.container, dom(outerRoot));
    assert.equal(innerResult.container, dom(innerRoot));
    assert.equal(result.renders?.[0].alias?.value, outerValue);
    assert.equal(innerResult.renders?.[0].alias?.value, innerValue);
    assert.notEqual(result.renders, innerResult.renders);
    assert.notEqual(result.nodes, innerResult.nodes);
    assert.notEqual(result.renders?.[0].alias, innerResult.renders?.[0].alias);
    assert.equal(outer.calls(), 1);
    assert.equal(inner.calls(), 1);
    assert.equal(outer.wired.length, 2);
    assert.equal(inner.wired.length, 2);
    assert.equal(inner.reads.size, 0, 'the second host reuses metadata shapes, not owner state');
    assert.equal(outerStart.data, '');
    assert.equal(innerStart.data, '');
  });

  for (const retainRanges of [false, true]) {
    test(`resumes DOM, condition, and repeat cursors after host reentry (${retainRanges ? 'fragment' : 'ordinary'})`, () => {
      const item: TemplateBlockMeta = { h: '<span></span>', tx: [[[1, 0], [['item.html']], 1]] };
      const body: TemplateBlockMeta = { h: '<p></p>', r: [['items', 'item', 1, [0, 0]]] };
      const meta: TemplateMeta = {
        h: '<div></div><footer></footer>', b: [body, item], c: [[unevaluated, 0, [1, 0]]],
      };
      const condition = comment('wc'), conditionEnd = comment('/wc');
      const repeat = comment('wr'), repeatEnd = comment('/wr');
      const itemStart = comment('wi'), raw = comment('w0'), rawEnd = comment('/w0');
      const value = text('trusted'), span = element('span', raw, value, rawEnd);
      const paragraph = element('p'), footer = element('footer');
      const div = element('div', condition, repeat, itemStart, span, repeatEnd, paragraph, conditionEnd);
      const ssr = root(div, footer);
      const outer = harness([
        [meta, root(element('div'), element('footer'))],
        [body, root(element('p'))], [item, root(element('span'))],
      ], { items: [{ html: 'client value must not replace SSR' }] });
      const innerBody: TemplateBlockMeta = { h: '<strong></strong>' };
      const innerMeta: TemplateMeta = retainRanges
        ? { h: '<em></em>', b: [innerBody], c: [[unevaluated, 0, [0, 0]]] }
        : { h: '<em></em>', b: [innerBody], u: [[0, [0, 0], 'input', 'arg']] };
      const input = { label: 'inner' };
      const inner = harness([
        [innerMeta, root(element('em'))], [innerBody, root(element('strong'))],
      ], { input });
      const nested: TemplateInstance[] = [];
      const reenter = () => {
        assert.equal(outer.wired.length, 0, 'outer ranges are not wired during its walk');
        assert.equal(condition.data, 'wc');
        assert.equal(repeatEnd.data, '/wr');
        const start = comment(retainRanges ? 'wc' : 'wf');
        const end = comment(retainRanges ? '/wc' : '/wf');
        const strong = element('strong'), emphasis = element('em');
        const innerRoot = root(start, strong, end, emphasis);
        const instance = hydrateTemplate(dom(innerRoot), innerMeta, inner.host, !retainRanges);
        nested.push(instance);
        assert.deepEqual(instance.nodes, retainRanges ? [strong, emphasis] : [start, end, emphasis]);
        const child = retainRanges ? instance.conds[0].instance! : instance.renders![0].instance!;
        assert.deepEqual(child.nodes, [strong]);
        if (!retainRanges) assert.equal(child.scope?.value, input);
      };
      const resolve = outer.host.$resolveValue;
      outer.host.$resolveValue = (path, scope) => {
        reenter();
        return resolve(path, scope);
      };
      outer.host.$hydratedRepeat = reenter;

      const instance = hydrateTemplate(dom(ssr), meta, outer.host, retainRanges);
      const conditional = instance.conds[0].instance!;
      const repeated = conditional.repeats[0].instances[0];
      assert.equal(nested.length, 2, 'both repeat entry and completion can reenter');
      assert.notEqual(nested[0].nodes, nested[1].nodes);
      assert.deepEqual(instance.nodes, [div, footer]);
      assert.deepEqual(conditional.nodes, retainRanges ? [repeat, repeatEnd, paragraph] : [repeat, span, paragraph]);
      assert.deepEqual(repeated.nodes, retainRanges ? [itemStart, span] : [span]);
      assert.deepEqual(outer.wired.map(entry => entry.meta), [meta, body, item]);
      assert.deepEqual(outer.wired[0].index.elements, [ssr, div, footer]);
      assert.deepEqual(outer.wired[1].index.elements, [div, paragraph]);
      assert.deepEqual(outer.wired[2].index.elements, [div, span]);
      if (retainRanges) assert.deepEqual(outer.wired.map(entry => entry.instance.order), [0, 1, 2]);
      for (const node of [div, condition, repeat, itemStart, span, raw, value, rawEnd, repeatEnd, paragraph, conditionEnd, footer]) {
        assert.equal(node.nextReads, 1, 'reentry never restarts a suspended cursor');
      }
      assert.equal(raw.data, 'w0');
      assert.equal(rawEnd.data, '/w0');
      assert.equal(value.data, 'trusted');
    });
  }

  test('accepts later fragment metadata without capturing the first owner or block table', () => {
    const firstBody: TemplateBlockMeta = { h: '<span></span>' };
    const firstMeta: TemplateMeta = { h: '', u: [[0, [0, 0]]], b: [firstBody] };
    const first = harness([
      [firstMeta, root()], [firstBody, root(element('span'))],
    ]);
    const firstSpan = element('span', text('First'));
    const firstResult = hydrateFragmentGraph(
      dom(root(comment('wf'), firstSpan, comment('/wf'))), firstMeta, first.host,
    );

    const lateBody: TemplateBlockMeta = { h: '<strong></strong><em></em>' };
    const lateMeta: TemplateMeta = {
      h: '', u: [[0, [0, 0], 'model', 'item']], b: [lateBody],
    };
    const lateValue = { label: 'Late' };
    const late = harness([
      [lateMeta, root()], [lateBody, root(element('strong'), element('em'))],
    ], { model: lateValue });
    const strong = element('strong', text('Late'));
    const emphasis = element('em', text('Different block'));
    const lateStart = comment('wf');
    const lateEnd = comment('/wf');
    const lateResult = hydrateFragmentGraph(
      dom(root(lateStart, strong, emphasis, lateEnd)), lateMeta, late.host,
    );

    assert.deepEqual(firstResult.renders?.[0].instance?.nodes, [dom(firstSpan)]);
    assert.equal(firstResult.renders?.[0].alias, undefined);
    assert.deepEqual(lateResult.renders?.[0].instance?.nodes, [dom(strong), dom(emphasis)]);
    assert.equal(lateResult.renders?.[0].alias?.value, lateValue);
    assert.equal(lateResult.renders?.[0].alias?.name, 'item');
    assert.equal(first.calls(), 1);
    assert.equal(late.calls(), 1);
    assert.equal(first.wired.length, 2);
    assert.equal(late.wired.length, 2);
    assert.equal(lateStart.data, '');
    assert.equal(lateEnd.data, '');
  });

  test('allocates its weak shape cache only on first graph hydration', async () => {
    const PreviousWeakMap = globalThis.WeakMap;
    let allocations = 0;
    try {
      globalThis.WeakMap = new Proxy(PreviousWeakMap, {
        construct(target, argumentsList, newTarget) {
          allocations++;
          return Reflect.construct(target, argumentsList, newTarget);
        },
      });
      const url = new URL('./hydration.js', import.meta.url);
      url.searchParams.set('lazy-shape-cache-test', '1');
      const fresh: typeof import('./hydration.js') = await import(url.href);
      assert.equal(allocations, 0, 'loading the module does not allocate the cache');

      const meta: TemplateMeta = { h: '' };
      const rig = harness([[meta, root()]]);
      fresh.hydrateTemplate(dom(root()), meta, rig.host, true);
      assert.equal(allocations, 1);
      fresh.hydrateTemplate(dom(root()), meta, rig.host, true);
      assert.equal(allocations, 1, 'later hydration reuses the weak cache');
      assert.equal(rig.reads.get(meta), 1, 'the compiled shape remains shared');
    } finally {
      globalThis.WeakMap = PreviousWeakMap;
    }
  });

  test('keeps shapes populated by reentry during the first template lookup', async () => {
    const url = new URL('./hydration.js', import.meta.url);
    url.searchParams.set('reentrant-shape-cache-test', '1');
    const fresh: typeof import('./hydration.js') = await import(url.href);
    const meta: TemplateMeta = { h: '<span></span>' };
    const innerMeta: TemplateMeta = { h: '<strong></strong>' };
    const outer = harness([[meta, root(element('span'))]]);
    const inner = harness([[innerMeta, root(element('strong'))]]);
    const lookup = outer.host.$templateElements;
    outer.host.$templateElements = block => {
      fresh.hydrateTemplate(dom(root(element('strong'))), innerMeta, inner.host);
      return lookup(block);
    };
    const span = element('span');
    const ssr = root(span);
    const result = fresh.hydrateTemplate(dom(ssr), meta, outer.host);
    // Representation migration: the host DOM, not a redundant root list, owns span.
    assert.equal(result.nodes, EMPTY_BINDINGS);
    assert.deepEqual(ssr.childNodes, [span]);
    fresh.hydrateTemplate(dom(root(element('strong'))), innerMeta, inner.host);
    fresh.hydrateTemplate(dom(root(element('span'))), meta, outer.host);
    assert.equal(inner.reads.get(innerMeta), 1, 'outer cache initialization preserves the nested shape');
    assert.equal(outer.reads.get(meta), 1);
    assert.equal(inner.wired.length, 2);
    assert.equal(outer.wired.length, 2);
  });

  test('a failed first SSR walk retains only shared preparation for an independent retry', async () => {
    const url = new URL('./hydration.js', import.meta.url);
    url.searchParams.set('failed-first-cursor-test', '1');
    const fresh: typeof import('./hydration.js') = await import(url.href);
    const body: TemplateBlockMeta = { h: '<strong></strong>' };
    const meta: TemplateMeta = { h: '', b: [body], c: [[unevaluated, 0, [0, 0]]] };
    const rig = harness([[meta, root()], [body, root(element('strong'))]]);
    const invalidStart = comment('wc');
    assert.throws(() => fresh.hydrateTemplate(dom(root(invalidStart)), meta, rig.host),
      /missing closing marker for cond/);
    assert.equal(rig.wired.length, 0);
    assert.equal(invalidStart.data, 'wc');

    const strong = element('strong');
    const ssr = root(comment('wc'), strong, comment('/wc'));
    const result = fresh.hydrateTemplate(dom(ssr), meta, rig.host);
    assert.deepEqual(result.nodes, [strong]);
    assert.equal(result.conds[0].instance!.parent, result);
    assert.deepEqual(rig.wired.map(entry => entry.meta), [meta, body]);
    assert.equal(rig.reads.get(meta), 1);
    assert.equal(rig.reads.get(body), 1);
    assert.equal(invalidStart.data, 'wc', 'retry never visits or cleans the failed operation');
  });

  test('empty invocation groups share storage without sharing live binding arrays', () => {
    const empty: TemplateBlockMeta = { h: '' };
    const bound: TemplateBlockMeta = { h: '', tx: [[[0, 0], [['value']]]] };
    const meta: TemplateMeta = {
      h: '', b: [empty, bound],
      u: [[0, [0, 0]], [0, [0, 0, 1]], [1, [0, 0, 2]], [1, [0, 0, 3]]],
    };
    const ssr = root(
      comment('wf'), comment('/wf'), comment('wf'), comment('/wf'),
      comment('wf'), text('first'), comment('/wf'),
      comment('wf'), text('second'), comment('/wf'),
    );
    const rig = harness([[meta, root()], [empty, root()], [bound, root()]]);
    hydrateFragmentGraph(dom(ssr), meta, rig.host);
    const absent = bindingArray(0);
    for (const entry of rig.wired.slice(1, 3)) {
      const { instance, index } = entry;
      for (const bindings of [
        instance.texts, instance.attrs, instance.conds, instance.repeats, instance.renders,
        index.conds, index.repeats, index.renders, index.raws, index.comments,
      ]) assert.equal(bindings, absent);
    }
    assert.notEqual(rig.wired[3].instance.texts, rig.wired[4].instance.texts);
    assert.notEqual(rig.wired[3].instance.texts, absent);
    assert.equal(rig.wired[3].instance.attrs, absent);
  });

  test('indexes only referenced authored comments by section-local parent', () => {
    const body: TemplateBlockMeta = {
      h: '<span><!--note--><strong></strong></span>',
      tx: [[[0, 0], [['beforeSpan']], 6], [[1, 0], ['value', ['beforeValue']], 7]],
    };
    const meta: TemplateMeta = {
      h: '<footer></footer>', b: [body], u: [[0, [0, 0]]],
      tx: [[[0, 0, 1], [['beforeFooter']], 6]],
    };
    const value = text('value'), note = comment('note'), strong = element('strong');
    const span = element('span', value, note, strong), footer = element('footer');
    const ssr = root(comment('wf'), span, comment('/wf'), footer);
    const rig = harness([
      [meta, root(element('footer'))],
      [body, root(element('span', comment('note'), element('strong')))],
    ]);
    hydrateFragmentGraph(dom(ssr), meta, rig.host);

    const owner = rig.wired[0].index;
    const called = rig.wired[1].index;
    assert.deepEqual(owner.elements, [ssr, footer]);
    assert.deepEqual(called.elements, [ssr, span, strong]);
    assert.deepEqual(owner.comments, []);
    assert.deepEqual(called.comments[1], [note]);
    assert.equal(called.comments[2], undefined, 'empty strong needs no comment bucket');
    assert.equal(called.comments[0], undefined, 'no bucket without a comment successor');
    assert.equal('ordinals' in owner, false);
    assert.equal('ordinals' in called, false);
  });

  test('does not index children when text slots have no following static sibling', () => {
    const body: TemplateBlockMeta = {
      h: '<span></span><aside><strong>static</strong></aside>',
      tx: [[[1, 0], [['value']]]],
    };
    const meta: TemplateMeta = { h: '', b: [body], u: [[0, [0, 0]]] };
    const span = element('span', text('dynamic'));
    const strong = element('strong', text('static')), aside = element('aside', strong);
    const ssr = root(comment('wf'), span, aside, comment('/wf'));
    const rig = harness([
      [meta, root()],
      [body, root(element('span'), element('aside', element('strong', text('static'))))],
    ]);
    hydrateFragmentGraph(dom(ssr), meta, rig.host);
    assert.deepEqual(rig.wired[1].index.elements, [ssr, span, aside, strong]);
    for (const entry of rig.wired) assert.deepEqual(entry.index.comments, []);
  });

  test('comment successors count authored blanks but exclude structural, raw, and callee comments', () => {
    const body: TemplateBlockMeta = { h: '<!--callee-->' };
    const meta: TemplateMeta = {
      h: '<!--first--><!----><span><!--nested--></span>', b: [body],
      u: [[0, [0, 1]]],
      tx: [[[0, 1, 1], [['html']], 1], [[0, 1, 2], [['before']], 15], [[1, 0], [['inner']], 7]],
    };
    const first = comment('first'), blank = comment(''), nested = comment('nested');
    const callee = comment('callee'), injected = comment('raw');
    const start = comment('wf'), end = comment('/wf'), raw = comment('w0'), rawEnd = comment('/w0');
    const span = element('span', text('inner'), nested);
    const ssr = root(first, start, callee, end, raw, injected, rawEnd, text('before'), blank, span);
    const rig = harness([
      [meta, root(comment('first'), comment(''), element('span', comment('nested')))],
      [body, root(comment('callee'))],
    ]);
    hydrateFragmentGraph(dom(ssr), meta, rig.host);
    assert.deepEqual(rig.wired[0].index.comments, [[first, blank], [nested]]);
    assert.deepEqual(rig.wired[1].index.comments, []);
    assert.equal(blank.data, '');
    assert.equal(start.data, '');
    assert.equal(end.data, '');
    assert.equal(raw.data, 'w0');
    assert.equal(injected.data, 'raw');
  });

  test('streamed markers select exact source IDs without changing structural ownership', () => {
    const body: TemplateBlockMeta = { h: '' };
    const meta: TemplateMeta = {
      h: '', b: [body], u: [[0, [0, 0], 'source', 'node'], [0, [0, 0, 1], 'source', 'node']],
    };
    const first = comment('wf:12'), second = comment('wf:7');
    const ssr = root(first, text('OLD'), comment('/wf'), second, text('older'), comment('/wf'));
    const rig = harness([[meta, root()], [body, root()]], { source: 'NEW' });
    const instance = hydrateFragmentGraph(dom(ssr), meta, rig.host);
    assert.deepEqual(rig.sourceIds, [12, 7]);
    assert.equal(instance.renders!.length, 2);
    assert.equal(first.data, '');
    assert.equal(second.data, '');
  });

  test('rejects source markers that no invocation can consume', () => {
    const body: TemplateBlockMeta = { h: '' };
    const parameterless: TemplateMeta = { h: '', b: [body], u: [[0, [0, 0]]] };
    const captured: TemplateMeta = { h: '', b: [body], u: [[0, [0, 0], 'source', 'node']] };
    const cases: Array<[TemplateMeta, string]> = [
      // A parameterless call has no alias to rebind, so an identifier on it can
      // only mean the client and the response disagree about the template.
      [parameterless, 'wf:3'],
      [captured, 'wf:'],
      [captured, 'wf:1x'],
      [captured, 'wf:-1'],
      [captured, `wf:${0x1_0000_0000}`],
    ];
    for (const [meta, label] of cases) {
      const rig = harness([[meta, root()], [body, root()]]);
      const ssr = root(comment(label), comment('/wf'));
      assert.throws(
        () => hydrateFragmentGraph(dom(ssr), meta, rig.host),
        /\[WebUI\]/,
        `${label} must not hydrate`,
      );
      assert.equal(rig.wired.length, 0);
      assert.deepEqual(rig.sourceIds, []);
    }
  });

  test('indexes sibling and nested calls independently without duplicating descendant nodes', () => {
    const leaf: TemplateBlockMeta = { h: 'leaf' };
    const body: TemplateBlockMeta = { h: '<span></span><b></b>', u: [[1, [1, 0]]] };
    const meta: TemplateMeta = {
      h: '<footer></footer>', u: [[0, [0, 0]], [1, [0, 0, 1]]], b: [body, leaf],
      tx: [[[0, 0, 2], [['beforeFooter']], 6]],
    };
    const f1 = comment('wf'), e1 = comment('/wf');
    const inner = comment('wf'), innerEnd = comment('/wf'), innerText = text('nested');
    const span = element('span', inner, innerText, innerEnd), bold = element('b');
    const f2 = comment('wf'), e2 = comment('/wf'), siblingText = text('sibling');
    const footer = element('footer');
    const ssr = root(f1, span, bold, e1, f2, siblingText, e2, footer);
    const rig = harness([
      [meta, root(element('footer'))],
      [body, root(element('span'), element('b'))],
      [leaf, root(text('leaf'))],
    ]);
    const instance = hydrateFragmentGraph(dom(ssr), meta, rig.host);
    assert.deepEqual(rig.sourceIds, [undefined, undefined, undefined]);
    const outer = instance.renders![0].instance!;
    const nested = outer.renders![0].instance!;
    const sibling = instance.renders![1].instance!;
    assert.deepEqual(instance.nodes, [f1, e1, f2, e2, footer]);
    assert.deepEqual(outer.nodes, [span, bold]);
    assert.deepEqual(nested.nodes, [innerText]);
    assert.deepEqual(sibling.nodes, [siblingText]);
    assert.equal(outer.parent, instance);
    assert.equal(nested.parent, outer);
    assert.equal(sibling.parent, instance);
    assert.deepEqual([instance.callDepth, outer.callDepth, nested.callDepth, sibling.callDepth], [0, 1, 2, 1]);
    assert.equal(instance.renders![0].end, e1);
    assert.equal(outer.renders![0].end, innerEnd);
    assert.equal(rig.reads.get(leaf), 1, 'one cached template lookup per body');
    assert.equal(rig.wired.length, 4);
    const outerIndex = rig.wired[1].index;
    assert.deepEqual(outerIndex.elements, [ssr, span, bold]);
    assert.equal(outerIndex.comments[1], undefined);
    assert.equal(outerIndex.start, f1);
    assert.equal(outerIndex.end, e1);
    assert.equal(rig.wired[0].index.elements[1], footer);
    assert.deepEqual(rig.wired[0].index.comments, []);
    for (const { instance: entry } of rig.wired) {
      assert.equal(entry.range, true);
      assert.equal(entry.alive, true);
      assert.equal(entry.container, entry === nested ? span : ssr);
    }
    for (const marker of [f1, e1, inner, innerEnd, f2, e2]) assert.equal(marker.data, '');
    assert.equal(span.parentNode, ssr);
    assert.equal(innerText.parentNode, span);
  });

  test('trusts empty/text/structural conditions and retains empty and multi-text repeat ranges', () => {
    const empty: TemplateBlockMeta = { h: '' };
    const textBody: TemplateBlockMeta = { h: '', tx: [[[0, 0], [['item'], 'b']]] };
    const structural: TemplateBlockMeta = { h: '', u: [[0, [0, 0]]] };
    const meta: TemplateMeta = {
      h: '', b: [empty, textBody, structural],
      c: [[unevaluated, 2, [0, 0]], [unevaluated, 1, [0, 0, 1]], [unevaluated, 2, [0, 0, 2]]],
      r: [['items', 'item', 1, [0, 0, 3], 'id']],
    };
    const c1 = comment('wc'), ce1 = comment('/wc');
    const c2 = comment('wc'), ce2 = comment('/wc'), value = text('text only');
    const c3 = comment('wc'), ce3 = comment('/wc'), f = comment('wf'), fe = comment('/wf');
    const r = comment('wr'), re = comment('/wr');
    const i1 = comment('wi'), i2 = comment('wi'), a = text('a'), b = text('b');
    const ssr = root(c1, ce1, c2, value, ce2, c3, f, fe, ce3, r, i1, a, b, i2, re);
    const rig = harness([[meta, root()], [empty, root()], [textBody, root()], [structural, root()]], {
      items: [{ id: 'a' }, { id: 'b' }],
    });
    const instance = hydrateFragmentGraph(dom(ssr), meta, rig.host);
    assert.equal(instance.conds[0].instance, null);
    assert.deepEqual(instance.conds[1].instance!.nodes, [value]);
    const conditional = instance.conds[2].instance!;
    assert.deepEqual(conditional.nodes, [f, fe]);
    assert.deepEqual(conditional.renders![0].instance!.nodes, []);
    assert.equal(conditional.callDepth, 0);
    assert.equal(conditional.renders![0].instance!.callDepth, 1);
    assert.equal(instance.conds[2].anchor, c3);
    assert.equal(instance.conds[2].end, ce3);
    const repeat = instance.repeats[0];
    assert.equal(repeat.start, r);
    assert.equal(repeat.end, re);
    assert.deepEqual(repeat.instances[0].nodes, [i1, a, b]);
    assert.deepEqual(repeat.instances[1].nodes, [i2]);
    assert.equal(repeat.instances[0].callDepth, 0);
    assert.deepEqual(repeat.keyState!.keys, ['a', 'b']);
    assert.equal(repeat.instances[0].scope!.known, true);
    assert.equal(repeat.instances[1].scope!.known, true);
    assert.deepEqual(instance.nodes, [c1, ce1, c2, ce2, c3, ce3, r, re]);
    const itemIndexes = rig.wired.filter(entry => entry.instance.parent === instance && entry.instance.scope);
    assert.equal(itemIndexes[0].index.elements.length, 1);
    assert.deepEqual(itemIndexes[0].instance.nodes.filter(node => node.nodeType === 3), [a, b]);
    assert.deepEqual(itemIndexes[0].index.comments, []);
    assert.equal(itemIndexes[0].index.start, i1);
    assert.equal(itemIndexes[0].index.end, i2);
    assert.equal(itemIndexes[1].index.end, re);
  });

  test('isolates call aliases from caller loops and preserves callee-local structural scopes', () => {
    const item: TemplateBlockMeta = {
      h: '', u: [[1, [0, 0], 'item.input', 'arg'], [3, [0, 0, 1]]],
    };
    const callee: TemplateBlockMeta = {
      h: '', c: [[unevaluated, 2, [0, 0]]], r: [['arg.rows', 'row', 3, [0, 0, 1]]],
    };
    const conditional: TemplateBlockMeta = { h: '' }, empty: TemplateBlockMeta = { h: '' };
    const meta: TemplateMeta = { h: '', r: [['items', 'item', 0, [0, 0]]], b: [item, callee, conditional, empty] };
    const ssr = root(
      comment('wr'), comment('wi'), comment('wf'),
      comment('wc'), text('present'), comment('/wc'),
      comment('wr'), comment('wi'), comment('/wr'), comment('/wf'),
      comment('wf'), comment('/wf'), comment('/wr'),
    );
    const input = { rows: ['row'] };
    const rig = harness([
      [meta, root()], [item, root()], [callee, root()], [conditional, root()], [empty, root()],
    ], { items: [{ input }] });
    const instance = hydrateFragmentGraph(dom(ssr), meta, rig.host);
    const caller = instance.repeats[0].instances[0];
    const render = caller.renders![0];
    const child = render.instance!;
    assert.equal(caller.nodes.length, 5, 'item owns its wi marker and both invocation pairs');
    assert.equal(render.scope, caller.scope);
    assert.equal(render.alias!.value, input);
    assert.equal(render.alias!.parent, undefined);
    assert.equal(child.scope, render.alias);
    assert.equal(child.conds[0].instance!.scope, render.alias);
    assert.equal(child.repeats[0].instances[0].scope!.parent, render.alias);
    assert.equal(child.repeats[0].instances[0].scope!.value, 'row');
    assert.equal(caller.renders![1].instance!.scope, undefined);
  });

  test('does not evaluate missing bootstrap scopes or seed mismatched repeat keys', () => {
    const item: TemplateBlockMeta = { h: '', u: [[1, [0, 0], 'item.value', 'value']] };
    const leaf: TemplateBlockMeta = { h: '' };
    const meta: TemplateMeta = { h: '', r: [['items', 'item', 0, [0, 0], 'id']], b: [item, leaf] };
    const makeSSR = () => root(
      comment('wr'), comment('wi'), comment('wf'), text('server'), comment('/wf'),
      comment('wi'), comment('wf'), comment('/wf'), comment('/wr'),
    );
    const templates: Array<[TemplateBlockMeta, MockNode]> = [[meta, root()], [item, root()], [leaf, root()]];
    const unknown = harness(templates);
    unknown.host.$resolveValue = () => { throw new Error('unknown bootstrap must not be evaluated'); };
    const preserved = hydrateFragmentGraph(dom(makeSSR()), meta, unknown.host);
    for (const entry of preserved.repeats[0].instances) {
      assert.equal(entry.scope!.known, false);
      assert.equal(entry.renders![0].alias!.known, false);
    }
    assert.deepEqual(preserved.repeats[0].keyState!.keys, []);
    const known = harness(templates, { items: [{ id: 'only', value: 0 }] });
    const mismatched = hydrateFragmentGraph(dom(makeSSR()), meta, known.host).repeats[0];
    assert.equal(mismatched.instances[0].scope!.known, true);
    assert.equal(mismatched.instances[1].scope!.known, false);
    assert.deepEqual(mismatched.keyState!.keys, []);
    assert.equal(mismatched.instances[0].renders![0].alias!.value, 0);
  });

  test('skips raw and component internals but descends into dynamic-only projected content', () => {
    const leaf: TemplateBlockMeta = { h: '' };
    const meta: TemplateMeta = {
      h: '<private-child></private-child><projected-child></projected-child>',
      tx: [[[0, 0], [['html']], 1], [[0, 0, 1], [['beforeChild']], 6]],
      u: [[0, [2, 0]]], b: [leaf],
    };
    const raw = comment('w0'), rawEnd = comment('/w0');
    const nestedRaw = comment('w0'), nestedRawEnd = comment('/w0'), rawCall = comment('wf');
    const injected = element('section', comment('wf'));
    const opaque = element('private-child', comment('wc'), comment('wf'));
    const f = comment('wf'), fe = comment('/wf'), content = text('projected');
    const projected = element('projected-child', f, content, fe);
    const ssr = root(raw, nestedRaw, rawCall, injected, nestedRawEnd, rawEnd, opaque, projected);
    const rig = harness([
      [meta, root(element('private-child'), element('projected-child'))], [leaf, root()],
    ]);
    const instance = hydrateFragmentGraph(dom(ssr), meta, rig.host);
    assert.equal(opaque.firstReads, 0);
    assert.equal(injected.firstReads, 0);
    assert.equal(opaque.head!.nextReads, 0);
    assert.equal(injected.head!.nextReads, 0);
    assert.equal(projected.firstReads, 1);
    for (const node of [raw, nestedRaw, rawCall, injected, nestedRawEnd, rawEnd, opaque, projected, f, content, fe]) {
      assert.equal(node.nextReads, 1, 'raw and owned siblings are never rescanned');
    }
    assert.equal(instance.renders![0].instance!.container, projected);
    assert.deepEqual(instance.renders![0].instance!.nodes, [content]);
    assert.deepEqual(rig.wired[0].index.elements, [ssr, opaque, projected]);
    assert.deepEqual(rig.wired[0].index.raws, [[raw, rawEnd]]);
    assert.deepEqual(rig.wired[0].index.comments, []);
    assert.equal(raw.data, 'w0');
    assert.equal(rawEnd.data, '/w0');
    assert.equal(nestedRaw.data, 'w0');
    assert.equal(rawCall.data, 'wf', 'raw content is opaque to structural hydration');
  });

  test('pairs static projected children and excludes server-only style resources', () => {
    const leaf: TemplateBlockMeta = { h: '' };
    const meta: TemplateMeta = {
      h: '<projected-child><span></span></projected-child>', u: [[0, [2, 0]]], b: [leaf],
      tx: [[[0, 0], [['beforeChild']], 6]],
    };
    const style = element('style', text('css'));
    style.attributes.set('data-webui-resource', 'child');
    style.attributes.set('data-webui-strategy', 'style');
    const span = element('span', comment('wf'), comment('/wf'));
    const child = element('projected-child', span);
    const ssr = root(style, child);
    const rig = harness([[meta, root(element('projected-child', element('span')))], [leaf, root()]]);
    hydrateFragmentGraph(dom(ssr), meta, rig.host);
    assert.equal(style.firstReads, 0);
    assert.deepEqual(rig.wired[0].index.elements, [ssr, child, span]);
    assert.deepEqual(rig.wired[0].index.comments, []);
  });

  test('visits each sibling exactly once through deep ordinary structural frames', () => {
    const depth = MAX_FRAGMENT_DEPTH + 20;
    const blocks: TemplateBlockMeta[] = [];
    for (let i = 0; i < depth; i++) {
      blocks.push({ h: '', c: [[unevaluated, i + 1, [0, 0]]] });
    }
    blocks.push({ h: '', u: [[depth + 1, [0, 0]]] }, { h: '' });
    const meta: TemplateMeta = { ...blocks[0], b: blocks };
    const nodes: MockNode[] = [];
    for (let i = 0; i < depth; i++) nodes.push(comment('wc'));
    nodes.push(comment('wf'), comment('/wf'));
    for (let i = 0; i < depth; i++) nodes.push(comment('/wc'));
    const ssr = attach(root(), nodes);
    const templates: Array<[TemplateBlockMeta, MockNode]> = [[meta, root()]];
    for (const block of blocks) templates.push([block, root()]);
    const rig = harness(templates);
    hydrateFragmentGraph(dom(ssr), meta, rig.host);
    assert.equal(ssr.firstReads, 1);
    for (const node of nodes) {
      assert.equal(node.nextReads, 1);
      assert.equal(node.dataReads, 1);
      assert.equal(node.firstReads, 0);
    }
    assert.equal(rig.calls(), 1);
    assert.equal(rig.wired[rig.wired.length - 1].instance.callDepth, 1);
  });

  test('enforces active invocation depth without counting sibling calls', () => {
    const leaf: TemplateBlockMeta = { h: '' };
    const body: TemplateBlockMeta = { h: '', c: [[unevaluated, 1, [0, 0]]] };
    const recursive: TemplateBlockMeta = { h: '', u: [[0, [0, 0]]] };
    const meta: TemplateMeta = { h: '', u: [[0, [0, 0]], [2, [0, 0, 1]]], b: [body, recursive, leaf] };
    const makeSSR = (depth: number) => {
      const nodes: MockNode[] = [];
      for (let i = 0; i < depth; i++) nodes.push(comment('wf'), comment('wc'));
      for (let i = 0; i < depth; i++) nodes.push(comment('/wc'), comment('/wf'));
      nodes.push(comment('wf'), comment('/wf'));
      return attach(root(), nodes);
    };
    const templates: Array<[TemplateBlockMeta, MockNode]> = [
      [meta, root()], [body, root()], [recursive, root()], [leaf, root()],
    ];
    const accepted = harness(templates);
    hydrateFragmentGraph(dom(makeSSR(MAX_FRAGMENT_DEPTH)), meta, accepted.host);
    assert.equal(accepted.calls(), MAX_FRAGMENT_DEPTH + 1);
    const rejected = harness(templates);
    assert.throws(
      () => hydrateFragmentGraph(dom(makeSSR(MAX_FRAGMENT_DEPTH + 1)), meta, rejected.host),
      error => error instanceof Error && error.message.includes('[WebUI]')
        && error.message.includes(`depth limit (${MAX_FRAGMENT_DEPTH})`),
    );
    assert.equal(rejected.wired.length, 0);
    assert.equal(rejected.calls(), MAX_FRAGMENT_DEPTH);
  });

  test('shares the visit budget across all sibling invocations', () => {
    const leaf: TemplateBlockMeta = { h: '' };
    const calls: CompiledRenderMeta[] = [];
    const nodes: MockNode[] = [];
    for (let i = 0; i <= MAX_FRAGMENT_VISITS; i++) {
      calls.push([0, [0, 0, i]]);
      nodes.push(comment('wf'), comment('/wf'));
    }
    const meta: TemplateMeta = { h: '', u: calls, b: [leaf] };
    const ssr = attach(root(), nodes);
    const rig = harness([[meta, root()], [leaf, root()]]);
    assert.throws(
      () => hydrateFragmentGraph(dom(ssr), meta, rig.host),
      error => error instanceof Error && error.message.includes('[WebUI]')
        && error.message.includes(`visit limit (${MAX_FRAGMENT_VISITS})`),
    );
    assert.equal(rig.calls(), MAX_FRAGMENT_VISITS);
    assert.equal(rig.wired.length, 0);
    assert.equal(nodes[nodes.length - 1].nextReads, 0, 'stop before traversing beyond the limit');
    assert.equal(nodes[0].data, 'wf', 'validation failure does not erase markers');
  });

  test('rejects incomplete or misnested markers before wiring any section', () => {
    const leaf: TemplateBlockMeta = { h: '' };
    const meta: TemplateMeta = { h: '', u: [[0, [0, 0]]], b: [leaf] };
    const invalidTrees = [
      root(comment('wf')),
      root(comment('wf'), comment('/wc')),
      root(comment('/wf')),
      root(),
      root(comment('wf'), comment('/wf'), comment('wf')),
    ];
    for (const ssr of invalidTrees) {
      const rig = harness([[meta, root()], [leaf, root()]]);
      assert.throws(() => hydrateFragmentGraph(dom(ssr), meta, rig.host), /\[WebUI\] Invalid template SSR/);
      assert.equal(rig.wired.length, 0);
    }
  });

  test('requires marker pairs and metadata slots to share their DOM parent', () => {
    const leaf: TemplateBlockMeta = { h: '' };
    const meta: TemplateMeta = { h: '<div></div>', u: [[0, [1, 0]]], b: [leaf] };
    for (const ssr of [
      root(element('div', comment('wf')), comment('/wf')),
      root(comment('wf'), comment('/wf'), element('div')),
    ]) {
      const rig = harness([[meta, root(element('div'))], [leaf, root()]]);
      assert.throws(() => hydrateFragmentGraph(dom(ssr), meta, rig.host), /\[WebUI\] Invalid template SSR/);
      assert.equal(rig.wired.length, 0);
    }
  });

  test('rejects unpaired raw/repeat markers and repeat content without item boundaries', () => {
    const body: TemplateBlockMeta = { h: '' };
    const rawMeta: TemplateMeta = { h: '', tx: [[[0, 0], [['html']], 1]] };
    const repeatMeta: TemplateMeta = { h: '', r: [['items', 'item', 0, [0, 0]]], b: [body] };
    const cases: Array<[TemplateMeta, MockNode]> = [
      [rawMeta, root(comment('w0'), element('div', comment('/w0')))],
      [rawMeta, root(comment('w0'), comment('/w1'))],
      [rawMeta, root(comment('/w0'))],
      [repeatMeta, root(comment('wr'), comment('wi'))],
      [repeatMeta, root(comment('wr'), text('not an item'), comment('/wr'))],
      [repeatMeta, root(comment('wi'), comment('/wr'))],
    ];
    for (const [meta, ssr] of cases) {
      const rig = harness([[meta, root()], [body, root()]]);
      assert.throws(() => hydrateFragmentGraph(dom(ssr), meta, rig.host), /\[WebUI\] Invalid template SSR/);
      assert.equal(rig.wired.length, 0);
    }
  });
});

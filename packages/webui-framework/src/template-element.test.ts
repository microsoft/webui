// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { strict as assert } from 'node:assert';
import { describe, test } from 'node:test';
import type { TemplateMeta } from './template.js';
import type { CompiledRenderMeta } from './template-types.js';
import type { RenderBinding, TemplateInstance, ScopeFrame, TextBinding } from './element/types.js';
import { EMPTY_BINDINGS } from './element/types.js';
import { createFragmentWork, type FragmentWork } from './element/fragment-work.js';
import { clearFragmentInputSources, forgetFragmentInput, fragmentInput, retainedFragmentInput, registerFragmentSources, registerFragmentSourceRefs } from './fragment-inputs.js';

/**
 * `TemplateElement extends HTMLElement` at module scope, so `HTMLElement`
 * must exist globally before the module is evaluated — same mocking pattern
 * as `static-host.test.ts` and `streaming.test.ts`.
 */
Object.defineProperty(globalThis, 'HTMLElement', {
  value: class HTMLElement {
    tagName = '';
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

const { TemplateElement } = await import('./template-element.js');
const { registerTemplateData } = await import('./template.js');

type IndexedBindings = Pick<TemplateInstance, 'texts' | 'attrs' | 'conds' | 'repeats' | 'renders'>;

interface FragmentCore {
  $meta: TemplateMeta;
  $root: TemplateInstance | null;
  $ready: boolean;
  $templateState: Record<string, unknown>;
  $fragmentWork: FragmentWork;
  $guardUnknownState?: boolean;
  $fragmentHydrating?: boolean;
  $fragmentKnownRoots?: Set<string>;
  $hasUnknownScopes?: boolean;
  $pathIndex?: Map<string, IndexedBindings>;
  $wildcardBindings?: IndexedBindings | null;
  $pendingFlush?: boolean;
  $makeRender(owner: TemplateInstance, meta: CompiledRenderMeta, start: Comment, end: Comment, hydrate: boolean, sourceId?: number): RenderBinding;
  $refreshRenderScope(binding: RenderBinding, unknown: boolean): void;
  $resolveValue(path: string, scope?: ScopeFrame): unknown;
  $buildPathIndex(): void;
  $updateInstance(instance: TemplateInstance): void;
  $update(path: string): void;
  $flushUpdates(): void;
  $removeInstance(instance: TemplateInstance): void;
  $destroy(): void;
}

function fragmentInstance(scope?: ScopeFrame, parent?: TemplateInstance): TemplateInstance {
  return {
    scope, parent, container: null, nodes: [], texts: [], attrs: [], conds: [], repeats: [],
    renders: [], alive: true, range: true, callDepth: parent?.callDepth ?? 0,
  };
}

function fragmentCore(state: Record<string, unknown>): FragmentCore {
  const core = new TemplateElement() as unknown as FragmentCore;
  core.$templateState = state;
  core.$meta = { h: '', tr: Object.keys(state) };
  core.$fragmentWork = createFragmentWork();
  core.$root = fragmentInstance();
  core.$ready = true;
  return core;
}

type PathCore = Omit<FragmentCore, '$fragmentWork'> & { $fragmentWork?: FragmentWork };

function ordinaryCore(state: Record<string, unknown>): PathCore {
  const core: PathCore = fragmentCore(state);
  core.$fragmentWork = undefined;
  core.$root = {
    container: null, nodes: [], texts: [], attrs: [], conds: [], repeats: [],
  };
  return core;
}

function fragmentCall(core: FragmentCore, owner: TemplateInstance, path?: string, sourceId?: number): RenderBinding {
  const metadata: CompiledRenderMeta = path ? [0, [0, 0], path, 'node'] : [0, [0, 0]];
  const binding = core.$makeRender(owner, metadata, {} as Comment, {} as Comment, true, sourceId);
  binding.instance = fragmentInstance(binding.alias, owner);
  binding.instance.callDepth = (owner.callDepth ?? 0) + 1;
  owner.renders!.push(binding);
  return binding;
}

describe('path-index binding storage', () => {
  test('shares only empty groups while preserving duplicates, wildcard buckets and host isolation', () => {
    const populated: TextBinding[][] = [];
    for (const fragment of [false, true]) {
      const core = fragment ? fragmentCore({ value: 'before' }) : ordinaryCore({ value: 'before' });
      const root = core.$root!;
      const direct: TextBinding = { node: {} as CharacterData, path: 'value', owner: root };
      const repeated: TextBinding = {
        node: {} as CharacterData, parts: [['value'], '/', ['value']], owner: root,
      };
      const wildcard: TextBinding = { node: {} as CharacterData, path: 'outside', owner: root };
      root.texts.push(direct, repeated, wildcard);
      core.$buildPathIndex();
      const entry = core.$pathIndex?.get('value');
      assert.ok(entry);
      assert.deepEqual(entry.texts, [direct, repeated, repeated]);
      assert.notEqual(entry.texts, EMPTY_BINDINGS);
      assert.equal(entry.attrs, EMPTY_BINDINGS);
      assert.equal(entry.conds, EMPTY_BINDINGS);
      assert.equal(entry.repeats, EMPTY_BINDINGS);
      assert.deepEqual(Object.keys(entry), ['texts', 'attrs', 'conds', 'repeats']);
      assert.deepEqual(core.$wildcardBindings?.texts, [wildcard]);
      assert.equal(core.$wildcardBindings?.attrs, EMPTY_BINDINGS);
      assert.equal(core.$pathIndex?.has('*'), false);
      populated.push(entry.texts);
    }
    assert.notEqual(populated[0], populated[1]);
    assert.deepEqual(EMPTY_BINDINGS, []);
  });

  test('populates each binding category independently in ordinary and fragment indexes', () => {
    for (const fragment of [false, true]) {
      const core = fragment ? fragmentCore({ value: [] }) : ordinaryCore({ value: [] });
      const root = core.$root!;
      root.texts.push({ node: {} as CharacterData, path: 'value', owner: root });
      root.attrs.push({ element: {} as Element, name: 'data-value', kind: 0, path: 'value' });
      root.conds.push({
        condition: [() => true, ['value']], blockIndex: 0,
        anchor: null, owner: root, instance: null,
      });
      root.repeats.push({
        markerId: 0, collection: 'value', itemVar: 'item', blockIndex: 0,
        container: null, start: null, end: null, owner: root, instances: [],
      });
      core.$buildPathIndex();
      const entry = core.$pathIndex?.get('value');
      assert.ok(entry);
      for (const key of ['texts', 'attrs', 'conds', 'repeats'] as const) {
        assert.deepEqual(entry[key], root[key]);
        assert.notEqual(entry[key], root[key]);
        assert.notEqual(entry[key], EMPTY_BINDINGS);
      }
      assert.equal(new Set([entry.texts, entry.attrs, entry.conds, entry.repeats]).size, 4);
      assert.deepEqual(Object.keys(entry), ['texts', 'attrs', 'conds', 'repeats']);
    }
  });

  test('stores call dependencies only in their own group, without duplicating leaf bindings', () => {
    const core = fragmentCore({ tree: { label: 'old' }, title: 'before' });
    const call = fragmentCall(core, core.$root!, 'tree');
    const text: TextBinding = {
      node: {} as CharacterData, path: 'title', owner: call.instance!,
    };
    call.instance!.texts.push(text);
    core.$buildPathIndex();
    assert.deepEqual(core.$pathIndex?.get('tree')?.renders, [call]);
    assert.equal(core.$pathIndex?.get('tree')?.texts, EMPTY_BINDINGS);
    assert.deepEqual(core.$pathIndex?.get('title')?.texts, [text]);
    assert.equal(core.$pathIndex?.get('title')?.renders, undefined);
    for (const entry of core.$pathIndex!.values()) {
      assert.equal(Object.hasOwn(entry, 'work'), false);
    }
  });

  test('drains reentrant writes and wildcard bindings through the same flush for both host types', () => {
    for (const fragment of [false, true]) {
      const core = fragment ? fragmentCore({ value: 'first', other: 'before' }) :
        ordinaryCore({ value: 'first', other: 'before' });
      const root = core.$root!;
      const writes: string[] = [];
      const first = {
        get data() { return ''; },
        set data(value: string) {
          writes.push(value);
          core.$templateState.other = 'second';
          core.$update('other');
        },
      } as CharacterData;
      const other = {
        get data() { return ''; },
        set data(value: string) { writes.push(value); },
      } as CharacterData;
      const wildcard = {
        get data() { return ''; },
        set data(value: string) { writes.push(value); },
      } as CharacterData;
      Object.defineProperty(core, 'outside', { value: 'wildcard' });
      root.texts.push(
        { node: first, path: 'value', owner: root },
        { node: other, path: 'other', owner: root },
        { node: wildcard, path: 'outside', owner: root },
      );
      core.$update('value');
      core.$flushUpdates();
      assert.deepEqual(writes, fragment ?
        ['wildcard', 'first', 'wildcard', 'second'] :
        ['first', 'wildcard', 'second', 'wildcard']);
    }
  });

  test('propagates a failed binding and releases flush ownership before a later write', async () => {
    for (const fragment of [false, true]) {
      const core = fragment ? fragmentCore({ value: 'rejected' }) : ordinaryCore({ value: 'rejected' });
      const root = core.$root!;
      let data = 'before';
      let reject = true;
      const text = {
        get data() { return data; },
        set data(value: string) {
          if (reject) throw new Error('rejected binding value');
          data = value;
        },
      } as CharacterData;
      root.texts.push({ node: text, path: 'value', owner: root });
      core.$update('value');
      assert.throws(() => core.$flushUpdates(), /rejected binding value/);
      assert.equal(core.$pendingFlush, false);
      if (core.$fragmentWork) {
        assert.equal(core.$fragmentWork.active, false);
        assert.equal(core.$fragmentWork.stack.length, 0);
      }
      reject = false;
      core.$templateState.value = 'accepted';
      core.$update('value');
      assert.equal(core.$pendingFlush, true);
      await Promise.resolve();
      assert.equal(data, 'accepted');
      assert.equal(core.$pendingFlush, false);
    }
  });

  test('keeps targeted and wildcard updates while preserving unavailable SSR scopes', () => {
    const core = ordinaryCore({ value: 'before' });
    const root = core.$root!;
    const known = { data: 'before' } as CharacterData;
    const unknown = { data: 'trusted SSR' } as CharacterData;
    const wildcard = { data: 'wild before' } as CharacterData;
    const scope: ScopeFrame = { name: 'item', value: undefined, known: false };
    core.$hasUnknownScopes = true;
    Object.defineProperty(core, 'outside', { value: 'wild after' });
    root.texts.push(
      { node: known, path: 'value' },
      { node: unknown, path: 'value', scope },
      { node: wildcard, path: 'outside' },
    );
    core.$buildPathIndex();
    core.$templateState.value = 'after';
    core.$update('value');
    core.$flushUpdates();
    assert.equal(known.data, 'after');
    assert.equal(wildcard.data, 'wild after');
    assert.equal(unknown.data, 'trusted SSR');
    scope.known = true;
    core.$update('value');
    core.$flushUpdates();
    assert.equal(unknown.data, 'after');
  });
});

describe('fragment instance runtime', () => {
  test('streamed captures survive owner-only and broad reconciliation but rebind on explicit input writes', () => {
    clearFragmentInputSources(true);
    const old = { label: 'OLD' };
    const current = { label: 'NEW' };
    const core = fragmentCore({ tree: current, title: 'resumed owner' });
    registerFragmentSources([[0, 0, old]]);
    const call = fragmentCall(core, core.$root!, 'tree', 0);
    const text = { data: 'OLD/resumed owner' } as CharacterData;
    call.instance!.texts.push({
      node: text, scope: call.alias, owner: call.instance!,
      parts: [['node.label'], '/', ['title']],
    });
    assert.equal(call.alias?.value, old);
    assert.equal(core.$templateState.tree, current);
    core.$templateState.title = 'client owner';
    core.$update('title');
    core.$flushUpdates();
    core.$updateInstance(core.$root!);
    assert.equal(text.data, 'OLD/client owner');
    assert.equal(core.$resolveValue('node.label', call.alias), 'OLD');
    core.$update('tree');
    core.$flushUpdates();
    assert.equal(text.data, 'NEW/client owner');
    assert.equal(call.alias?.value, current);
  });

  test('a pathless lifecycle refresh preserves captured inputs that an explicit write rebinds', () => {
    clearFragmentInputSources(true);
    const old = { label: 'OLD' };
    const current = { label: 'NEW' };
    const core = fragmentCore({ tree: current });
    registerFragmentSources([[0, 0, old]]);
    const call = fragmentCall(core, core.$root!, 'tree', 0);
    assert.equal(call.alias?.value, old);
    // A synchronous reparent (remove and re-append in one task) never reaches
    // delayed teardown, so it reconnects straight into a pathless refresh.
    // Nothing was written, so nothing may rebind to the owner's newer state.
    (core as unknown as { $update(path?: string): void }).$update();
    core.$flushUpdates();
    core.$updateInstance(core.$root!);
    assert.equal(call.alias?.value, old, 'a reconnect is not an input write');
    assert.equal(call.captureVersion, 0, 'and it does not consume an input revision');
    core.$update('tree');
    core.$flushUpdates();
    core.$updateInstance(core.$root!);
    assert.equal(call.alias?.value, current, 'an explicit write still rebinds');
  });

  test('owner-loop captured calls rebind when the ultimate collection input is written', () => {
    clearFragmentInputSources(true);
    const current = { child: { label: 'NEW' } };
    const old = { label: 'OLD' };
    const core = fragmentCore({ tree: [current] });
    core.$root!.scope = { name: 'item', value: current, sourceRoot: 'tree' };
    registerFragmentSources([[0, 0, old]]);
    const call = fragmentCall(core, core.$root!, 'item.child', 0);
    core.$updateInstance(core.$root!);
    assert.equal(call.alias?.value, old);
    core.$update('tree');
    core.$updateInstance(core.$root!);
    assert.equal(call.alias?.value, current.child);
  });

  test('captured loop-member inputs and nested aliases rebind on either fallback dependency', () => {
    for (const root of ['items', 'item']) {
      clearFragmentInputSources(true);
      const current = { child: 'NEW' };
      const core = fragmentCore({ items: [{}], item: { fallback: current } });
      core.$root!.scope = { name: 'item', value: {}, known: true, sourceRoot: 'items' };
      registerFragmentSources([[0, 0, { child: 'OLD' }], [1, 0, 'OLD']]);
      const call = fragmentCall(core, core.$root!, 'item.fallback', 0);
      const nested = fragmentCall(core, call.instance!, 'node.child', 1);
      core.$updateInstance(core.$root!);
      assert.equal(nested.alias?.value, 'OLD');
      core.$update(root);
      core.$flushUpdates();
      core.$updateInstance(core.$root!);
      assert.equal(call.alias?.value, current);
      assert.equal(nested.alias?.value, 'NEW');
    }
  });

  test('captured aliases survive teardown and reconnect without the response registry', () => {
    clearFragmentInputSources(true);
    const old = { label: 'OLD' };
    const current = { label: 'NEW' };
    const core = fragmentCore({ tree: current });
    registerFragmentSources([[0, 0, old]]);
    registerFragmentSourceRefs(core as unknown as Element, [0]);
    clearFragmentInputSources();
    const call = fragmentCall(core, core.$root!, 'tree', 0);
    const start = call.anchor, end = call.end;
    clearFragmentInputSources();
    core.$destroy();
    const owner = fragmentInstance();
    const restored = core.$makeRender(owner, [0, [0, 0], 'tree', 'node'], start, end, true);
    assert.equal(restored.alias?.value, old);
    core.$update('tree');
    const rebound = core.$makeRender(owner, [0, [0, 0], 'tree', 'node'], start, end, true);
    assert.equal(rebound.alias?.value, current);
  });

  test('teardown transfers reservations to retained aliases before reconnect rebind or removal', () => {
    for (const action of ['rebind', 'remove']) {
      clearFragmentInputSources(true);
      const old = { label: 'OLD' };
      const current = { label: 'NEW' };
      const core = fragmentCore({ tree: current });
      const host = core as unknown as Element;
      registerFragmentSources([[0, 0, old], [1, 0, 'dormant']]);
      registerFragmentSourceRefs(host, [0, 1]);
      clearFragmentInputSources();
      const first = fragmentCall(core, core.$root!, 'tree', 0);
      const second = fragmentCall(core, core.$root!, 'tree', 0);
      core.$destroy();
      assert.throws(() => fragmentInput(host, 0), /unknown source 0 after the response closed/);
      assert.equal(fragmentInput(host, 1), 'dormant');
      const owner = fragmentInstance();
      core.$root = owner;
      core.$ready = true;
      for (const previous of [first, second]) {
        assert.equal(retainedFragmentInput(previous.anchor)?.scope.value, old);
        const restored = core.$makeRender(
          owner, [0, [0, 0], 'tree', 'node'], previous.anchor, previous.end, true,
          previous === first ? 0 : undefined,
        );
        restored.instance = fragmentInstance(restored.alias, owner);
        owner.renders!.push(restored);
        assert.equal(restored.alias?.value, old);
        assert.equal(restored.sourceId, undefined);
      }
      if (action === 'rebind') {
        core.$update('tree');
        core.$flushUpdates();
        for (const restored of owner.renders!) assert.equal(restored.alias?.value, current);
      } else {
        core.$removeInstance(owner);
      }
      for (const previous of [first, second]) {
        assert.equal(retainedFragmentInput(previous.anchor), undefined);
      }
      assert.throws(() => fragmentInput(host, 0), /unknown source 0 after the response closed/);
      assert.equal(fragmentInput(host, 1), 'dormant');
    }
  });

  test('rejects skewed identifiers whether or not the response table is still open', () => {
    clearFragmentInputSources(true);
    const core = fragmentCore({ tree: { label: 'NEW' } });
    registerFragmentSources([[0, 0, { label: 'OLD' }]]);
    // An open response owns the whole identifier space, so a miss is real skew
    // between the delivered markers and the delivered table.
    assert.throws(() => fragmentCall(core, core.$root!, 'tree', 7), /unknown source 7/);
    clearFragmentInputSources();
    // Closing the table is not permission to answer a captured marker out of
    // the owner's current state: that would render NEW under an invocation that
    // still claims OLD and hide exactly the skew rejected above.
    assert.throws(
      () => fragmentCall(core, core.$root!, 'tree', 7),
      /unknown source 7 after the response closed/,
    );
    // An invocation that surrendered its capture identity — an explicit input
    // write, or removal — is the one case that legitimately falls back.
    const anchor = {} as Comment;
    forgetFragmentInput(anchor);
    const late = core.$makeRender(
      core.$root!, [0, [0, 0], 'tree', 'node'], anchor, {} as Comment, true, 7,
    );
    assert.equal(late.alias?.known, true);
    assert.deepEqual(late.alias?.value, { label: 'NEW' });
  });

  test('host-scoped inputs outlive the response table for every later frame', () => {
    clearFragmentInputSources(true);
    const old = { child: { label: 'OLD' } };
    const core = fragmentCore({ tree: { child: { label: 'NEW' } } });
    registerFragmentSources([[0, 0, old], [1, 1, 0, 'child']]);
    registerFragmentSourceRefs(core as unknown as Element, [0]);
    // A second span completion adds to the same host rather than replacing it.
    registerFragmentSourceRefs(core as unknown as Element, [1]);
    clearFragmentInputSources();
    // Nested and dormant frames hydrate on their own schedule; retention is
    // bounded by the host element, never by an end-of-walk release step.
    assert.equal(fragmentCall(core, core.$root!, 'tree', 0).alias?.value, old);
    assert.equal(fragmentCall(core, core.$root!, 'tree', 1).alias?.value, old.child);
    assert.equal(fragmentCall(core, core.$root!, 'tree', 0).alias?.value, old);
    // An unrelated host shares none of it, and hears about it rather than
    // silently rendering its own newer state under a captured marker.
    const other = fragmentCore({ tree: { label: 'NEW' } });
    assert.throws(
      () => fragmentCall(other, other.$root!, 'tree', 0),
      /unknown source 0 after the response closed/,
    );
  });

  test('resolves caller input while isolating explicit and parameterless callees', () => {
    const core = fragmentCore({ item: 'owner item', node: { absent: 'owner fallback' } });
    const caller = fragmentInstance({ name: 'item', value: { value: { label: 'local' } }, known: true });
    const call = fragmentCall(core, caller, 'item.value');
    assert.equal(call.scope, caller.scope);
    assert.equal(call.alias?.parent, undefined);
    assert.equal(core.$resolveValue('node.label', call.alias), 'local');
    assert.equal(core.$resolveValue('node.absent', call.alias), undefined);
    assert.equal(core.$resolveValue('item', call.alias), 'owner item');
    const parameterless = fragmentCall(core, caller);
    assert.equal(parameterless.alias, undefined);
    assert.equal(core.$resolveValue('item', parameterless.alias), 'owner item');
  });

  test('missing loop members resolve against owner state without exposing shadowed scopes', () => {
    const core = fragmentCore({ items: [{}], item: { fallback: 'GLOBAL' } });
    const caller = fragmentInstance({
      name: 'item', value: {}, known: true, sourceRoot: 'items',
      parent: { name: 'item', value: { fallback: 'OUTER' }, known: true },
    });
    const call = fragmentCall(core, caller, 'item.fallback');
    assert.equal(call.alias?.value, 'GLOBAL');
    for (const value of [null, false, 0, '']) {
      caller.scope!.value = { fallback: value };
      assert.equal(core.$resolveValue('item.fallback', caller.scope), value);
    }
  });

  test('a missing loop member keeps its input unknown when owner fallback state was not sent', () => {
    const core = fragmentCore({ items: [{}] });
    const caller = fragmentInstance({ name: 'item', value: {}, known: true, sourceRoot: 'items' });
    const call = fragmentCall(core, caller, 'item.fallback');
    assert.equal(call.alias?.known, false);
    core.$templateState.item = { fallback: 'arrived' };
    core.$refreshRenderScope(call, true);
    assert.equal(call.alias?.value, 'arrived');
    core.$templateState.item = {};
    assert.throws(() => core.$refreshRenderScope(call, true), /every path segment/);
  });

  test('accepts all supported scalar and container scope values', () => {
    for (const value of [null, false, 0, '', 'leaf', [], {}, [1]]) {
      const core = fragmentCore({ value });
      const binding = fragmentCall(core, core.$root!, 'value');
      assert.equal(binding.alias?.known, true);
      assert.equal(binding.alias?.value, value);
    }
    const core = fragmentCore({ value: 'leaf' });
    assert.equal(fragmentCall(core, core.$root!, 'value.length').alias?.value, 4);
    for (const [value, expected] of [['é', 2], ['😀', 4]] as const) {
      core.$templateState.value = value;
      assert.equal(fragmentCall(core, core.$root!, 'value.length').alias?.value, expected);
      assert.throws(() => fragmentCall(core, core.$root!, 'value.length.more'), /every path segment/);
    }
  });

  test('unknown bootstrap input preserves its frame but known missing input throws', () => {
    const core = fragmentCore({});
    const binding = fragmentCall(core, core.$root!, 'missing.child');
    assert.equal(binding.alias?.known, false);
    assert.throws(() => core.$refreshRenderScope(binding, false), /Missing fragment scope/);
    core.$templateState.missing = {};
    assert.throws(() => core.$refreshRenderScope(binding, true), /every path segment/);
    core.$templateState.missing = { child: null };
    core.$refreshRenderScope(binding, true);
    assert.equal(binding.alias?.known, true);
    assert.equal(binding.alias?.value, null);
  });

  test('callee loops shadow the alias without exposing caller loops', () => {
    const core = fragmentCore({ tree: { label: 'alias' } });
    const call = fragmentCall(core, core.$root!, 'tree');
    const loop: ScopeFrame = { name: 'node', value: { label: 'own loop' }, parent: call.alias };
    assert.equal(core.$resolveValue('node.label', loop), 'own loop');
    assert.equal(core.$resolveValue('node.label', call.alias), 'alias');
  });

  test('class defaults do not invent a captured alias absent from SSR bootstrap', () => {
    const core = fragmentCore({});
    Object.defineProperty(core, 'source', { value: { label: 'default' }, configurable: true });
    core.$fragmentHydrating = true;
    const unknown = fragmentCall(core, core.$root!, 'source');
    assert.equal(unknown.alias?.known, false);
    core.$fragmentKnownRoots = new Set(['source']);
    const known = fragmentCall(core, core.$root!, 'source');
    assert.equal(known.alias?.known, true);
    assert.deepEqual(known.alias?.value, { label: 'default' });
    core.$fragmentHydrating = false;
  });

  test('updates aliases before descendants and deduplicates targeted owner work', () => {
    const core = fragmentCore({ tree: { child: { label: 'old' } }, title: 'before' });
    const call = fragmentCall(core, core.$root!, 'tree');
    const nested = fragmentCall(core, call.instance!, 'node.child');
    let writes = 0;
    let data = 'old/before';
    const text = {
      get data() { return data; },
      set data(value: string) { data = value; writes++; },
    } as CharacterData;
    nested.instance!.texts.push({
      node: text, scope: nested.alias, owner: nested.instance!,
      parts: [['node.label'], '/', ['title']],
    });
    core.$buildPathIndex();
    assert.equal(core.$pathIndex?.has('node'), false);
    assert.equal(core.$wildcardBindings, null);
    core.$templateState.title = 'after';
    core.$templateState.tree = { child: { label: 'new' } };
    core.$update('title');
    core.$update('tree');
    core.$flushUpdates();
    assert.equal(data, 'new/after');
    assert.equal(writes, 1);
    assert.equal(core.$fragmentWork.visits, 2);
    core.$update('unrelated');
    core.$flushUpdates();
    assert.equal(writes, 1);
    assert.equal(core.$fragmentWork.visits, 2);
    core.$templateState.title = 'owner only';
    core.$update('title');
    core.$flushUpdates();
    assert.equal(data, 'new/owner only');
    assert.equal(core.$fragmentWork.visits, 0);
  });

  test('owner-only bindings update even when an SSR alias remains unknown', () => {
    const core = fragmentCore({ title: 'before' });
    core.$guardUnknownState = true;
    const call = fragmentCall(core, core.$root!, 'tree');
    const known = { data: 'before' } as CharacterData;
    const unknown = { data: 'trusted SSR' } as CharacterData;
    call.instance!.texts.push(
      { node: known, path: 'title', scope: call.alias, owner: call.instance! },
      { node: unknown, path: 'node.label', scope: call.alias, owner: call.instance! },
    );
    core.$templateState.title = 'after';
    core.$updateInstance(core.$root!);
    assert.equal(known.data, 'after');
    assert.equal(unknown.data, 'trusted SSR');
  });

  test('shared sibling data remains valid and invocation scopes remain distinct', () => {
    const shared = { label: 'shared' };
    const core = fragmentCore({ shared });
    const first = fragmentCall(core, core.$root!, 'shared');
    const second = fragmentCall(core, core.$root!, 'shared');
    core.$updateInstance(core.$root!);
    assert.notEqual(first.alias, second.alias);
    assert.equal(first.alias?.value, second.alias?.value);
    assert.equal(core.$fragmentWork.visits, 2);
  });

  test('deep call chains update and dispose iteratively with the exact depth limit', () => {
    const core = fragmentCore({ tree: {} });
    let owner = core.$root!;
    for (let depth = 1; depth <= 256; depth++) {
      const binding = fragmentCall(core, owner);
      owner = binding.instance!;
    }
    core.$updateInstance(core.$root!);
    assert.equal(core.$fragmentWork.visits, 256);
    fragmentCall(core, owner);
    assert.throws(() => core.$updateInstance(core.$root!), /depth exceeds 256/);
    assert.equal(core.$fragmentWork.active, false);
    assert.equal(core.$fragmentWork.stack.length, 0);
    const root = core.$root!;
    core.$removeInstance(root);
    assert.equal(root.alive, false);
    assert.equal(owner.alive, false);
    assert.equal(root.renders?.length, 0);
  });

  test('teardown restores retained invocation markers for in-place reconnect', () => {
    const core = fragmentCore({ tree: { label: 'retained' } });
    const call = fragmentCall(core, core.$root!, 'tree');
    const start = call.anchor;
    const end = call.end;
    const child = call.instance!;
    let cleaned = 0;
    child.cleanups = [() => { cleaned++; }];
    core.$destroy();
    assert.equal(start.data, 'wf');
    assert.equal(end.data, '/wf');
    assert.equal(child.alive, false);
    assert.equal(child.scope, undefined);
    assert.equal(call.alias, undefined);
    assert.equal(cleaned, 1);
    assert.equal(core.$root, null);
  });
});
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
const PENDING_ROOT_CONNECTED = Symbol.for('microsoft.webui.pendingRootConnected');

/** Register template metadata for a tag exactly like `registerTemplateData()`. */
function registerTemplate(tag: string): void {
  (window as unknown as { __webui: { templates: Record<string, unknown> } }).__webui.templates[tag] = {
    h: '<div></div>',
  };
}

interface ComplexPropertyWriter {
  $writeComplexProperty(
    element: Element,
    name: string,
    value: unknown,
    replayAfterHydration: boolean,
  ): void;
}

interface PendingParentStateConsumer {
  $applyPendingParentState(replayAfterHydration: boolean): void;
}

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

    const parent = new TemplateElement();
    const child = new PendingChild() as PendingChild & {
      localName: string;
      _payload?: unknown;
    };
    child.localName = tag;

    (parent as unknown as ComplexPropertyWriter).$writeComplexProperty(
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
    const parent = new TemplateElement();
    const child = { localName: 'external-property-target' } as unknown as
      Element & { payload?: unknown };

    (parent as unknown as ComplexPropertyWriter).$writeComplexProperty(
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
      [PENDING_ROOT_CONNECTED]?: () => void;
      [STREAMING_BOUNDARY_ACTIVATE](
        state?: Record<string, unknown>,
      ): number;
    };
    raw.tagName = tag;
    raw.$hydrated = true;
    raw.setAttribute('data-ws', '');
    raw[PENDING_ROOT_CONNECTED] = () => {
      received = { status: 'ready' };
      assert.equal(
        raw[STREAMING_BOUNDARY_ACTIVATE](received),
        ACTIVATION_ACTIVATED,
      );
      raw.removeAttribute('data-ws');
    };

    el.connectedCallback();

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
    raw.$meta = { h: '<span></span>' };
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
      override $update(): void {
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

// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { strict as assert } from 'node:assert';
import { describe, test } from 'node:test';
import type { CompiledCondition } from './template.js';
import {
  ATTR_KIND_ATTRIBUTE,
  ATTR_KIND_BOOLEAN,
  ATTR_KIND_COMPLEX,
  ATTR_KIND_TEMPLATE,
  type AttrBinding,
  type CondBinding,
  type RepeatBinding,
  type TemplateInstance,
  type TextBinding,
} from './element/types.js';
import {
  attrDiffersFromDom,
  bindingsDisagreeWithDom,
  condDiffersFromDom,
  formatHydrationMismatch,
  type MismatchContext,
  type PathBindings,
  repeatDiffersFromDom,
  reportHydrationMismatch,
  resetHydrationMismatchWarnings,
  textDiffersFromDom,
  warnHydrationMismatch,
} from './hydration-mismatch.js';

// ── Fakes ───────────────────────────────────────────────────────
// The comparators only read getAttribute/hasAttribute, Text.data,
// Element.innerHTML, and instance counts, so plain objects suffice — no DOM.

function fakeEl(attrs: Record<string, string> = {}, innerHTML = ''): Element {
  return {
    getAttribute: (n: string) => (n in attrs ? attrs[n] : null),
    hasAttribute: (n: string) => n in attrs,
    innerHTML,
  } as unknown as Element;
}

function makeCtx(values: Record<string, unknown> = {}, parts = ''): MismatchContext {
  return {
    resolver: (p) => values[p],
    resolveValue: (p) => values[p],
    resolveParts: () => parts,
  };
}

function cond(fn: (resolve: (p: string, s?: unknown) => unknown) => boolean): CompiledCondition {
  return [(resolve: (p: string, s?: unknown) => unknown) => fn(resolve), []] as unknown as CompiledCondition;
}

const emptyEntry: PathBindings = { texts: [], attrs: [], conds: [], repeats: [] };

// ── textDiffersFromDom ──────────────────────────────────────────

describe('textDiffersFromDom', () => {
  test('path binding matching the DOM does not differ', () => {
    const b = { node: { data: '3' } as unknown as Text, path: 'value' } as TextBinding;
    assert.equal(textDiffersFromDom(b, makeCtx({ value: '3' })), false);
  });

  test('path binding diverging from the DOM differs', () => {
    const b = { node: { data: '' } as unknown as Text, path: 'value' } as TextBinding;
    assert.equal(textDiffersFromDom(b, makeCtx({ value: '3' })), true);
  });

  test('null resolves to empty string, matching an empty text node', () => {
    const b = { node: { data: '' } as unknown as Text, path: 'value' } as TextBinding;
    assert.equal(textDiffersFromDom(b, makeCtx({ value: null })), false);
  });

  test('parts binding compares the resolved template string', () => {
    const b = { node: { data: 'a-b' } as unknown as Text, parts: ['x'] } as unknown as TextBinding;
    assert.equal(textDiffersFromDom(b, makeCtx({}, 'a-b')), false);
    assert.equal(textDiffersFromDom(b, makeCtx({}, 'a-c')), true);
  });

  test('raw bindings are skipped (innerHTML re-serialization is unreliable)', () => {
    const b = {
      node: { data: 'stale' } as unknown as Text,
      path: 'html',
      raw: true,
      rawParent: fakeEl({}, '<b>x</b>'),
    } as TextBinding;
    // Even with a divergent value, raw bindings never report a mismatch.
    assert.equal(textDiffersFromDom(b, makeCtx({ html: '<b>y</b>' })), false);
  });

  test('binding with neither path nor parts never differs', () => {
    const b = { node: { data: 'anything' } as unknown as Text } as TextBinding;
    assert.equal(textDiffersFromDom(b, makeCtx()), false);
  });
});

// ── attrDiffersFromDom ──────────────────────────────────────────

describe('attrDiffersFromDom', () => {
  test('complex :prop bindings are always trusted (never differ)', () => {
    // #286: a child may hydrate before the parent assigns the complex prop.
    const b: AttrBinding = { element: fakeEl(), name: 'data', kind: ATTR_KIND_COMPLEX, path: 'obj' };
    assert.equal(attrDiffersFromDom(b, makeCtx({ obj: { a: 1 } })), false);
  });

  test('boolean attribute compares presence against the condition', () => {
    const present: AttrBinding = {
      element: fakeEl({ disabled: '' }), name: 'disabled', kind: ATTR_KIND_BOOLEAN,
      condition: cond(() => true),
    };
    assert.equal(attrDiffersFromDom(present, makeCtx()), false);

    const shouldBePresent: AttrBinding = {
      element: fakeEl({}), name: 'disabled', kind: ATTR_KIND_BOOLEAN,
      condition: cond(() => true),
    };
    assert.equal(attrDiffersFromDom(shouldBePresent, makeCtx()), true);
  });

  test('template attribute compares the resolved parts string', () => {
    const match: AttrBinding = {
      element: fakeEl({ 'data-x': 'a-b' }), name: 'data-x', kind: ATTR_KIND_TEMPLATE, parts: ['x'],
    };
    assert.equal(attrDiffersFromDom(match, makeCtx({}, 'a-b')), false);

    const differ: AttrBinding = {
      element: fakeEl({ 'data-x': 'a-b' }), name: 'data-x', kind: ATTR_KIND_TEMPLATE, parts: ['x'],
    };
    assert.equal(attrDiffersFromDom(differ, makeCtx({}, 'a-c')), true);
  });

  test('plain attribute compares the stringified value', () => {
    const match: AttrBinding = {
      element: fakeEl({ 'data-value': '3' }), name: 'data-value', kind: ATTR_KIND_ATTRIBUTE, path: 'value',
    };
    assert.equal(attrDiffersFromDom(match, makeCtx({ value: '3' })), false);

    const differ: AttrBinding = {
      element: fakeEl({ 'data-value': '' }), name: 'data-value', kind: ATTR_KIND_ATTRIBUTE, path: 'value',
    };
    assert.equal(attrDiffersFromDom(differ, makeCtx({ value: '3' })), true);
  });

  test('form-control value/checked/selected are skipped', () => {
    for (const name of ['value', 'checked', 'selected']) {
      const b: AttrBinding = {
        element: fakeEl({}), name, kind: ATTR_KIND_ATTRIBUTE, path: 'v',
      };
      assert.equal(attrDiffersFromDom(b, makeCtx({ v: 'x' })), false, name);
    }
  });
});

// ── condDiffersFromDom ──────────────────────────────────────────

describe('condDiffersFromDom', () => {
  const instance = {} as unknown as TemplateInstance;

  test('rendered block with a true condition does not differ', () => {
    const c: CondBinding = { condition: cond(() => true), blockIndex: 0, anchor: {} as Comment, owner: instance, instance };
    assert.equal(condDiffersFromDom(c, makeCtx()), false);
  });

  test('absent block with a false condition does not differ', () => {
    const c: CondBinding = { condition: cond(() => false), blockIndex: 0, anchor: {} as Comment, owner: instance, instance: null };
    assert.equal(condDiffersFromDom(c, makeCtx()), false);
  });

  test('absent block with a true condition differs (server dropped it)', () => {
    const c: CondBinding = { condition: cond(() => true), blockIndex: 0, anchor: {} as Comment, owner: instance, instance: null };
    assert.equal(condDiffersFromDom(c, makeCtx()), true);
  });

  test('rendered block with a false condition differs', () => {
    const c: CondBinding = { condition: cond(() => false), blockIndex: 0, anchor: {} as Comment, owner: instance, instance };
    assert.equal(condDiffersFromDom(c, makeCtx()), true);
  });

  test('render-invariant value change does not differ', () => {
    // count 5 -> 3 both satisfy `count > 0`; the block stays rendered, so a
    // value-only comparison would false-positive but a presence check must not.
    const gtZero = cond((r) => (r('count') as number) > 0);
    const c: CondBinding = { condition: gtZero, blockIndex: 0, anchor: {} as Comment, owner: instance, instance };
    assert.equal(condDiffersFromDom(c, makeCtx({ count: 3 })), false);
  });
});

// ── repeatDiffersFromDom ────────────────────────────────────────

function repeat(collection: string, instanceCount: number): RepeatBinding {
  return {
    markerId: 0, collection, itemVar: 'item', blockIndex: 0,
    container: null, start: null, end: null,
    owner: {} as unknown as TemplateInstance,
    instances: Array.from({ length: instanceCount }, () => ({})) as unknown as RepeatBinding['instances'],
  };
}

describe('repeatDiffersFromDom', () => {
  test('matching lengths do not differ', () => {
    assert.equal(repeatDiffersFromDom(repeat('items', 2), makeCtx({ items: [1, 2] })), false);
  });

  test('length mismatch differs', () => {
    assert.equal(repeatDiffersFromDom(repeat('items', 2), makeCtx({ items: [1, 2, 3] })), true);
  });

  test('non-array collection is treated as length zero', () => {
    assert.equal(repeatDiffersFromDom(repeat('items', 0), makeCtx({ items: undefined })), false);
    assert.equal(repeatDiffersFromDom(repeat('items', 1), makeCtx({ items: undefined })), true);
  });
});

// ── bindingsDisagreeWithDom ─────────────────────────────────────

describe('bindingsDisagreeWithDom', () => {
  test('returns false when every binding agrees', () => {
    const entry: PathBindings = {
      ...emptyEntry,
      attrs: [{ element: fakeEl({ 'data-value': '3' }), name: 'data-value', kind: ATTR_KIND_ATTRIBUTE, path: 'value' }],
    };
    assert.equal(bindingsDisagreeWithDom(entry, makeCtx({ value: '3' })), false);
  });

  test('returns true when any binding disagrees', () => {
    const entry: PathBindings = {
      ...emptyEntry,
      attrs: [{ element: fakeEl({ 'data-value': '' }), name: 'data-value', kind: ATTR_KIND_ATTRIBUTE, path: 'value' }],
    };
    assert.equal(bindingsDisagreeWithDom(entry, makeCtx({ value: '3' })), true);
  });

  test('empty entry never disagrees', () => {
    assert.equal(bindingsDisagreeWithDom(emptyEntry, makeCtx()), false);
  });
});

// ── formatHydrationMismatch ─────────────────────────────────────

describe('formatHydrationMismatch', () => {
  test('names the tag, quotes each path, and gives the two fixes', () => {
    const msg = formatHydrationMismatch('x-widget', ['show', 'value']);
    assert.match(msg, /<x-widget>/);
    assert.match(msg, /"show", "value"/);
    assert.match(msg, /super\.connectedCallback\(\)/);
    assert.match(msg, /SSR state/);
  });
});

// ── warnHydrationMismatch (per-tag dedup) ───────────────────────

describe('warnHydrationMismatch', () => {
  function captureWarn(fn: () => void): string[] {
    const original = console.warn;
    const calls: string[] = [];
    console.warn = (msg?: unknown) => { calls.push(String(msg)); };
    try {
      fn();
    } finally {
      console.warn = original;
    }
    return calls;
  }

  test('warns once per (tag, path-set) even when called repeatedly', () => {
    resetHydrationMismatchWarnings();
    const calls = captureWarn(() => {
      warnHydrationMismatch('x-dedup', ['show']);
      warnHydrationMismatch('x-dedup', ['show']);
    });
    assert.equal(calls.length, 1);
  });

  test('path-set order does not affect dedup', () => {
    resetHydrationMismatchWarnings();
    const calls = captureWarn(() => {
      warnHydrationMismatch('x-order', ['show', 'value']);
      warnHydrationMismatch('x-order', ['value', 'show']);
    });
    assert.equal(calls.length, 1);
  });

  test('a genuinely different mismatch on the same tag still warns', () => {
    resetHydrationMismatchWarnings();
    const calls = captureWarn(() => {
      warnHydrationMismatch('x-diff', ['show']);
      warnHydrationMismatch('x-diff', ['value']);
    });
    assert.equal(calls.length, 2);
  });

  test('warns separately for distinct tags', () => {
    resetHydrationMismatchWarnings();
    const calls = captureWarn(() => {
      warnHydrationMismatch('x-tag-a', ['show']);
      warnHydrationMismatch('x-tag-b', ['show']);
    });
    assert.equal(calls.length, 2);
  });
});

// ── reportHydrationMismatch (dynamic-import entry point) ─────────

describe('reportHydrationMismatch', () => {
  function captureWarn(fn: () => void): string[] {
    const original = console.warn;
    const calls: string[] = [];
    console.warn = (msg?: unknown) => { calls.push(String(msg)); };
    try {
      fn();
    } finally {
      console.warn = original;
    }
    return calls;
  }

  function disagreeingEntry(): PathBindings {
    return {
      ...emptyEntry,
      attrs: [{ element: fakeEl({ 'data-value': '' }), name: 'data-value', kind: ATTR_KIND_ATTRIBUTE, path: 'value' }],
    };
  }

  function agreeingEntry(): PathBindings {
    return {
      ...emptyEntry,
      attrs: [{ element: fakeEl({ 'data-value': '3' }), name: 'data-value', kind: ATTR_KIND_ATTRIBUTE, path: 'value' }],
    };
  }

  test('warns once naming the diverged path when an entry disagrees', () => {
    resetHydrationMismatchWarnings();
    const index = new Map<string, PathBindings>([['value', disagreeingEntry()]]);
    const calls = captureWarn(() => {
      reportHydrationMismatch('x-report', new Set(['value']), index, makeCtx({ value: '3' }));
    });
    assert.equal(calls.length, 1);
    assert.match(calls[0], /<x-report>/);
    assert.match(calls[0], /"value"/);
  });

  test('stays silent when every recorded write agrees with the DOM', () => {
    resetHydrationMismatchWarnings();
    const index = new Map<string, PathBindings>([['value', agreeingEntry()]]);
    const calls = captureWarn(() => {
      reportHydrationMismatch('x-agree', new Set(['value']), index, makeCtx({ value: '3' }));
    });
    assert.equal(calls.length, 0);
  });

  test('stays silent when there are no recorded writes', () => {
    resetHydrationMismatchWarnings();
    const index = new Map<string, PathBindings>([['value', disagreeingEntry()]]);
    const calls = captureWarn(() => {
      reportHydrationMismatch('x-empty', new Set(), index, makeCtx({ value: '3' }));
    });
    assert.equal(calls.length, 0);
  });

  test('skips write paths absent from the index and reports only real divergences', () => {
    resetHydrationMismatchWarnings();
    const index = new Map<string, PathBindings>([['value', disagreeingEntry()]]);
    const calls = captureWarn(() => {
      reportHydrationMismatch('x-mixed', new Set(['missing', 'value']), index, makeCtx({ value: '3' }));
    });
    assert.equal(calls.length, 1);
    assert.match(calls[0], /"value"/);
    assert.doesNotMatch(calls[0], /missing/);
  });
});

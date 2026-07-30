// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { strict as assert } from 'node:assert';
import { describe, test } from 'node:test';

import type { BoundaryBootstrap } from './streaming.js';

// `streaming.ts` declares a sentinel custom element class at module scope
// (`class WebUiHydrateSentinel extends HTMLElement`), so `HTMLElement` must
// exist globally before the module is evaluated — mirrors the dynamic-import-
// after-mock pattern used by static-host.test.ts.
Object.defineProperty(globalThis, 'HTMLElement', {
  value: class HTMLElement {},
  configurable: true,
});

const { parseBoundaryEnvelope } = await import('./streaming.js');

describe('parseBoundaryEnvelope', () => {
  test('accepts a well-formed non-terminal boundary envelope', () => {
    const result = parseBoundaryEnvelope(
      '[1,0,0,0,{"inventory":"01","state":{"count":1},"templates":{"my-counter":{"h":"<button></button>"}}}]',
    );
    assert.equal(result.ok, true);
    if (!result.ok) return;
    const [version, sequence, kind, target, payload] = result.envelope;
    const bootstrap = payload as BoundaryBootstrap;
    assert.equal(version, 1);
    assert.equal(sequence, 0);
    assert.equal(kind, 0);
    assert.equal(target, 0);
    assert.equal(bootstrap.inventory, '01');
    assert.deepEqual(bootstrap.state, { count: 1 });
    assert.ok(bootstrap.templates?.['my-counter']);
  });

  test('accepts the empty terminal boundary envelope', () => {
    const result = parseBoundaryEnvelope('[1,2,3,0,{}]');
    assert.equal(result.ok, true);
    if (!result.ok) return;
    const [, sequence, kind, target, bootstrap] = result.envelope;
    assert.equal(sequence, 2);
    assert.equal(kind, 3);
    assert.equal(target, 0);
    assert.deepEqual(bootstrap, {});
  });

  test('accepts a projected state update record', () => {
    const result = parseBoundaryEnvelope('[1,2,2,0,{"forecast":"Sunny"}]');
    assert.equal(result.ok, true);
    if (!result.ok) return;
    const [, sequence, kind, target, patch] = result.envelope;
    assert.equal(sequence, 2);
    assert.equal(kind, 2);
    assert.equal(target, 0);
    assert.deepEqual(patch, { forecast: 'Sunny' });
  });

  test('rejects invalid JSON', () => {
    const result = parseBoundaryEnvelope('[1,0,0,0,{');
    assert.equal(result.ok, false);
    if (result.ok) return;
    assert.match(result.reason, /not valid JSON/);
  });

  test('rejects a non-array envelope', () => {
    const result = parseBoundaryEnvelope('{"version":1}');
    assert.equal(result.ok, false);
    if (result.ok) return;
    assert.match(result.reason, /5-element/);
  });

  test('rejects an envelope with the wrong element count', () => {
    const result = parseBoundaryEnvelope('[1,0,0,0]');
    assert.equal(result.ok, false);
    if (result.ok) return;
    assert.match(result.reason, /5-element/);
  });

  test('rejects an unsupported version', () => {
    const result = parseBoundaryEnvelope('[2,0,0,0,{}]');
    assert.equal(result.ok, false);
    if (result.ok) return;
    assert.match(result.reason, /unsupported boundary envelope version/);
  });

  // Fields past the version gate are written by the Rust checkpoint serializer
  // and are deliberately not re-validated here. A record that reaches this
  // parser intact but is inconsistent with document state is rejected by the
  // coordinator instead, which is covered in streaming-pipeline.test.ts.
  test('passes malformed trailing fields through to the coordinator', () => {
    for (const record of [
      '[1,-1,0,0,{}]',
      '[1,0,9,0,{}]',
      '[1,0,0,-1,{}]',
      '[1,0,0,0,null]',
      '[1,2,3,0,{"state":{"count":1}}]',
    ]) {
      const result = parseBoundaryEnvelope(record);
      assert.equal(result.ok, true, `expected ${record} to parse`);
    }
  });
});

// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { strict as assert } from 'node:assert';
import { describe, test } from 'node:test';

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
      '[1,0,0,{"inventory":"01","state":{"count":1},"templates":{"my-counter":{"h":"<button></button>"}}}]',
    );
    assert.equal(result.ok, true);
    if (!result.ok) return;
    const [version, sequence, terminal, bootstrap] = result.envelope;
    assert.equal(version, 1);
    assert.equal(sequence, 0);
    assert.equal(terminal, 0);
    assert.equal(bootstrap.inventory, '01');
    assert.deepEqual(bootstrap.state, { count: 1 });
    assert.ok(bootstrap.templates?.['my-counter']);
  });

  test('accepts the empty terminal boundary envelope', () => {
    const result = parseBoundaryEnvelope('[1,2,1,{}]');
    assert.equal(result.ok, true);
    if (!result.ok) return;
    const [, sequence, terminal, bootstrap] = result.envelope;
    assert.equal(sequence, 2);
    assert.equal(terminal, 1);
    assert.deepEqual(bootstrap, {});
  });

  test('rejects invalid JSON', () => {
    const result = parseBoundaryEnvelope('[1,0,0,{');
    assert.equal(result.ok, false);
    if (result.ok) return;
    assert.match(result.reason, /not valid JSON/);
  });

  test('rejects a non-array envelope', () => {
    const result = parseBoundaryEnvelope('{"version":1}');
    assert.equal(result.ok, false);
    if (result.ok) return;
    assert.match(result.reason, /4-element/);
  });

  test('rejects an envelope with the wrong element count', () => {
    const result = parseBoundaryEnvelope('[1,0,0]');
    assert.equal(result.ok, false);
    if (result.ok) return;
    assert.match(result.reason, /4-element/);
  });

  test('rejects an unsupported version', () => {
    const result = parseBoundaryEnvelope('[2,0,0,{}]');
    assert.equal(result.ok, false);
    if (result.ok) return;
    assert.match(result.reason, /unsupported boundary envelope version/);
  });

  test('rejects a negative or non-integer sequence', () => {
    for (const bad of ['[1,-1,0,{}]', '[1,1.5,0,{}]', '[1,"0",0,{}]']) {
      const result = parseBoundaryEnvelope(bad);
      assert.equal(result.ok, false, `expected ${bad} to be rejected`);
      if (result.ok) continue;
      assert.match(result.reason, /sequence must be a non-negative integer/);
    }
  });

  test('rejects a terminal flag that is not 0 or 1', () => {
    const result = parseBoundaryEnvelope('[1,0,2,{}]');
    assert.equal(result.ok, false);
    if (result.ok) return;
    assert.match(result.reason, /terminal flag must be 0 or 1/);
  });

  test('rejects a non-object bootstrap', () => {
    for (const bad of ['[1,0,0,null]', '[1,0,0,[1,2]]', '[1,0,0,"x"]']) {
      const result = parseBoundaryEnvelope(bad);
      assert.equal(result.ok, false, `expected ${bad} to be rejected`);
      if (result.ok) continue;
      assert.match(result.reason, /bootstrap must be an object/);
    }
  });

  test('rejects a boundary payload larger than the size cap', () => {
    const huge = `[1,0,0,{"state":{"pad":"${'x'.repeat(2_100_000)}"}}]`;
    const result = parseBoundaryEnvelope(huge);
    assert.equal(result.ok, false);
    if (result.ok) return;
    assert.match(result.reason, /exceeds .* characters/);
  });

  test('rejects a boundary declaring more templates than the cap', () => {
    const templates: Record<string, unknown> = {};
    for (let i = 0; i < 501; i++) templates[`tag-${i}`] = { h: '<p></p>' };
    const result = parseBoundaryEnvelope(JSON.stringify([1, 0, 0, { templates }]));
    assert.equal(result.ok, false);
    if (result.ok) return;
    assert.match(result.reason, /more than 500 templates/);
  });
});

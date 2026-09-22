// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import test from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { createRequire } from 'node:module';
import { resolve } from 'node:path';
import { runInNewContext } from 'node:vm';
import { build } from 'esbuild';
import { defaultLimits, type MessageCodec } from '../src/index.js';

const fixture = resolve('../../crates/webui-desktop/tests/fixtures/typed-ipc');
const require = createRequire(import.meta.url);
type FixtureCodec = MessageCodec<Record<string, unknown>>;

function assertUnchangedPrototype(prototype: object, descriptors: object): void {
  assert.deepEqual(Reflect.ownKeys(prototype), Reflect.ownKeys(descriptors));
  for (const key of Reflect.ownKeys(descriptors)) {
    const actual = Object.getOwnPropertyDescriptor(prototype, key)!;
    const expected = Object.getOwnPropertyDescriptor(descriptors, key)!.value as PropertyDescriptor;
    for (const field of ['value', 'writable', 'get', 'set', 'enumerable', 'configurable'] as const) {
      assert.equal(actual[field], expected[field], `${String(key)}.${field} changed`);
    }
  }
}

async function loadFixture() {
  const result = await build({
    // Codec invariants exercise the private generated implementation, not the lazy application facade.
    entryPoints: [resolve(fixture, 'ts/ipc-runtime.ts')],
    bundle: true, write: false, format: 'cjs', platform: 'node', target: 'es2022',
    alias: {
      '@microsoft/webui-desktop': resolve('src/index.ts'),
      '@bufbuild/protobuf/wire': require.resolve('@bufbuild/protobuf/wire'),
    },
  });
  const module: { exports: unknown } = { exports: {} };
  runInNewContext(result.outputFiles[0]!.text, {
    module, exports: module.exports, require, Map, TextEncoder, TextDecoder,
    Uint8Array, DataView, ArrayBuffer,
  });
  const { schema } = module.exports as { schema: { methods: { id: number; request: FixtureCodec }[] } };
  const method = schema.methods.find(method => method.id === 1101);
  assert(method);
  return { codec: method.request, objectPrototype: Object.getPrototypeOf(schema) as object };
}

test('regenerated Map codecs preserve prototype-like strings without prototype pollution', async () => {
  const { codec, objectPrototype } = await loadFixture();
  const hostDescriptors = Object.getOwnPropertyDescriptors(Object.prototype);
  const fixtureDescriptors = Object.getOwnPropertyDescriptors(objectPrototype);
  const mapDescriptors = Object.getOwnPropertyDescriptors(Map.prototype);
  const value = codec.decode(new Uint8Array(readFileSync(resolve(fixture, 'golden.bin'))));
  assert(value.labels instanceof Map);
  for (const key of ['__proto__', 'constructor', 'prototype']) value.labels.set(key, `${key} is data`);
  codec.validate(value, defaultLimits);
  const encoded = codec.encode(value);
  codec.validateBytes(encoded, defaultLimits);
  const decoded = codec.decode(encoded);
  assert(decoded.labels instanceof Map);
  for (const key of ['__proto__', 'constructor', 'prototype']) assert.equal(decoded.labels.get(key), `${key} is data`);
  assert.equal(Object.getPrototypeOf(decoded), objectPrototype);
  assertUnchangedPrototype(objectPrototype, fixtureDescriptors);
  assertUnchangedPrototype(Object.prototype, hostDescriptors);
  assertUnchangedPrototype(Map.prototype, mapDescriptors);
  assert.deepEqual(decoded, value);
});

test('Rust and TypeScript golden maps roundtrip semantically with lossless typed keys', async () => {
  const { codec } = await loadFixture();
  const values = ['golden.bin', 'golden-ts.bin'].map(name => {
    const bytes = new Uint8Array(readFileSync(resolve(fixture, name)));
    codec.validateBytes(bytes, defaultLimits);
    const value = codec.decode(bytes);
    codec.validate(value, defaultLimits);
    assert.deepEqual(codec.decode(codec.encode(value)), value);
    return value;
  });

  // Protobuf field and map-entry order need not be identical across languages.
  assert.deepEqual(values[0], values[1]);
  const value = values[0]!;
  for (const [field, type] of [
    ['labels', 'string'], ['chunks', 'number'], ['flags', 'boolean'],
    ['unsignedKeys', 'bigint'], ['signedKeys', 'bigint'],
  ] as const) {
    const map = value[field];
    assert(map instanceof Map);
    assert(map.size > 0);
    for (const key of map.keys()) assert.equal(typeof key, type);
  }
  assert((value.chunks as Map<number, unknown>).has(-2147483648));
  assert((value.flags as Map<boolean, unknown>).has(false));
  assert((value.flags as Map<boolean, unknown>).has(true));
  assert((value.unsignedKeys as Map<bigint, unknown>).has(0xffffffffffffffffn));
  assert((value.signedKeys as Map<bigint, unknown>).has(-0x8000000000000000n));
  assert((value.signedKeys as Map<bigint, unknown>).has(0x7fffffffffffffffn));
});

test('a generated 6000-entry Map passes both value and wire validation', async () => {
  const { codec } = await loadFixture();
  const value = codec.decode(new Uint8Array());
  const labels = new Map<string, string>();
  for (let i = 0; i < 6000; i++) labels.set(`key${i}`, `value${i}`);
  value.labels = labels;
  codec.validate(value, defaultLimits);
  const bytes = codec.encode(value);
  codec.validateBytes(bytes, defaultLimits);
  assert.deepEqual(codec.decode(bytes), value);
});

test('generated Map and packed collections roundtrip at the exact shared entry cap', async () => {
  const { codec } = await loadFixture();
  const cap = defaultLimits.maxCollectionEntriesPerMessage;
  for (const mapEntries of [cap, 0, cap / 2]) {
    const value = codec.decode(new Uint8Array());
    const labels = new Map<string, string>();
    for (let i = 0; i < mapEntries; i++) labels.set(`key${i}`, `value${i}`);
    const scores = value.scores as number[];
    scores.length = cap - mapEntries;
    scores.fill(0);
    value.labels = labels;
    codec.validate(value, defaultLimits);
    const bytes = codec.encode(value);
    codec.validateBytes(bytes, defaultLimits);
    assert.deepEqual(codec.decode(bytes), value);
    scores.push(0);
    assert.throws(() => codec.validate(value, defaultLimits), { code: 'invalid-payload' });
    assert.throws(() => codec.validateBytes(codec.encode(value), defaultLimits), { code: 'invalid-payload' });
  }
});

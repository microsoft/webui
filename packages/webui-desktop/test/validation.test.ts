// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import test from 'node:test';
import assert from 'node:assert/strict';
import { BinaryWriter } from '@bufbuild/protobuf/wire';
import { defaultLimits, validateValue, validateMessage, type MessageShape } from '../src/index.js';
import { decodeFrame } from '../src/framing.js';
import { BoundaryReader, readDelimited, readUint32, readUint64, skipField } from '../src/boundary.js';
import { item, itemCodec, shapes, frame, IpcFrame, Kind } from './helpers.js';

test('generated codecs preserve extreme integers and 256 KiB bytes exactly', () => {
  const large = { ...item, data: new Uint8Array(256 * 1024).fill(255) };
  itemCodec.validate(large, defaultLimits);
  const encoded = itemCodec.encode(large);
  itemCodec.validateBytes(encoded, defaultLimits);
  assert.deepEqual(itemCodec.decode(encoded), large);
  const small = itemCodec.encode(item);
  assert.equal(Buffer.from(small).toString('hex'), '08ffffffffffffffffff0110ffffffffffffffffff011a030080ff');
});

test('early scalar validation rejects narrowing, wrong bytes, and out-of-range bigint', () => {
  for (const bad of [
    { ...item, id: 1 }, { ...item, id: -1n }, { ...item, id: 1n << 64n },
    { ...item, signed: 1n << 63n }, { ...item, data: [1, 2] },
  ]) assert.throws(() => validateValue(bad, 0, shapes, defaultLimits), { code: 'invalid-payload' });
  const floats: MessageShape[] = [{ fields: [{ number: 1, name: 'value', kind: 'double', repeated: false, optional: false, mapKey: false }] }];
  for (const value of [Infinity, -Infinity, NaN]) validateValue({ value }, 0, floats, defaultLimits);
});

test('wire guard rejects groups, malformed fields and bounded collections before decode', () => {
  assert.throws(() => validateMessage(new Uint8Array([11, 12]), 0, shapes, defaultLimits));
  assert.throws(() => validateMessage(new Uint8Array([26, 50, 1]), 0, shapes, defaultLimits));
  const many = new BinaryWriter();
  for (let i = 0; i < 5; i++) many.uint32(8).uint64(1n);
  const repeated: MessageShape[] = [{ fields: [{ ...shapes[0]!.fields[0]!, repeated: true }] }];
  assert.throws(() => validateMessage(many.finish(), 0, repeated, { ...defaultLimits, maxCollectionEntriesPerMessage: 4 }));
});

test('prototype-like Map keys and oneof conflicts validate on value and wire paths', () => {
  const field = { number: 1, name: 'values', kind: 'message', message: 1, repeated: true, optional: false, mapKey: false, map: true };
  const messages: MessageShape[] = [{ fields: [field] }, { fields: [
    { number: 1, name: 'key', kind: 'string', repeated: false, optional: false, mapKey: true },
    { number: 2, name: 'value', kind: 'uint64', repeated: false, optional: false, mapKey: false },
  ] }];
  validateValue({ values: new Map([['good', 1n], ['__proto__', 2n], ['constructor', 3n], ['prototype', 4n]]) }, 0, messages, defaultLimits);
  assert.throws(() => validateValue({ values: { good: 1n } }, 0, messages, defaultLimits));
  assert.throws(() => validateValue({ values: new Map([['key', 1n]]) }, 0, messages, {
    ...defaultLimits, maxSchemaDepth: 1,
  }), { code: 'invalid-payload' });
  assert.throws(() => validateValue({ values: new Map([['large', 1n]]) }, 0, messages, {
    ...defaultLimits, maxFrameBytes: 4,
  }), { code: 'payload-too-large' });
  const bytes = new BinaryWriter().uint32(10).fork().uint32(10).string('constructor').uint32(16).uint64(1n).join().finish();
  validateMessage(bytes, 0, messages, defaultLimits);
  const oneof: MessageShape[] = [{ fields: [
    { number: 1, name: 'a', kind: 'string', oneof: 'choice', repeated: false, optional: false, mapKey: false },
    { number: 2, name: 'b', kind: 'string', oneof: 'choice', repeated: false, optional: false, mapKey: false },
  ] }];
  validateValue({ choice: { $case: 'a', value: 'ok' } }, 0, oneof, defaultLimits);
  assert.throws(() => validateValue({ choice: { $case: 'missing', value: 'bad' } }, 0, oneof, defaultLimits));
  assert.throws(() => validateMessage(new BinaryWriter().uint32(10).string('a').uint32(18).string('b').finish(), 0, oneof, defaultLimits));
});

test('envelope rejects ambiguous body, illegal kind, groups and malformed lengths', () => {
  const valid = IpcFrame.encode(frame(1n, 1n, Kind.ACCEPT)).finish();
  assert.equal(decodeFrame(valid, defaultLimits).id, 1n);
  for (const invalid of [
    IpcFrame.encode({ ...frame(1n, 1n, Kind.ACCEPT), body: { $case: 'payload', value: new Uint8Array() } }).finish(),
    IpcFrame.encode(frame(1n, 1n, 99 as Kind)).finish(),
    new Uint8Array([11, 12]),
    new Uint8Array([58, 255]),
  ]) assert.throws(() => decodeFrame(invalid, defaultLimits));
  const maximum = IpcFrame.encode(frame(0xffffffffffffffffn, 0xffffffffffffffffn, Kind.ACCEPT)).finish();
  assert.equal(Buffer.from(maximum).toString('hex'), '080211ffffffffffffffff19ffffffffffffffff2005');
  assert.equal(decodeFrame(maximum, defaultLimits).generation, 0xffffffffffffffffn);
});

test('wire guard prevents uint32 and length-prefix narrowing before generated decoding', () => {
  const messages: MessageShape[] = [{ fields: [
    { number: 1, name: 'value', kind: 'uint32', repeated: false, optional: false, mapKey: false },
  ] }];
  assert.throws(() => validateMessage(new BinaryWriter().uint32(8).uint64(0x100000001n).finish(), 0, messages, defaultLimits));
  assert.throws(() => validateMessage(new BinaryWriter().uint32(18).uint64(0x100000000n).finish(), 0, messages, defaultLimits));
});

test('bigint Map keys, recursive values and schema depth limits', () => {
  const messages: MessageShape[] = [
    { fields: [{ number: 1, name: 'values', kind: 'message', message: 1, repeated: true, optional: false, mapKey: false, map: true }] },
    { fields: [
      { number: 1, name: 'key', kind: 'uint64', repeated: false, optional: false, mapKey: true },
      { number: 2, name: 'value', kind: 'bool', repeated: false, optional: false, mapKey: false },
    ] },
  ];
  validateValue({ values: new Map([[0xffffffffffffffffn, true]]) }, 0, messages, defaultLimits);
  for (const key of [1, '1', '18446744073709551615', -1n, 0x10000000000000000n]) {
    assert.throws(() => validateValue({ values: new Map([[key, true]]) }, 0, messages, defaultLimits));
  }
  const recursive: MessageShape[] = [{ fields: [{ number: 1, name: 'child', kind: 'message', message: 0, repeated: false, optional: true, mapKey: false }] }];
  const cycle: { child?: unknown } = {};
  cycle.child = cycle;
  assert.throws(() => validateValue(cycle, 0, recursive, defaultLimits));
});

test('Map keys preserve scalar types and full integer ranges without coercion', () => {
  const cases: { kind: string; valid: unknown[]; invalid: unknown[] }[] = [
    { kind: 'string', valid: ['', '__proto__', 'constructor', 'prototype'], invalid: [0, false, 1n] },
    { kind: 'bool', valid: [true, false], invalid: ['true', 'false', 0, 1] },
    { kind: 'int32', valid: [-2147483648, 2147483647], invalid: [-2147483649, 2147483648, '1', 1n, 1.5, NaN] },
    { kind: 'uint32', valid: [0, 4294967295], invalid: [-1, 4294967296, '1', 1n, Infinity] },
    { kind: 'int64', valid: [-0x8000000000000000n, 0x7fffffffffffffffn], invalid: [-0x8000000000000001n, 0x8000000000000000n, 1, '1'] },
    { kind: 'uint64', valid: [0n, 0xffffffffffffffffn], invalid: [-1n, 0x10000000000000000n, 1, '1'] },
  ];
  for (const { kind, valid, invalid } of cases) {
    const messages: MessageShape[] = [
      { fields: [{ number: 1, name: 'values', kind: 'message', message: 1, repeated: true, optional: false, mapKey: false, map: true }] },
      { fields: [
        { number: 1, name: 'key', kind, repeated: false, optional: false, mapKey: true },
        { number: 2, name: 'value', kind: 'bool', repeated: false, optional: false, mapKey: false },
      ] },
    ];
    for (const key of valid) validateValue({ values: new Map([[key, true]]) }, 0, messages, defaultLimits);
    for (const key of invalid) assert.throws(() => validateValue({ values: new Map([[key, true]]) }, 0, messages, defaultLimits), { code: 'invalid-payload' });
    assert.throws(() => validateValue({ values: new Map(valid.map(key => [key, true])) }, 0, messages, {
      ...defaultLimits, maxCollectionEntriesPerMessage: 1,
    }), { code: 'invalid-payload' });
  }
});

test('raw varints reject u64 overflow, excess bytes and truncation before codec narrowing', () => {
  const maximum = new BinaryWriter().uint64(0xffffffffffffffffn).finish();
  assert.equal(readUint64(new BoundaryReader(maximum)), 0xffffffffffffffffn);
  const overflow = new Uint8Array(10).fill(128);
  overflow[9] = 2; // 2^64, which the unguarded codec truncates to zero.
  const overlong = new Uint8Array(11).fill(128);
  overlong[10] = 0;
  for (const malformed of [overflow, overlong, new Uint8Array(9).fill(128)]) {
    assert.throws(() => readUint64(new BoundaryReader(malformed)), { code: 'invalid-payload' });
    assert.throws(() => readUint32(new BoundaryReader(malformed)), { code: 'invalid-payload' });
    assert.throws(() => readDelimited(new BoundaryReader(malformed)), { code: 'invalid-payload' });
    assert.throws(() => skipField(new BoundaryReader(malformed), 0), { code: 'invalid-payload' });
    assert.throws(() => new BoundaryReader(malformed).tag(), { code: 'invalid-payload' });
    const bytes = new Uint8Array(1 + malformed.length);
    bytes[0] = 8;
    bytes.set(malformed, 1);
    for (const kind of ['uint64', 'int64', 'sint64', 'uint32', 'int32', 'sint32', 'enum', 'bool']) {
      const message: MessageShape[] = [{ fields: [{
        number: 1, name: 'value', kind, repeated: false, optional: false, mapKey: false,
      }] }];
      assert.throws(() => validateMessage(bytes, 0, message, defaultLimits), { code: 'invalid-payload' });
    }
    assert.throws(() => decodeFrame(bytes, defaultLimits), { code: 'invalid-frame' });
    bytes[0] = 26; // Length-delimited payload length uses the same raw guard.
    assert.throws(() => validateMessage(bytes, 0, shapes, defaultLimits), { code: 'invalid-payload' });
  }
  const valid = new BinaryWriter().uint32(8).uint64(0xffffffffffffffffn).finish();
  itemCodec.validateBytes(valid, defaultLimits);
  assert.equal(itemCodec.decode(valid).id, 0xffffffffffffffffn);
});

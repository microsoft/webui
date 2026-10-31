// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import { strict as assert } from 'node:assert';
import { test } from 'node:test';
import { NdjsonReader } from './ndjson-reader.js';

const encoder = new TextEncoder();

function body(parts: readonly Uint8Array[], cancel?: () => void): ReadableStream<Uint8Array> {
  return new ReadableStream({
    start(controller) {
      for (const part of parts) controller.enqueue(part);
      controller.close();
    },
    cancel,
  });
}

test('reads split UTF-8, CRLF, blank lines, and an unterminated final record', async () => {
  const bytes = encoder.encode('\n{"name":"café"}\r\n \n{"count":2}');
  const stream = body(Array.from(bytes, byte => new Uint8Array([byte])));
  const reader = new NdjsonReader(stream);
  assert.deepEqual(await reader.read(), { name: 'café' });
  assert.deepEqual(await reader.read(), { count: 2 });
  assert.equal(await reader.read(), undefined);
  assert.equal(stream.locked, false);
});

test('bounds individual record bytes rather than a coalesced transport chunk', async () => {
  const stream = body([encoder.encode('{"n":1}\n{"n":2}\n{"n":3}\n')]);
  const reader = new NdjsonReader(stream, { maxRecordBytes: 7 });
  assert.deepEqual(await reader.read(), { n: 1 });
  assert.deepEqual(reader.readBuffered(), { n: 2 });
  assert.deepEqual(reader.readBuffered(), { n: 3 });
  assert.equal(reader.readBuffered(), undefined);
  assert.equal(await reader.read(), undefined);
});

test('drains the remaining records in order without retaining the first record', async () => {
  const stream = body([encoder.encode('{"n":1}\n{"n":2}\n'), encoder.encode('{"n":3}')]);
  const reader = new NdjsonReader(stream);
  assert.deepEqual(await reader.read(), { n: 1 });
  const remaining: unknown[] = [];
  await reader.drain(value => { remaining.push(value); });
  assert.deepEqual(remaining, [{ n: 2 }, { n: 3 }]);
  assert.equal(stream.locked, false);
});

test('the record limit counts UTF-8 bytes, not decoded string length', async () => {
  const bytes = encoder.encode('"é"');
  const exact = new NdjsonReader(body([bytes]), { maxRecordBytes: 4 });
  assert.equal(await exact.read(), 'é');
  assert.equal(await exact.read(), undefined);

  const oversized = new NdjsonReader(body([bytes]), { maxRecordBytes: 3 });
  await assert.rejects(oversized.read(), /exceeds 3 bytes/);
});

test('oversized split records fail before allocating an unbounded decoded string', async () => {
  const stream = body([encoder.encode('{"a":'), encoder.encode('"1234567890"}\n')]);
  const reader = new NdjsonReader(stream, { maxRecordBytes: 8 });
  await assert.rejects(reader.read(), /exceeds 8 bytes/);
  assert.equal(stream.locked, false);
});

for (const [name, bytes, error] of [
  ['malformed JSON', encoder.encode('{"bad":}\n'), /Invalid NDJSON record/],
  ['invalid UTF-8', new Uint8Array([34, 0xff, 34, 10]), /Invalid UTF-8/],
] as const) {
  test(`${name} fails explicitly and releases the reader`, async () => {
    const stream = body([bytes]);
    const reader = new NdjsonReader(stream);
    await assert.rejects(reader.read(), error);
    assert.equal(stream.locked, false);
  });
}

test('abort releases a stalled read without waiting for another network chunk', async () => {
  let cancelled = false;
  const stream = new ReadableStream<Uint8Array>({
    cancel() { cancelled = true; },
  });
  const controller = new AbortController();
  const reader = new NdjsonReader(stream, { signal: controller.signal });
  const pending = reader.read();
  controller.abort(new DOMException('Superseded', 'AbortError'));
  await assert.rejects(pending, { name: 'AbortError' });
  assert.equal(cancelled, true);
  assert.equal(stream.locked, false);
});

test('explicit cancellation discards an unread tail and is idempotent', async () => {
  let cancellations = 0;
  const stream = new ReadableStream<Uint8Array>({
    start(controller) {
      controller.enqueue(encoder.encode('{"first":true}\n{"second":true}\n'));
    },
    cancel() { cancellations++; },
  });
  const reader = new NdjsonReader(stream);
  assert.deepEqual(await reader.read(), { first: true });
  await reader.cancel();
  await reader.cancel();
  assert.equal(cancellations, 1);
  assert.equal(reader.readBuffered(), undefined);
  assert.equal(await reader.read(), undefined);
  assert.equal(stream.locked, false);
});

test('invalid buffered records also release an abandoned body', async () => {
  let cancelled = false;
  const stream = new ReadableStream<Uint8Array>({
    start(controller) { controller.enqueue(encoder.encode('{}\ninvalid\n')); },
    cancel() { cancelled = true; },
  });
  const reader = new NdjsonReader(stream);
  assert.deepEqual(await reader.read(), {});
  assert.throws(() => reader.readBuffered(), /Invalid NDJSON record/);
  assert.equal(cancelled, true);
  assert.equal(stream.locked, false);
});

test('concurrent reads fail without stealing the first operation', async () => {
  let source!: ReadableStreamDefaultController<Uint8Array>;
  const stream = new ReadableStream<Uint8Array>({
    start(controller) { source = controller; },
  });
  const reader = new NdjsonReader(stream);
  const first = reader.read();
  await assert.rejects(reader.read(), /Await the current NDJSON read/);
  assert.throws(() => reader.readBuffered(), /Await the current NDJSON read/);
  source.enqueue(encoder.encode('{"ok":true}\n'));
  source.close();
  assert.deepEqual(await first, { ok: true });
  assert.equal(await reader.read(), undefined);
});

test('an aborted constructor does not lock the response body', () => {
  const stream = body([]);
  const controller = new AbortController();
  controller.abort();
  assert.throws(() => new NdjsonReader(stream, { signal: controller.signal }), { name: 'AbortError' });
  assert.equal(stream.locked, false);
});

test('invalid record budgets are rejected before reading', () => {
  for (const maxRecordBytes of [0, -1, Number.NaN, Number.POSITIVE_INFINITY, 1.5]) {
    const stream = body([]);
    assert.throws(() => new NdjsonReader(stream, { maxRecordBytes }), /positive safe integer/);
    assert.equal(stream.locked, false);
  }
});

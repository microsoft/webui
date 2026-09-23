// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import test from 'node:test';
import assert from 'node:assert/strict';
import { connect, createConnection, IpcError } from '../src/index.js';
import { FakeTransport, schema, save, selected, changed, label, item, itemCodec, invocation, frame, Kind, deferred, turn } from './helpers.js';

test('connection sends only the four Hello fields, never local schema properties', async () => {
  const transport = new FakeTransport();
  const start = transport.start.bind(transport);
  transport.start = (hello, receiver) => {
    assert.deepEqual(Reflect.ownKeys(hello as object).sort(), ['contractMajor', 'contractName', 'schemaHash', 'wireVersion']);
    return start(hello, receiver);
  };
  const local = { ...schema, localCodec: () => {}, debug: { local: true } };
  Object.defineProperty(local, 'extra', { enumerable: true, get() { assert.fail('extra schema property was read'); } });
  const connection = await createConnection(local, transport);
  connection.close();
});

test('four flows preserve bigint/bytes; RPC completion differs from notification acceptance', async () => {
  const transport = new FakeTransport();
  const handler = deferred<typeof item>();
  const callback = deferred<void>();
  const called = deferred<void>();
  const connection = await createConnection(schema, transport, { setup(c) { c.handle(label, () => handler.promise); } });
  connection.subscribe(changed, async value => { assert.deepEqual(value, item); called.resolve(); await callback.promise; });
  let saved = false;
  const saving = connection.call(save, item).then(value => { assert.equal(value, undefined); saved = true; });
  const notifying = connection.notify(selected, item);
  await turn();
  assert.equal(saved, false);
  assert.deepEqual(itemCodec.decode((transport.sent[0]!.body as { value: Uint8Array }).value), item);
  await transport.receive(frame(1n, 2n, Kind.ACCEPT));
  await notifying;
  assert.equal(saved, false);
  await transport.receive({ ...frame(1n, 1n, Kind.RESULT), body: { $case: 'payload', value: new Uint8Array() } });
  await saving;
  await transport.receive(invocation(1n, Kind.REQUEST, label.id));
  await transport.receive(invocation(2n, Kind.NOTIFY, changed.id));
  await called.promise;
  assert(transport.sent.some(f => f.id === 2n && f.kind === Kind.ACCEPT));
  assert(!transport.sent.some(f => f.id === 1n && f.kind === Kind.RESULT));
  handler.resolve(item);
  callback.resolve();
  await turn();
  assert(transport.sent.some(f => f.id === 1n && f.kind === Kind.RESULT));
  connection.close();
});

test('handler rejection becomes bounded Handler; early invalid values never send', async () => {
  const transport = new FakeTransport();
  const connection = await createConnection(schema, transport, { setup(c) { c.handle(label, async () => { throw new Error('secret stack'); }); } });
  await assert.rejects(connection.call(save, { ...item, id: 5 as unknown as bigint }), { code: 'invalid-payload' });
  assert.equal(transport.sent.length, 0);
  await transport.receive(invocation(1n, Kind.REQUEST, label.id));
  await turn();
  const error = transport.sent[0]!;
  assert.equal(error.kind, Kind.ERROR);
  assert(!JSON.stringify(error.body).includes('secret'));
  connection.close();
});

test('unknown methods reject; unknown and stale completions ignored; duplicates close', async () => {
  const transport = new FakeTransport();
  const connection = await createConnection(schema, transport);
  await transport.receive(frame(1n, 90n, Kind.ACCEPT));
  await transport.receive(frame(2n, 91n, Kind.ACCEPT));
  assert.equal(connection.stats.staleReplies, 2);
  await transport.receive(invocation(1n, Kind.REQUEST, 9000));
  await turn();
  assert.equal(transport.sent[0]!.kind, Kind.ERROR);
  await transport.receive(invocation(1n, Kind.REQUEST, 9000));
  assert.equal((await connection.closed).code, 'invalid-frame');
});

test('wrong completion kind closes and settles local RPC exactly once', async () => {
  const transport = new FakeTransport();
  const connection = await createConnection(schema, transport);
  const pending = connection.call(save, item);
  const rejected = assert.rejects(pending, { code: 'invalid-frame' });
  await transport.receive(frame(1n, 1n, Kind.ACCEPT));
  await rejected;
  connection.close();
});

test('abort settles locally, queued calls are suppressed and sent calls send CANCEL', async () => {
  const transport = new FakeTransport();
  const gate = deferred<void>();
  transport.accept = () => gate.promise;
  const connection = await createConnection(schema, transport);
  const firstAbort = new AbortController();
  const first = connection.call(save, item, { signal: firstAbort.signal });
  const secondAbort = new AbortController();
  const second = connection.call(save, item, { signal: secondAbort.signal });
  const a = assert.rejects(first, { code: 'cancelled' });
  const b = assert.rejects(second, { code: 'cancelled' });
  firstAbort.abort(); secondAbort.abort();
  await Promise.all([a, b]);
  gate.resolve();
  await turn();
  assert.deepEqual(transport.sent.map(f => f.kind), [Kind.REQUEST, Kind.CANCEL]);
  connection.close();
});

test('timeout includes local queue time and transmits remaining monotonic budget', async t => {
  t.mock.timers.enable({ apis: ['setTimeout'] });
  let now = 10;
  t.mock.method(performance, 'now', () => now);
  const transport = new FakeTransport();
  const gate = deferred<void>();
  transport.accept = () => gate.promise;
  const connection = await createConnection(schema, transport);
  const a = connection.call(save, item, { timeoutMs: 100 });
  const b = connection.call(save, item, { timeoutMs: 10 });
  const rejected = assert.rejects(b, { code: 'deadline-exceeded' });
  now += 11;
  t.mock.timers.tick(11);
  await rejected;
  gate.resolve();
  await turn();
  assert.equal(transport.sent.length, 1);
  const remaining = connection.call(save, item, { timeoutMs: 100 });
  await turn();
  assert.equal(transport.sent[1]!.timeoutMs, 100);
  const closedA = assert.rejects(a, { code: 'closed' });
  const closedB = assert.rejects(remaining, { code: 'closed' });
  connection.close();
  await Promise.all([closedA, closedB]);
});

test('navigation rejects waits, aborts handlers, removes subscribers and rejects reuse', async () => {
  const transport = new FakeTransport();
  const started = deferred<AbortSignal>();
  const handler = deferred<typeof item>();
  let callbacks = 0;
  const connection = await createConnection(schema, transport, { setup(c) {
    c.handle(label, (_value, context) => { started.resolve(context.signal); return handler.promise; });
  } });
  connection.subscribe(changed, () => { callbacks++; });
  await transport.receive(invocation(1n, Kind.REQUEST, label.id));
  const signal = await started.promise;
  const pending = connection.call(save, item);
  const rejected = assert.rejects(pending, { code: 'navigated' });
  transport.receiver.closed(new IpcError('navigated'));
  await rejected;
  assert.equal(signal.aborted, true);
  handler.resolve(item);
  await transport.receive(invocation(2n, Kind.NOTIFY, changed.id));
  await turn();
  assert.equal(callbacks, 0);
  assert.equal((await connection.closed).code, 'navigated');
  await assert.rejects(connection.call(save, item), { code: 'navigated' });
});

test('notification subscribers are ordered; close skips queued work; errors call onError', async () => {
  const transport = new FakeTransport();
  const gate = deferred<void>();
  const errors: IpcError[] = [];
  const seen: number[] = [];
  const connection = await createConnection(schema, transport, { onError: error => errors.push(error) });
  const subscription = connection.subscribe(changed, async () => { seen.push(1); await gate.promise; throw new Error('private'); });
  await transport.receive(invocation(1n, Kind.NOTIFY, changed.id));
  await transport.receive(invocation(2n, Kind.NOTIFY, changed.id));
  await turn();
  assert.equal(seen.length, 1);
  subscription.close(); subscription.close();
  gate.resolve();
  await turn();
  assert.equal(seen.length, 1);
  assert.equal(errors[0]!.code, 'handler');
  connection.close();
});

test('bounded pending and data queue reserve completion controls under saturation', async () => {
  const transport = new FakeTransport();
  transport.limits.maxPendingCallsPerDirection = 1;
  transport.limits.maxQueuedFramesPerDirection = 1;
  const gate = deferred<void>();
  transport.accept = () => gate.promise;
  const connection = await createConnection(schema, transport);
  const a = connection.call(save, item);
  await assert.rejects(connection.call(save, item), { code: 'overloaded' });
  await transport.receive(invocation(1n, Kind.NOTIFY, changed.id));
  gate.resolve();
  await turn();
  assert.deepEqual(transport.sent.map(f => f.kind), [Kind.REQUEST, Kind.ACCEPT]);
  const rejected = assert.rejects(a, { code: 'closed' });
  connection.close();
  await rejected;
});

test('schema mismatch and failed transport admission close once without reconnect', async () => {
  const transport = new FakeTransport();
  transport.start = async () => { throw new IpcError('schema-mismatch'); };
  await assert.rejects(createConnection(schema, transport), { code: 'schema-mismatch' });
  assert.equal(transport.closed, true);
});

test('generated runtime seam registers handlers before start and atomically rejects duplicates', async () => {
  const transport = new FakeTransport();
  const definitions = schema.methods.map(m => ({ ...m, name: String(m.id), developmentOnly: false }));
  const handlers = new Map([[label.id, () => item]]);
  const connection = await connect(transport, { hello: schema, methods: definitions }, { handlers });
  assert.throws(() => connection.register(handlers), { code: 'invalid-payload' });
  const pending = connection.call<void>(save.id, item);
  await transport.receive({ ...frame(1n, 1n, Kind.RESULT), body: { $case: 'payload', value: new Uint8Array() } });
  await pending;
  connection.close();
});

test('notification acceptance deadline rejects without pretending to retract delivery', async t => {
  t.mock.timers.enable({ apis: ['setTimeout'] });
  const transport = new FakeTransport();
  transport.limits.notificationAcceptTimeoutMs = 10;
  const connection = await createConnection(schema, transport);
  const waiting = assert.rejects(connection.notify(selected, item), { code: 'deadline-exceeded' });
  t.mock.timers.tick(10);
  await waiting;
  await transport.receive(frame(1n, 1n, Kind.ACCEPT));
  await turn();
  assert.equal(connection.stats.staleReplies, 1);
  assert.deepEqual(transport.sent.map(f => f.kind), [Kind.NOTIFY]);
  connection.close();
});

test('notification fanout is reserved atomically and callback slots release after close', async () => {
  const transport = new FakeTransport();
  transport.limits.maxCallbacksPerEvent = 2;
  transport.limits.maxCallbackTasksPerDocument = 2;
  const gate = deferred<void>();
  const connection = await createConnection(schema, transport);
  const a = connection.subscribe(changed, () => gate.promise);
  const b = connection.subscribe(changed, () => gate.promise);
  assert.throws(() => connection.subscribe(changed, () => {}), { code: 'overloaded' });
  await transport.receive(invocation(1n, Kind.NOTIFY, changed.id));
  await transport.receive(invocation(2n, Kind.NOTIFY, changed.id));
  await turn();
  assert.deepEqual(transport.sent.map(f => f.kind), [Kind.ACCEPT, Kind.ERROR]);
  a.close(); b.close();
  gate.resolve();
  await turn();
  const replacement = connection.subscribe(changed, () => {});
  await transport.receive(invocation(3n, Kind.NOTIFY, changed.id));
  await turn();
  assert.equal(transport.sent[2]!.kind, Kind.ACCEPT);
  replacement.close();
  connection.close();
});

test('deadline or cancel while a handler result is queued suppresses its late RESULT', async t => {
  t.mock.timers.enable({ apis: ['setTimeout'] });
  const transport = new FakeTransport();
  const gate = deferred<void>();
  transport.accept = () => gate.promise;
  const connection = await createConnection(schema, transport, { setup(c) { c.handle(label, () => item); } });
  const local = connection.call(save, item);
  const rejected = assert.rejects(local, { code: 'closed' });
  await transport.receive({ ...invocation(1n, Kind.REQUEST, label.id), timeoutMs: 10 });
  await turn();
  t.mock.timers.tick(10);
  gate.resolve();
  await turn();
  assert.deepEqual(transport.sent.map(f => f.kind), [Kind.REQUEST, Kind.ERROR]);
  connection.close();
  await rejected;
});

test('cancellation releases unsent queue slots immediately behind a blocked POST', async () => {
  const transport = new FakeTransport();
  transport.limits.maxQueuedFramesPerDirection = 2;
  const gate = deferred<void>();
  transport.accept = () => gate.promise;
  const connection = await createConnection(schema, transport);
  const first = connection.call(save, item);
  const controller = new AbortController();
  const second = connection.call(save, item, { signal: controller.signal });
  const cancelled = assert.rejects(second, { code: 'cancelled' });
  controller.abort();
  await cancelled;
  const third = connection.call(save, item);
  gate.resolve();
  await turn();
  assert.deepEqual(transport.sent.map(f => f.id), [1n, 3n]);
  const one = assert.rejects(first, { code: 'closed' });
  const three = assert.rejects(third, { code: 'closed' });
  connection.close();
  await Promise.all([one, three]);
});

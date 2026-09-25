// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import test from 'node:test';
import assert from 'node:assert/strict';
import { ByteLedger, hasLedger } from '../src/budget.js';
import { createConnection, defaultLimits, createDesktopTransport, type BinaryReceiver, type NativeIpcBootstrap, type IpcTransport } from '../src/index.js';
import { FakeTransport, schema, save, selected, label, changed, item, itemCodec, invocation, frame, IpcFrame, Kind, deferred, turn } from './helpers.js';

test('input permits charge capacity, validate ownership and do not reset on task cancellation', async () => {
  const transport = new FakeTransport();
  transport.limits.maxFrameBytes = 4096;
  transport.limits.maxAdmittedInputBytesPerFrame = 4096;
  const blocked = deferred<typeof item>();
  let called = 0;
  const connection = await createConnection(schema, transport, { setup(c) {
    c.handle(label, () => { called++; return blocked.promise; });
  } });
  const payload = itemCodec.encode({ ...item, data: new Uint8Array(3000) });
  const request = (id: bigint) => ({ ...invocation(id, Kind.REQUEST, label.id), body: { $case: 'payload' as const, value: payload } });
  await transport.receive(request(1n));
  await turn();
  await transport.receive(frame(1n, 1n, Kind.CANCEL));
  await transport.receive(request(2n));
  await turn();
  assert.equal(called, 1);
  assert.equal(transport.sent[0]!.body?.$case, 'error');
  assert.equal((transport.sent[0]!.body as { value: { code: string } }).value.code, 'overloaded');
  blocked.resolve(item);
  await turn();
  await transport.receive(request(3n));
  await turn();
  assert.equal(called, 2);
  connection.close();
});

test('notification shared input credit lasts through every subscriber, including closed running callbacks', async () => {
  const transport = new FakeTransport();
  transport.limits.maxFrameBytes = 4096;
  transport.limits.maxAdmittedInputBytesPerFrame = 4096;
  const blocked = deferred<void>();
  const connection = await createConnection(schema, transport);
  const a = connection.subscribe(changed, () => blocked.promise);
  connection.subscribe(changed, () => {});
  const notification = (id: bigint) => ({
    ...invocation(id, Kind.NOTIFY, changed.id),
    body: { $case: 'payload' as const, value: itemCodec.encode({ ...item, data: new Uint8Array(3000) }) },
  });
  await transport.receive(notification(1n));
  await turn();
  a.close();
  await transport.receive(notification(2n));
  await turn();
  assert.deepEqual(transport.sent.map(f => f.kind), [Kind.ACCEPT, Kind.ERROR]);
  blocked.resolve();
  await turn();
  await transport.receive(notification(3n));
  await turn();
  assert.equal(transport.sent[2]!.kind, Kind.ACCEPT);
  connection.close();
});

test('ledger rejects over-budget growth atomically and releases idempotently', () => {
  const limits = { ...defaultLimits, maxAdmittedInputBytesPerFrame: 10, maxRetainedBytesPerFrame: 16 };
  const ledger = new ByteLedger(limits);
  const credit = ledger.reserveInput(8);
  assert.equal(credit.owns(new Uint8Array(8), ledger), true);
  assert.equal(credit.owns(new Uint8Array(8), new ByteLedger(limits)), false);
  const release = ledger.reserve(8);
  assert.throws(() => credit.grow(1), { code: 'overloaded' });
  assert.equal(ledger.input, 8);
  release();
  credit.grow(2);
  assert.throws(() => credit.grow(1), { code: 'overloaded' });
  credit.release(); credit.release(); release();
  assert.equal(ledger.retained, 0);
  assert.equal(ledger.input, 0);
  assert.equal(credit.owns(new Uint8Array(1), ledger), false);
});

test('native streaming allocation reserves credit before growth and releases on over-budget failure', async () => {
  for (const size of [0, 3000]) {
    let control!: (value: { kind: 'ready'; generation: string }) => void;
    const limits = { ...defaultLimits, maxFrameBytes: 4096, maxAdmittedInputBytesPerFrame: 4096, maxRetainedBytesPerFrame: 8192 };
    const ledger = new ByteLedger(limits);
    const occupying = ledger.reserveInput(4096);
    const native: NativeIpcBootstrap = {
      documentNonce: 'a'.repeat(32), activate: () => true,
      async hello() { return { generation: '1', token: 'b'.repeat(32), limits }; },
      subscribeControl(callback) { control = callback; return { close() {} }; },
      disconnect() {},
    };
    const failed = deferred<unknown>();
    const receiver: BinaryReceiver & { byteLedger: ByteLedger; receiveReserved(): Promise<void> } = {
      byteLedger: ledger, async receiveReserved() { assert.fail('over-budget data dispatched'); },
      async receive() { assert.fail('over-budget data dispatched'); }, closed: failed.resolve,
    };
    const body = IpcFrame.encode({
      ...invocation(1n, Kind.REQUEST, label.id),
      body: { $case: 'payload', value: itemCodec.encode({ ...item, data: new Uint8Array(size) }) },
    }).finish();
    const transport = createDesktopTransport({ bootstrap: native, fetch: (async () => new Response(body, {
      headers: { 'Content-Type': 'application/x-protobuf', 'Content-Length': String(body.byteLength) },
    })) as typeof fetch });
    await transport.start(schema, receiver);
    control({ kind: 'ready', generation: '1' });
    assert.equal((await failed.promise as { code: string }).code, 'overloaded');
    assert.equal(ledger.input, 4096);
    occupying.release();
    assert.equal(ledger.retained, 0);
  }
});

test('real transport delivers CANCEL, ERROR and ACCEPT while application input credit is exhausted', async t => {
  let control!: (value: { kind: 'ready'; generation: string }) => void;
  const limits = { ...defaultLimits, maxFrameBytes: 4096, maxAdmittedInputBytesPerFrame: 4096 };
  const native: NativeIpcBootstrap = {
    documentNonce: 'a'.repeat(32), activate: () => true,
    async hello() { return { generation: '1', token: 'b'.repeat(32), limits }; },
    subscribeControl(callback) { control = callback; return { close() {} }; },
    disconnect() {},
  };
  const outbound: IpcFrame[] = [{
    ...invocation(1n, Kind.REQUEST, label.id),
    body: { $case: 'payload', value: itemCodec.encode({ ...item, data: new Uint8Array(3000) }) },
  }];
  const empty = deferred<void>();
  let gets = 0;
  const transport = createDesktopTransport({
    bootstrap: native,
    fetch: (async (_path, init) => {
      if (init?.method === 'POST') return new Response(null, { status: 204 });
      gets++;
      const next = outbound.shift();
      if (!next) { empty.resolve(); return new Response(null, { status: 204 }); }
      // No Content-Length: the 3000-byte request retains a 4096-byte buffer.
      const headers: Record<string, string> = { 'Content-Type': 'application/x-protobuf' };
      if (next.kind === Kind.ACCEPT) headers['Content-Length'] = '1'; // A false small length must not consume data credit.
      return new Response(IpcFrame.encode(next).finish(), { headers });
    }) as typeof fetch,
  });
  let ledger: ByteLedger | undefined;
  const tracked: IpcTransport = { ...transport, start(hello, receiver) {
    assert(hasLedger(receiver));
    ledger = receiver.byteLedger;
    return transport.start(hello, receiver);
  } };
  const started = deferred<AbortSignal>();
  const blocked = deferred<typeof item>();
  const connection = await createConnection(schema, tracked, { setup(connection) {
    connection.handle(label, (_value, context) => { started.resolve(context.signal); return blocked.promise; });
  } });
  t.after(() => { blocked.resolve(item); connection.close(); });
  let closed = false;
  void connection.closed.then(() => { closed = true; });
  control({ kind: 'ready', generation: '1' });
  const signal = await started.promise;
  await empty.promise;
  assert(ledger);
  assert.equal(ledger.input, 4096);
  const rejected = assert.rejects(connection.call(save, item), { code: 'handler' });
  const notified = connection.notify(selected, item);
  await turn();
  outbound.push(
    frame(1n, 1n, Kind.CANCEL),
    { ...frame(1n, 1n, Kind.ERROR), body: { $case: 'error', value: {
      code: 'handler', message: 'handler', help: 'test', applicationCode: '',
    } } },
    frame(1n, 2n, Kind.ACCEPT),
  );
  control({ kind: 'ready', generation: '1' });
  await Promise.all([rejected, notified]);
  await turn();
  assert.equal(signal.aborted, true);
  assert.equal(closed, false);
  assert.equal(ledger.input, 4096, 'cancelled running handler must retain its input credit');
  const idleGets = gets;
  await turn();
  assert.equal(gets, idleGets);
  blocked.resolve(item);
  await turn();
  assert.equal(ledger.input, 0);
});

test('one real-transport wake drains a 3000-byte REQUEST then CANCEL without Content-Length', async t => {
  let control!: (value: { kind: 'ready'; generation: string }) => void;
  const limits = { ...defaultLimits, maxFrameBytes: 4096, maxAdmittedInputBytesPerFrame: 4096 };
  const native: NativeIpcBootstrap = {
    documentNonce: 'a'.repeat(32), activate: () => true,
    async hello() { return { generation: '1', token: 'b'.repeat(32), limits }; },
    subscribeControl(callback) { control = callback; return { close() {} }; },
    disconnect() {},
  };
  const outbound: IpcFrame[] = [
    { ...invocation(1n, Kind.REQUEST, label.id), body: {
      $case: 'payload', value: itemCodec.encode({ ...item, data: new Uint8Array(3000) }),
    } },
    frame(1n, 1n, Kind.CANCEL),
  ];
  const drained = deferred<void>();
  let gets = 0;
  const transport = createDesktopTransport({ bootstrap: native, fetch: (async (_path, init) => {
    assert.equal(init?.method, 'GET');
    gets++;
    const next = outbound.shift();
    if (!next) { drained.resolve(); return new Response(null, { status: 204 }); }
    const response = new Response(IpcFrame.encode(next).finish(), { headers: { 'Content-Type': 'application/x-protobuf' } });
    assert.equal(response.headers.has('Content-Length'), false);
    return response;
  }) as typeof fetch });
  let ledger: ByteLedger | undefined;
  const tracked: IpcTransport = { ...transport, start(hello, receiver) {
    assert(hasLedger(receiver));
    ledger = receiver.byteLedger;
    return transport.start(hello, receiver);
  } };
  const blocked = deferred<typeof item>();
  let signal: AbortSignal | undefined;
  const connection = await createConnection(schema, tracked, { setup(connection) {
    connection.handle(label, (request, context) => {
      assert.equal(request.data.byteLength, 3000);
      signal = context.signal;
      return blocked.promise;
    });
  } });
  t.after(() => { blocked.resolve(item); connection.close(); });
  let closed = false;
  void connection.closed.then(() => { closed = true; });
  control({ kind: 'ready', generation: '1' }); // Exactly one wake for both frames.
  await Promise.race([drained.promise, connection.closed.then(error => { throw error; })]);
  await turn();
  assert.equal(gets, 3, 'REQUEST, CANCEL, then terminal 204');
  assert.equal(signal?.aborted, true);
  assert.equal(closed, false);
  assert(ledger);
  assert.equal(ledger.input, 4096, 'running cancelled handler retains its actual buffer capacity');
  blocked.resolve(item);
  await turn();
  assert.equal(ledger.input, 0);
  assert.equal(gets, 3, 'no idle polling');
});

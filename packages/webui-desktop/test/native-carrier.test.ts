// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import test from 'node:test';
import assert from 'node:assert/strict';
import { createNativeBootstrap, type NativeChannel } from '../src/bootstrap.js';
import { ByteLedger } from '../src/budget.js';
import { NativeFrameCarrier } from '../src/native-carrier.js';
import { createNativeDesktopTransport as createDesktopTransport } from '../src/local-index.js';
import { defaultLimits, type NativeIpcBootstrap } from '../src/index.js';
import type { NativeDataLane } from '../src/types.js';
import { deferred, frame, IpcFrame, Kind, schema, turn } from './helpers.js';

const session = { generation: '1', token: 'f'.repeat(32), limits: defaultLimits, nativeCarrierVersion: 1 as const };
const payload = (size: number) => IpcFrame.encode({
  ...frame(1n, 1n, Kind.RESULT), body: { $case: 'payload' as const, value: new Uint8Array(size) },
}).finish();

function dataLane(replyCapable: boolean) {
  let listener: ((raw: unknown) => void) | undefined;
  const messages: Record<string, unknown>[] = [];
  let responder: (message: Record<string, unknown>) => unknown = () => undefined;
  const lane: NativeDataLane = {
    subscribe(callback) {
      assert.equal(listener, undefined);
      listener = callback;
      return { close() { if (listener === callback) listener = undefined; } };
    },
    postMessage(message) {
      messages.push(message);
      const answer = responder(message);
      if (replyCapable) return answer;
      if (answer !== undefined) queueMicrotask(() => listener?.(answer));
      return undefined;
    },
  };
  return { lane, messages, respond(fn: typeof responder) { responder = fn; }, push(raw: unknown) { listener?.(raw); }, listening: () => !!listener };
}
function result(message: Record<string, unknown>, fields: Record<string, unknown>) {
  return { kind: 'ipcDataResult', version: 1, callId: message.callId, generation: message.generation,
    operation: message.operation, ...fields };
}

test('native capability remains invisible until activation and exact carrier negotiation', async () => {
  for (const carrierReply of [undefined, 2, 1]) {
    const lane = dataLane(true);
    const sent: Record<string, unknown>[] = [];
    const control: NativeChannel = {
      subscribe: () => ({ close() {} }),
      postMessage(message) {
        sent.push(message);
        if (message.kind !== 'hello') return undefined;
        return { kind: 'helloResult', callId: '1', navigation: '1',
          documentNonce: bootstrap.documentNonce, challenge: 'b'.repeat(32),
          generation: '1', token: session.token, limits: defaultLimits,
          ...(carrierReply === undefined ? {} : { nativeCarrierVersion: carrierReply }) };
      },
    };
    const bootstrap = createNativeBootstrap(control, lane.lane);
    assert.equal(bootstrap.nativeData, undefined);
    const admitted = bootstrap.hello(schema);
    assert.equal(bootstrap.activate({ navigation: '1', documentNonce: bootstrap.documentNonce,
      challenge: 'b'.repeat(32), nativeCarrierVersion: 1 }), true);
    if (carrierReply !== 1) {
      await assert.rejects(admitted, { code: 'unsupported-version' });
      assert.equal(bootstrap.nativeData, undefined);
      assert.equal(sent[1]?.kind, 'disconnect');
    } else {
      assert.equal((await admitted).nativeCarrierVersion, 1);
      assert.equal(bootstrap.nativeData, lane.lane);
      assert.equal(sent[0]?.nativeCarrierVersion, 1);
      bootstrap.disconnect(session.generation, session.token);
      assert.equal(bootstrap.nativeData, undefined);
    }
  }
});

test('legacy admission does not select native data even if a lane exists', async () => {
  const lane = dataLane(false);
  let push!: (value: unknown) => void;
  const bootstrap = createNativeBootstrap({
    subscribe(callback) { push = callback; return { close() {} }; },
    postMessage() { queueMicrotask(() => push({ kind: 'helloResult', callId: '1', navigation: '1',
      documentNonce: bootstrap.documentNonce, challenge: 'b'.repeat(32),
      generation: '1', token: session.token, limits: defaultLimits })); },
  }, lane.lane);
  const admitted = bootstrap.hello(schema);
  bootstrap.activate({ navigation: '1', documentNonce: bootstrap.documentNonce, challenge: 'b'.repeat(32) });
  assert.equal((await admitted).nativeCarrierVersion, undefined);
  assert.equal(bootstrap.nativeData, undefined);
  bootstrap.disconnect('1', session.token);
});

test('both reply modes chunk binary frames and reassemble with bounded credits', async () => {
  for (const replies of [false, true]) {
    const data = dataLane(replies);
    const ledger = new ByteLedger(defaultLimits);
    const carrier = new NativeFrameCarrier(data.lane, session, ledger);
    const bytes = payload(70 * 1024);
    let inputOffset = 0;
    let outputOffset = 0;
    const reassembled = new Uint8Array(bytes.length);
    data.respond(message => {
      assert.equal(message.token, session.token);
      assert.equal(message.version, 1);
      if (message.operation === 'send') {
        const chunk = Uint8Array.from(atob(message.data as string), c => c.charCodeAt(0));
        assert(chunk.length <= 24 * 1024);
        assert.equal(message.offset, inputOffset);
        reassembled.set(chunk, inputOffset);
        inputOffset += chunk.length;
        return result(message, { nextOffset: inputOffset, complete: inputOffset === bytes.length });
      }
      const chunk = bytes.subarray(outputOffset, outputOffset + (message.maxBytes as number));
      const current = outputOffset;
      outputOffset += chunk.length;
      return result(message, { totalBytes: bytes.length, offset: current,
        nextOffset: outputOffset, complete: outputOffset === bytes.length,
        data: btoa(String.fromCharCode(...chunk)) });
    });
    await carrier.send(bytes);
    const received = await carrier.read();
    assert.deepEqual(received?.bytes, bytes);
    assert.deepEqual(reassembled, bytes);
    assert(received?.credit);
    assert.equal(ledger.input, bytes.length);
    assert.equal(ledger.retained, bytes.length);
    received?.credit.release();
    assert.equal(ledger.retained, 0);
    assert.equal(ledger.input, 0);
    assert.equal(data.messages.length, 7);
    carrier.close();
    assert.equal(data.listening(), false);
  }
});

test('native selection never fetches local HTTP and stale data cannot reopen after close', async () => {
  const data = dataLane(false);
  data.respond(message => result(message, { nextOffset: message.totalBytes, complete: true }));
  const disconnected: unknown[] = [];
  let control!: (value: { kind: 'closed'; generation: string; code: 'navigated' }) => void;
  const bootstrap: NativeIpcBootstrap = {
    documentNonce: 'a'.repeat(32), activate: () => true,
    hello: async () => session, nativeData: data.lane,
    subscribeControl(listener) { control = listener; return { close() {} }; },
    disconnect(generation, token) { disconnected.push({ generation, token }); },
  };
  const transport = createDesktopTransport({ bootstrap, fetch: (async () => assert.fail('local HTTP IPC request')) as typeof fetch });
  const closed = deferred<unknown>();
  await transport.start(schema, { async receive() {}, closed: closed.resolve });
  await transport.send(payload(0));
  control({ kind: 'closed', generation: '1', code: 'navigated' });
  assert.equal((await closed.promise as { code: string }).code, 'navigated');
  assert.deepEqual(disconnected, [{ generation: '1', token: session.token }]);
  assert.equal(data.listening(), false);
  await assert.rejects(transport.send(payload(0)), { code: 'navigated' });
});

test('a native overload leaves the session usable for a later send', async () => {
  const data = dataLane(true);
  let sends = 0;
  data.respond(message => result(message, ++sends === 1
    ? { error: { code: 'overloaded' } }
    : { nextOffset: message.totalBytes, complete: true }));
  const disconnected: unknown[] = [];
  const bootstrap: NativeIpcBootstrap = {
    documentNonce: 'a'.repeat(32), activate: () => true,
    hello: async () => session, nativeData: data.lane,
    subscribeControl() { return { close() {} }; },
    disconnect(generation, token) { disconnected.push({ generation, token }); },
  };
  const transport = createDesktopTransport({ bootstrap,
    fetch: (async () => assert.fail('native overload fetched HTTP')) as typeof fetch });
  await transport.start(schema, { async receive() {}, closed() {} });
  await assert.rejects(transport.send(payload(0)), { code: 'overloaded' });
  assert.deepEqual(disconnected, []);
  await transport.send(payload(0));
  assert.equal(sends, 2);
  transport.close();
  assert.deepEqual(disconnected, [{ generation: '1', token: session.token }]);
});

test('missing or mismatched native carrier never sends a token to local HTTP', async () => {
  for (const nativeCarrierVersion of [undefined, 2]) {
    const data = dataLane(false);
    const disconnected: unknown[] = [];
    const bootstrap: NativeIpcBootstrap = {
      documentNonce: 'a'.repeat(32), activate: () => true,
      hello: async () => nativeCarrierVersion === undefined
        ? { generation: session.generation, token: session.token, limits: session.limits }
        : { ...session, nativeCarrierVersion: nativeCarrierVersion as 1 },
      nativeData: data.lane,
      subscribeControl() { return { close() {} }; },
      disconnect(generation, token) { disconnected.push({ generation, token }); },
    };
    const transport = createDesktopTransport({ bootstrap,
      fetch: (async () => assert.fail('unnegotiated carrier fetched HTTP')) as typeof fetch });
    await assert.rejects(transport.start(schema, { async receive() {}, closed() {} }), { code: 'unsupported-version' });
    assert.equal(data.listening(), false);
    assert.deepEqual(disconnected, [{ generation: '1', token: session.token }]);
  }
});

test('native ready drains frames through the existing receiver, then stops on empty without polling', async () => {
  const data = dataLane(true);
  const bytes = IpcFrame.encode(frame(1n, 1n, Kind.ACCEPT)).finish();
  let reads = 0;
  data.respond(message => {
    assert.equal(message.operation, 'receive');
    reads++;
    return result(message, reads === 1 ? {
      totalBytes: bytes.length, offset: 0, nextOffset: bytes.length, complete: true,
      data: btoa(String.fromCharCode(...bytes)),
    } : { empty: true });
  });

  let control!: (value: { kind: 'ready'; generation: string }) => void;
  const bootstrap: NativeIpcBootstrap = {
    documentNonce: 'a'.repeat(32), activate: () => true,
    hello: async () => session, nativeData: data.lane,
    subscribeControl(listener) { control = listener; return { close() {} }; },
    disconnect() {},
  };
  const received: Uint8Array[] = [];
  const transport = createDesktopTransport({ bootstrap,
    fetch: (async () => assert.fail('HTTP data lane')) as typeof fetch });
  await transport.start(schema, { async receive(value) { received.push(value); }, closed() {} });
  control({ kind: 'ready', generation: '1' });
  await turn();
  assert.deepEqual(received, [bytes]);
  assert.equal(reads, 2);
  await turn();
  assert.equal(reads, 2);
  transport.close();
});

test('small control frames drain without application input credit under saturation', async () => {
  const data = dataLane(true);
  const bytes = IpcFrame.encode(frame(1n, 1n, Kind.CANCEL)).finish();
  data.respond(message => {
    assert((message.maxBytes as number) >= bytes.length);
    return result(message, { totalBytes: bytes.length, offset: 0, nextOffset: bytes.length,
      complete: true, data: btoa(String.fromCharCode(...bytes)) });
  });
  const ledger = new ByteLedger(defaultLimits);
  const occupied = ledger.reserveInput(defaultLimits.maxAdmittedInputBytesPerFrame);
  const carrier = new NativeFrameCarrier(data.lane, session, ledger);
  const received = await carrier.read();
  assert.deepEqual(received?.bytes, bytes);
  assert.equal(received?.credit, undefined);
  assert.equal(ledger.input, defaultLimits.maxAdmittedInputBytesPerFrame);
  carrier.close();
  occupied.release();
  assert.equal(ledger.retained, 0);
});

test('malformed size, wrong ACK and unresponsive native callback fail closed without retained bytes', async t => {
  const bytes = payload(0);
  for (const malformed of [
    () => null,
    (message: Record<string, unknown>) => result(message, { empty: true, totalBytes: 1 }),
    (message: Record<string, unknown>) => result(message, { empty: false }),
    (message: Record<string, unknown>) => result(message, { callId: 'wrong' }),
    (message: Record<string, unknown>) => result(message, { data: 'A'.repeat(32769), totalBytes: bytes.length, offset: 0 }),
    (message: Record<string, unknown>) => result(message, { data: btoa(String.fromCharCode(...bytes)),
      totalBytes: bytes.length, offset: 0, nextOffset: 1, complete: true }),
    (message: Record<string, unknown>) => result(message, { data: 'AAAA', totalBytes: defaultLimits.maxFrameBytes + 1, offset: 0 }),
  ]) {
    const data = dataLane(true);
    const ledger = new ByteLedger(defaultLimits);
    data.respond(malformed);
    const carrier = new NativeFrameCarrier(data.lane, session, ledger);
    await assert.rejects(carrier.read());
    assert.equal(ledger.retained, 0);
    assert.equal(ledger.input, 0);
    carrier.close();
  }
  t.mock.timers.enable({ apis: ['setTimeout'] });
  const data = dataLane(false);
  const carrier = new NativeFrameCarrier(data.lane, session, new ByteLedger(defaultLimits));
  const pending = assert.rejects(carrier.read(), { code: 'deadline-exceeded' });
  await turn();
  t.mock.timers.tick(5000);
  await pending;
  carrier.close();
  assert.equal(data.listening(), false);
});

test('close rejects pending native reply and a late correlated message cannot resume a cursor', async () => {
  const data = dataLane(false);
  const carrier = new NativeFrameCarrier(data.lane, session, new ByteLedger(defaultLimits));
  const pending = assert.rejects(carrier.send(payload(0)), { code: 'navigated' });
  await turn();
  assert.equal(data.messages.length, 1);
  carrier.close(new (await import('../src/errors.js')).IpcError('navigated'));
  await pending;
  data.push(result(data.messages[0]!, { nextOffset: payload(0).length, complete: true }));
  await assert.rejects(carrier.read(), { code: 'navigated' });
  assert.equal(data.listening(), false);
});

test('a late WK reply after deadline does not reject the next native call', async t => {
  t.mock.timers.enable({ apis: ['setTimeout'] });
  const data = dataLane(true);
  const first = deferred<unknown>();
  const second = deferred<unknown>();
  let calls = 0;
  data.respond(() => (++calls === 1 ? first.promise : second.promise));
  const carrier = new NativeFrameCarrier(data.lane, session, new ByteLedger(defaultLimits));
  const expired = assert.rejects(carrier.send(payload(0)), { code: 'deadline-exceeded' });
  await turn();
  t.mock.timers.tick(5000);
  await expired;
  const next = carrier.send(payload(0));
  await turn();
  assert.equal(data.messages.length, 2);
  first.resolve(result(data.messages[0]!, { nextOffset: payload(0).length, complete: true }));
  await turn();
  second.resolve(result(data.messages[1]!, { nextOffset: payload(0).length, complete: true }));
  await next;
  carrier.close();
});

test('data lane does not accept a wrong version, generation or byte-acknowledgement', async () => {
  for (const change of [
    { version: 2 }, { generation: '2' }, { nextOffset: 0 },
  ]) {
    const data = dataLane(true);
    data.respond(message => ({ ...result(message, {
      nextOffset: message.totalBytes, complete: true,
    }), ...change }));
    const carrier = new NativeFrameCarrier(data.lane, session, new ByteLedger(defaultLimits));
    await assert.rejects(carrier.send(payload(0)), { code: change.version ? 'unsupported-version' :
      change.generation ? 'unsupported-version' : 'invalid-frame' });
    carrier.close();
  }
});

// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import test from 'node:test';
import assert from 'node:assert/strict';
import { createDesktopTransport, defaultLimits, IpcError, type NativeControl, type NativeIpcBootstrap, type SessionInfo } from '../src/index.js';
import { deferred, frame, IpcFrame, Kind, schema, turn } from './helpers.js';

function native() {
  let listener: ((control: NativeControl) => void) | undefined;
  const admitted = deferred<SessionInfo>();
  const state = { listening: false, disconnected: [] as { generation: string; token: string }[], hello: 0 };
  const bootstrap: NativeIpcBootstrap = {
    documentNonce: 'b'.repeat(32),
    activate: () => true,
    hello() { assert(state.listening); state.hello++; return admitted.promise; },
    subscribeControl(callback) {
      assert(!listener);
      listener = callback;
      state.listening = true;
      return { close() { listener = undefined; state.listening = false; } };
    },
    disconnect(generation, token) { state.disconnected.push({ generation, token }); },
  };
  return { bootstrap, admitted, state, control(control: NativeControl) { listener?.(control); } };
}
const session = { generation: '1', token: 'f'.repeat(32), limits: defaultLimits };

test('transport projects direct callers to the exact four-field bootstrap Hello', async () => {
  const host = native();
  const hello = host.bootstrap.hello.bind(host.bootstrap);
  host.bootstrap.hello = value => {
    assert.deepEqual(Reflect.ownKeys(value).sort(), ['contractMajor', 'contractName', 'schemaHash', 'wireVersion']);
    return hello(value);
  };
  const local = { ...schema, localCodec: () => {}, extra: 'not native data' };
  const transport = createDesktopTransport({ bootstrap: host.bootstrap });
  const started = transport.start(local, { async receive() {}, closed() {} });
  host.admitted.resolve(session);
  await started;
  transport.close();
});

test('listener before hello, coalesced handshake wake, final-204 race and no idle GETs/timers', async t => {
  const host = native();
  const first = deferred<Response>();
  const final = deferred<Response>();
  const received: Uint8Array[] = [];
  const requests: { path: string; init: RequestInit | undefined }[] = [];
  const fetcher = (async (path: string, init?: RequestInit) => {
    requests.push({ path, init });
    if (requests.length === 1) return first.promise;
    if (requests.length === 2) return final.promise;
    return new Response(null, { status: 204 });
  }) as typeof fetch;
  const timer = t.mock.method(globalThis, 'setTimeout', () => { throw new Error('idle timer'); });
  const transport = createDesktopTransport({ bootstrap: host.bootstrap, fetch: fetcher });
  const start = transport.start(schema, { async receive(bytes) { received.push(bytes); }, closed() {} });
  host.control({ kind: 'ready', generation: '1' });
  host.control({ kind: 'ready', generation: '1' });
  assert.equal(requests.length, 0);
  host.admitted.resolve(session);
  await start;
  assert.equal(requests.length, 1);
  first.resolve(new Response(IpcFrame.encode(frame(1n, 1n, Kind.ACCEPT)).finish(), { headers: { 'Content-Type': 'application/x-protobuf' } }));
  await turn();
  assert.equal(requests.length, 2);
  host.control({ kind: 'ready', generation: '1' });
  final.resolve(new Response(null, { status: 204 }));
  await turn();
  assert.equal(requests.length, 3);
  assert.equal(received.length, 1);
  await turn();
  assert.equal(requests.length, 3);
  assert.equal(timer.mock.callCount(), 0);
  for (const request of requests) {
    assert(!request.path.includes(session.token));
    assert.equal((request.init?.headers as Record<string, string>)['X-WebUI-Ipc-Session'], session.token);
    assert.equal(request.init?.cache, 'no-store');
  }
  transport.close(); transport.close();
  assert.deepEqual(host.state.disconnected, [{ generation: '1', token: session.token }]);
  assert.equal(host.state.listening, false);
});

test('POST is binary and resolves on ingress acceptance only', async () => {
  const host = native();
  const accepted = deferred<Response>();
  let seen: RequestInit | undefined;
  const transport = createDesktopTransport({ bootstrap: host.bootstrap, fetch: (async (_path, init) => { seen = init; return accepted.promise; }) as typeof fetch });
  const starting = transport.start(schema, { async receive() {}, closed() {} });
  host.admitted.resolve(session);
  await starting;
  const bytes = new Uint8Array([1, 2, 3]);
  const sending = transport.send(bytes);
  assert.equal(seen?.body, bytes);
  assert.equal(seen?.method, 'POST');
  await assert.rejects(transport.send(bytes), { code: 'overloaded' });
  accepted.resolve(new Response(null, { status: 204 }));
  await sending;
  transport.close();
});

test('application POSTs above 64 KiB do not opt into the browser keepalive quota', async () => {
  const host = native();
  const bytes = new Uint8Array(64 * 1024 + 1);
  const transport = createDesktopTransport({
    bootstrap: host.bootstrap,
    fetch: (async (_path, init) => {
      assert.equal(init?.method, 'POST');
      assert.equal(init?.body, bytes);
      assert.notEqual(init?.keepalive, true);
      return new Response(null, { status: 204 });
    }) as typeof fetch,
  });
  const starting = transport.start(schema, { async receive() {}, closed() {} });
  host.admitted.resolve(session);
  await starting;
  await transport.send(bytes);
  transport.close();
});

test('absent and failed hello are explicit; close during admission disconnects late session', async () => {
  const absent = createDesktopTransport();
  await assert.rejects(absent.start(schema, { async receive() {}, closed() {} }), { code: 'not-ready' });
  const host = native();
  const errors: IpcError[] = [];
  const transport = createDesktopTransport({ bootstrap: host.bootstrap });
  const pending = transport.start(schema, { async receive() {}, closed(error) { errors.push(error); } });
  const rejected = assert.rejects(pending, { code: 'closed' });
  transport.close();
  host.admitted.resolve(session);
  await rejected;
  assert.equal(errors.length, 1);
  assert.deepEqual(host.state.disconnected, [{ generation: '0', token: '' }, { generation: '1', token: session.token }]);
  const failedHost = native();
  const failed = createDesktopTransport({ bootstrap: failedHost.bootstrap });
  const failure = failed.start(schema, { async receive() {}, closed() {} });
  failedHost.admitted.reject(new IpcError('schema-mismatch'));
  await assert.rejects(failure, { code: 'schema-mismatch' });
  assert.equal(failedHost.state.listening, false);
});

test('actual streamed body size is capped without trusting Content-Length', async () => {
  const host = native();
  let closed!: IpcError;
  const transport = createDesktopTransport({
    bootstrap: host.bootstrap,
    fetch: (async () => new Response(new Uint8Array(defaultLimits.maxFrameBytes + 1), { headers: { 'Content-Type': 'application/x-protobuf', 'Content-Length': '1' } })) as typeof fetch,
  });
  const starting = transport.start(schema, { async receive() { assert.fail('oversize dispatched'); }, closed(error) { closed = error; } });
  host.admitted.resolve(session);
  await starting;
  host.control({ kind: 'ready', generation: '1' });
  await turn();
  assert.equal(closed.code, 'payload-too-large');
  assert.deepEqual(host.state.disconnected, [{ generation: '1', token: session.token }]);
});

test('native navigation rejects sends and stale controls cannot reopen the transport', async () => {
  const host = native();
  const closed = deferred<IpcError>();
  const transport = createDesktopTransport({ bootstrap: host.bootstrap });
  const starting = transport.start(schema, { async receive() {}, closed: closed.resolve });
  host.admitted.resolve(session);
  await starting;
  host.control({ kind: 'closed', generation: '1', code: 'navigated' });
  assert.equal((await closed.promise).code, 'navigated');
  await assert.rejects(transport.send(new Uint8Array()), { code: 'navigated' });
  host.control({ kind: 'ready', generation: '1' });
  assert.equal(host.state.listening, false);
});

test('transport retires an invalid admitted session using its exact credentials', async () => {
  const host = native();
  const transport = createDesktopTransport({ bootstrap: host.bootstrap });
  const started = transport.start(schema, { async receive() {}, closed() {} });
  const rejected = assert.rejects(started, { code: 'invalid-frame' });
  host.admitted.resolve({ ...session, limits: { ...defaultLimits, maxFrameBytes: 0 } });
  await rejected;
  assert.deepEqual(host.state.disconnected, [{ generation: '1', token: session.token }]);
});

// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import test from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { resolve } from 'node:path';
import { pathToFileURL } from 'node:url';
import { runInNewContext } from 'node:vm';
import { defaultLimits, type NativeIpcBootstrap, type IpcError } from '../src/index.js';
import { WireError } from '../src/envelope.js';
import { deferred, schema, save, item, invocation, IpcFrame, Kind, turn } from './helpers.js';

type Runtime = typeof import('../src/index.js');
type FailurePoint = 'post' | 'get' | 'body';

async function raceHarness(point: FailurePoint) {
  const runtime = await import(pathToFileURL(resolve('dist/desktop-runtime.js')).href) as Runtime;
  const request = deferred<Response>();
  const reading = deferred<void>();
  const pagehide: ((event: { isTrusted: boolean; persisted: boolean }) => void)[] = [];
  const order: string[] = [];
  const requests: RequestInit[] = [];
  const target: Record<string, any> = {};
  let stream: ReadableStreamDefaultController<Uint8Array> | undefined;
  let failed = false;
  target.top = target;
  target.addEventListener = (name: string, listener: typeof pagehide[number]) => {
    assert.equal(name, 'pagehide');
    pagehide.push(listener);
  };
  target.webkit = { messageHandlers: { webuiDesktopIpc: { postMessage(message: Record<string, unknown>) {
    if (message.kind === 'disconnect') return undefined;
    assert.equal(Reflect.ownKeys(message).length, 9);
    return Promise.resolve({
      kind: 'helloResult', callId: message.callId, navigation: message.navigation,
      documentNonce: message.documentNonce, challenge: message.challenge,
      generation: '1', token: 'a'.repeat(32), limits: defaultLimits,
    });
  } } } };
  runInNewContext(readFileSync('dist/native-bootstrap.js', 'utf8'), {
    window: target, crypto, TextEncoder, setTimeout, clearTimeout, Uint8Array,
  });
  const bootstrap = target.__webuiDesktopIpcV2 as NativeIpcBootstrap;
  const rejectWire = (error: unknown) => {
    if (failed) return;
    failed = true;
    order.push('fetch-rejected');
    if (stream) stream.error(error); else request.reject(error);
  };
  const transport = runtime.createDesktopTransport({ bootstrap, fetch: (async (_path, init) => {
    assert(init);
    requests.push(init);
    if (init?.method === 'POST' && point !== 'post') return new Response(null, { status: 204 });
    order.push('fetch-pending');
    init?.signal?.addEventListener('abort', () => rejectWire(new DOMException('Request aborted', 'AbortError')), { once: true });
    if (point !== 'body') { reading.resolve(); return request.promise; }
    return new Response(new ReadableStream<Uint8Array>({
      start(controller) { stream = controller; controller.enqueue(Uint8Array.of(8, 2)); },
      pull() { reading.resolve(); },
    }), { headers: { 'Content-Type': 'application/x-protobuf' } });
  }) as typeof fetch });
  return {
    runtime, transport, reading: reading.promise, order, requests, rejectWire,
    respond: request.resolve,
    activate() {
      assert.equal(bootstrap.activate({
        navigation: '1', documentNonce: bootstrap.documentNonce, challenge: 'b'.repeat(32),
      }), true);
    },
    ready() { target.__webuiDesktopIpcReceiveV2({ kind: 'ready', generation: '1' }); },
    retire(reason: 'navigated' | 'closed' | 'pagehide') {
      order.push(reason);
      if (reason === 'pagehide') for (const listener of pagehide) listener({ isTrusted: true, persisted: true });
      else target.__webuiDesktopIpcReceiveV2({ kind: 'closed', generation: '1', code: reason });
    },
  };
}

test('bodyless keepalive pulls remain owned and cancelled by document retirement', async () => {
  const h = await raceHarness('get');
  const starting = h.runtime.createConnection(schema, h.transport);
  h.activate();
  const connection = await starting;
  const rejected = assert.rejects(connection.call(save, item), { code: 'navigated' });
  h.ready();
  await h.reading;
  const pull = h.requests.find(request => request.method === 'GET');
  assert(pull);
  assert.equal(pull.keepalive, true);
  assert.equal(pull.body, undefined);
  assert.equal(pull.signal?.aborted, false);
  assert(h.requests.every(request => request.method !== 'POST' || request.keepalive !== true));
  const count = h.requests.length;
  h.retire('pagehide');
  await rejected;
  assert.equal((await connection.closed).code, 'navigated');
  assert.equal(pull.signal?.aborted, true);
  await turn();
  assert.equal(h.requests.length, count);
  assert.deepEqual(h.order, ['fetch-pending', 'pagehide', 'fetch-rejected']);
});

test('native retirement responses settle pending calls before the delayed closed control', async () => {
  for (const point of ['post', 'get'] as const) {
    for (const code of ['navigated', 'closed', 'transport'] as const) {
      const h = await raceHarness(point);
      const starting = h.runtime.createConnection(schema, h.transport);
      h.activate();
      const connection = await starting;
      let settlements = 0;
      const call = connection.call(save, item).catch(error => { settlements++; throw error; });
      const rejected = assert.rejects(call, { code });
      if (point === 'get') h.ready();
      await h.reading;
      h.respond(new Response(WireError.encode({
        code, message: 'Native IPC request rejected',
        help: 'Reload the trusted document and use the generated binary transport',
        applicationCode: '',
      }).finish(), {
        status: code === 'navigated' ? 409 : 503,
        headers: { 'Content-Type': 'application/x-protobuf', 'Cache-Control': 'no-store' },
      }));
      await rejected;
      const terminal = await connection.closed;
      assert.equal(terminal.code, code);
      h.retire('pagehide');
      await turn();
      assert.equal(await connection.closed, terminal);
      assert.equal(settlements, 1);
      connection.close();
    }
  }
});

test('an in-flight POST reports the known retirement reason instead of its resulting fetch abort', async () => {
  for (const reason of ['navigated', 'closed', 'pagehide'] as const) {
    const h = await raceHarness('post');
    const closed: IpcError[] = [];
    const starting = h.transport.start(schema, { async receive() {}, closed(error) { closed.push(error); } });
    h.activate();
    await starting;
    const expected = reason === 'pagehide' ? 'navigated' : reason;
    const bytes = IpcFrame.encode(invocation(1n, Kind.REQUEST, save.id)).finish();
    const rejected = assert.rejects(h.transport.send(bytes), { code: expected });
    await h.reading;
    h.retire(reason);
    await rejected;
    assert.deepEqual(h.order, ['fetch-pending', reason, 'fetch-rejected']);
    assert.equal(closed.length, 1);
    assert.equal(closed[0]!.code, expected);
    h.transport.close();
    assert.equal(closed.length, 1);
  }
});

test('trusted retirement wins exactly once for pending RPCs when native abort and close race', async () => {
  for (const point of ['get', 'body'] as const) {
    for (const reason of ['navigated', 'closed', 'pagehide'] as const) {
      const h = await raceHarness(point);
      const starting = h.runtime.createConnection(schema, h.transport);
      h.activate();
      const connection = await starting;
      const expected = reason === 'pagehide' ? 'navigated' : reason;
      let settlements = 0;
      const call = connection.call(save, item).catch(error => { settlements++; throw error; });
      const rejected = assert.rejects(call, { code: expected });
      h.ready();
      await h.reading;
      // The raw rejection is queued first, but the trusted close is known
      // before its promise reaction runs. No delay or retry is introduced.
      h.rejectWire(new TypeError('Load failed'));
      h.retire(reason);
      await rejected;
      assert.equal((await connection.closed).code, expected);
      await turn();
      assert.equal(settlements, 1);
      connection.close();
    }
  }
});

test('unannounced fetch failures stay Transport and cannot be rewritten by a later navigation', async () => {
  for (const point of ['post', 'get', 'body'] as const) {
    for (const error of [new TypeError('Load failed'), new DOMException('Request aborted', 'AbortError')]) {
      const h = await raceHarness(point);
      const starting = h.runtime.createConnection(schema, h.transport);
      h.activate();
      const connection = await starting;
      let settlements = 0;
      const call = connection.call(save, item).catch(error => { settlements++; throw error; });
      const rejected = assert.rejects(call, { code: 'transport' });
      h.ready();
      await h.reading;
      // No lifecycle signal is available before the failing operation settles.
      // This includes POST acceptance and streamed body reads, not just GET.
      h.rejectWire(error);
      const terminal = await connection.closed;
      assert.equal(terminal.code, 'transport');
      await rejected;
      h.retire('pagehide');
      await turn();
      assert.equal(await connection.closed, terminal);
      assert.equal(settlements, 1);
      connection.close();
    }
  }
});

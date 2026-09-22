// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import test from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { resolve } from 'node:path';
import { pathToFileURL } from 'node:url';
import { runInNewContext } from 'node:vm';
import { defaultLimits, type NativeIpcBootstrap, type SessionInfo, type IpcLimits, type IpcTransport } from '../src/index.js';
import { hasLedger, type ByteLedger } from '../src/budget.js';
import { schema, save, label, changed, item, itemCodec, frame, invocation, IpcFrame, Kind, deferred, turn } from './helpers.js';

type Runtime = typeof import('../src/index.js');
type LifecycleEvent = { isTrusted: boolean; persisted: boolean };
type Platform = 'webkit' | 'webview2';

async function loadHarness(platform: Platform, limits: IpcLimits = defaultLimits) {
  const runtime = await import(pathToFileURL(resolve('dist/desktop-runtime.js')).href) as Runtime;
  const events = new Map<string, Set<(event: LifecycleEvent) => void>>();
  const nativeListeners = new Set<(event: { data: unknown }) => void>();
  const messages: Record<string, unknown>[] = [];
  const grants: {
    message: Record<string, unknown>; info: SessionInfo;
    resolve(value: unknown): void; reject(error: unknown): void;
  }[] = [];
  const outbound: Uint8Array<ArrayBuffer>[] = [];
  const posts: IpcFrame[] = [];
  const target: Record<string, any> = {};
  let active: SessionInfo | undefined;
  let automaticResults = true;
  let gets = 0;
  const on = (name: string, listener: (event: LifecycleEvent) => void) => {
    const listeners = events.get(name) ?? new Set();
    listeners.add(listener);
    events.set(name, listeners);
  };
  const push = (message: Record<string, unknown>) => {
    if (message.kind === 'closed' && message.generation === active?.generation) active = undefined;
    if (platform === 'webkit') target.__webuiDesktopIpcReceiveV2(message);
    else for (const listener of nativeListeners) listener({ data: message });
  };
  const postMessage = (message: Record<string, unknown>) => {
    messages.push(message);
    if (message.kind === 'disconnect') {
      if (message.generation === active?.generation && message.token === active?.token) active = undefined;
      return undefined;
    }
    assert.equal(message.kind, 'hello');
    assert.deepEqual(Reflect.ownKeys(message).sort(), [
      'kind', 'callId', 'wireVersion', 'contractName', 'contractMajor', 'schemaHash',
      'navigation', 'documentNonce', 'challenge',
    ].sort());
    const pending = deferred<unknown>();
    const sequence = grants.length + 1;
    const info = { generation: String(sequence), token: sequence.toString(16).padStart(32, '0'), limits };
    active = info;
    grants.push({ message, info, resolve: pending.resolve, reject: pending.reject });
    return platform === 'webkit' ? pending.promise : undefined;
  };
  target.top = target;
  target.addEventListener = on;
  if (platform === 'webkit') target.webkit = { messageHandlers: { webuiDesktopIpc: { postMessage } } };
  else target.chrome = { webview: {
    postMessage,
    addEventListener(_name: string, listener: (event: { data: unknown }) => void) { nativeListeners.add(listener); },
    removeEventListener(_name: string, listener: (event: { data: unknown }) => void) { nativeListeners.delete(listener); },
  } };
  runInNewContext(readFileSync('dist/native-bootstrap.js', 'utf8'), {
    window: target, crypto, TextEncoder, setTimeout, clearTimeout, Uint8Array,
  });
  const bootstrap = target.__webuiDesktopIpcV2 as NativeIpcBootstrap;
  const fetcher = (async (_path, init) => {
    assert.equal(new Headers(init?.headers).get('X-WebUI-Ipc-Session'), active?.token);
    if (init?.method === 'POST') {
      assert(init.body instanceof Uint8Array);
      const request = IpcFrame.decode(init.body);
      posts.push(request);
      if (automaticResults && request.kind === Kind.REQUEST) {
        outbound.push(IpcFrame.encode({
          ...frame(request.generation, request.id, Kind.RESULT),
          body: { $case: 'payload', value: new Uint8Array() },
        }).finish());
        push({ kind: 'ready', generation: active!.generation });
      }
      return new Response(null, { status: 204 });
    }
    gets++;
    const bytes = outbound.shift();
    return bytes ? new Response(bytes, { headers: { 'Content-Type': 'application/x-protobuf' } })
      : new Response(null, { status: 204 });
  }) as typeof fetch;
  return {
    runtime, bootstrap, grants, messages, posts, on, push,
    get gets() { return gets; },
    currentSession: () => active,
    get listenerCount() { return nativeListeners.size; },
    set automaticResults(value: boolean) { automaticResults = value; },
    transport: () => runtime.createDesktopTransport({ bootstrap, fetch: fetcher }),
    emit(name: string, persisted = false, isTrusted = true) {
      for (const listener of events.get(name) ?? []) listener({ isTrusted, persisted });
    },
    activate(navigation: string) {
      const proof = { navigation, documentNonce: bootstrap.documentNonce, challenge: BigInt(navigation).toString(16).padStart(32, '0') };
      assert.equal(bootstrap.activate(proof), true);
      return proof;
    },
    reply(index: number) {
      const grant = grants[index]!;
      const result = {
        kind: 'helloResult', callId: grant.message.callId, navigation: grant.message.navigation,
        documentNonce: grant.message.documentNonce, challenge: grant.message.challenge, ...grant.info,
      };
      if (platform === 'webkit') grant.resolve(result); else push(result);
    },
    deliver(value: IpcFrame) {
      outbound.push(IpcFrame.encode(value).finish());
      push({ kind: 'ready', generation: active!.generation });
    },
  };
}

test('built bootstrap rotates before BFCache freeze; only explicit pageshow reconnect creates a new connection', async t => {
  const timers = t.mock.method(globalThis, 'setTimeout');
  for (const platform of ['webkit', 'webview2'] as const) {
    const h = await loadHarness(platform);
    const nonce = h.bootstrap.documentNonce;
    assert(Object.isFrozen(h.bootstrap));
    assert.equal(typeof Object.getOwnPropertyDescriptor(h.bootstrap, 'documentNonce')?.get, 'function');
    const unusedOldTransport = h.transport();
    const first = h.runtime.createConnection(schema, h.transport());
    const oldProof = h.activate('1');
    await turn();
    h.reply(0);
    const old = await first;
    t.after(() => old.close());
    let oldEvents = 0;
    const subscription = old.subscribe(changed, () => { oldEvents++; });
    h.emit('hashchange');
    h.emit('popstate');
    h.emit('pageshow', false);
    h.emit('pagehide', true, false);
    assert.equal(h.bootstrap.documentNonce, nonce);
    assert.equal(await old.call(save, item), undefined);
    h.automaticResults = false;
    const pending = assert.rejects(old.call(save, item), { code: 'navigated' });
    await turn();
    const postsBeforeHide = h.posts.length;
    const timerCount = timers.mock.callCount();
    h.emit('pagehide', true);
    assert.notEqual(h.bootstrap.documentNonce, nonce, 'fresh nonce exists before pageshow/native restore probe');
    assert.equal((await old.closed).code, 'navigated');
    await pending;
    assert.equal(h.currentSession(), undefined);
    assert.equal(timers.mock.callCount(), timerCount, 'preparing a cache epoch creates no idle timer');
    assert.equal(h.bootstrap.activate(oldProof), false);
    assert.equal(h.grants.length, 1);
    let reconnect: ReturnType<Runtime['createConnection']> | undefined;
    h.on('pageshow', event => {
      if (event.persisted) reconnect = h.runtime.createConnection(schema, h.transport());
    });
    h.activate('2'); // Native post-commit activation may precede pageshow.
    assert.equal(h.grants.length, 1);
    h.emit('pageshow', true); // Explicit application listener creates the new connection.
    await turn();
    h.push({ kind: 'closed', generation: '1', code: 'navigated' });
    h.reply(1);
    assert(reconnect);
    const current = await reconnect;
    t.after(() => current.close());
    assert.equal(h.grants.length, 2);
    assert.equal(h.posts.length, postsBeforeHide, 'no automatic RPC replay');
    assert.equal(h.posts.every(post => post.generation === 1n), true);
    h.bootstrap.disconnect('1', h.grants[0]!.info.token);
    assert.equal(h.currentSession()?.generation, '2');
    await assert.rejects(old.call(save, item), { code: 'navigated' });
    await assert.rejects(h.runtime.createConnection(schema, unusedOldTransport), { code: 'navigated' });
    subscription.close();
    let currentEvents = 0;
    current.subscribe(changed, () => { currentEvents++; });
    h.deliver({ ...invocation(1n, Kind.NOTIFY, changed.id), generation: 2n });
    await turn();
    assert.equal(oldEvents, 0);
    assert.equal(currentEvents, 1);
    h.automaticResults = true;
    assert.equal(await current.call(save, item), undefined);
    await turn();
    const gets = h.gets;
    const idleTimers = timers.mock.callCount();
    await turn();
    assert.equal(h.gets, gets);
    assert.equal(timers.mock.callCount(), idleTimers);
    if (platform === 'webview2') assert.equal(h.listenerCount, 1);
    current.close();
  }
});

test('old pending hello replies cannot admit or reject a restored epoch', async t => {
  for (const platform of ['webkit', 'webview2'] as const) {
    const h = await loadHarness(platform);
    const abandoned = h.transport();
    const first = h.runtime.createConnection(schema, h.transport());
    const rejected = assert.rejects(first, { code: 'navigated' });
    const oldProof = h.activate('1');
    await turn();
    h.emit('pagehide', true);
    await rejected;
    const next = h.runtime.createConnection(schema, h.transport());
    abandoned.close(); // Old local cleanup must not cancel the new pending hello.
    h.activate('2');
    await turn();
    let settled = false;
    void next.then(() => { settled = true; });
    h.reply(0);
    await turn();
    assert.equal(settled, false);
    assert.equal(h.bootstrap.activate(oldProof), false);
    h.reply(1);
    const current = await next;
    t.after(() => current.close());
    assert.equal(h.currentSession()?.generation, '2');
    assert.equal(await current.call(save, item), undefined);
    const retirements = h.messages.filter(message => message.kind === 'disconnect');
    if (platform === 'webkit') {
      assert.deepEqual(retirements.map(message => [message.generation, message.token]), [['1', h.grants[0]!.info.token]]);
    }
    current.close();
  }
});

test('pagehide still prepares a fresh cache epoch after native navigation already closed the old session', async t => {
  const h = await loadHarness('webkit');
  const first = h.runtime.createConnection(schema, h.transport());
  h.activate('1');
  await turn();
  h.reply(0);
  const old = await first;
  h.push({ kind: 'closed', generation: '1', code: 'navigated' });
  assert.equal((await old.closed).code, 'navigated');
  const nonce = h.bootstrap.documentNonce;
  h.emit('pagehide', true);
  assert.notEqual(h.bootstrap.documentNonce, nonce);
  const next = h.runtime.createConnection(schema, h.transport());
  h.activate('2');
  await turn();
  h.reply(1);
  const current = await next;
  t.after(() => current.close());
  assert.equal(await current.call(save, item), undefined);
});

test('non-persisted pagehide is terminal and pending activation is retired with Navigated', async () => {
  const h = await loadHarness('webkit');
  const nonce = h.bootstrap.documentNonce;
  const first = h.runtime.createConnection(schema, h.transport());
  const rejected = assert.rejects(first, { code: 'navigated' });
  h.emit('pagehide', false);
  await rejected;
  h.emit('pageshow', false);
  assert.equal(h.bootstrap.documentNonce, nonce);
  assert.equal(h.grants.length, 0);
  assert.equal(h.bootstrap.activate({ navigation: '1', documentNonce: nonce, challenge: 'a'.repeat(32) }), false);
  await assert.rejects(h.bootstrap.hello(schema), { code: 'not-ready' });
});

test('cached pending activation retires without a hello, then explicitly reconnects with fresh proof', async t => {
  const h = await loadHarness('webkit');
  const nonce = h.bootstrap.documentNonce;
  const first = h.runtime.createConnection(schema, h.transport());
  const rejected = assert.rejects(first, { code: 'navigated' });
  h.emit('pagehide', true);
  const next = h.runtime.createConnection(schema, h.transport());
  h.activate('2');
  await rejected;
  await turn();
  assert.notEqual(h.bootstrap.documentNonce, nonce);
  assert.equal(h.grants.length, 1, 'the retired unactivated hello was never posted');
  h.reply(0);
  const current = await next;
  t.after(() => current.close());
  assert.equal(await current.call(save, item), undefined);
});

test('a delayed native rejection belongs only to its retired hello epoch', async t => {
  const h = await loadHarness('webkit');
  const first = h.runtime.createConnection(schema, h.transport());
  const rejected = assert.rejects(first, { code: 'navigated' });
  h.activate('1');
  await turn();
  h.emit('pagehide', true);
  await rejected;
  const next = h.runtime.createConnection(schema, h.transport());
  h.activate('2');
  await turn();
  h.grants[0]!.reject(new Error('old native reply was cancelled'));
  await turn();
  h.reply(1);
  const current = await next;
  t.after(() => current.close());
  assert.equal(await current.call(save, item), undefined);
});

test('retired running handler credit stays charged across explicit cache-epoch reconnect', async t => {
  const h = await loadHarness('webkit', {
    ...defaultLimits, maxFrameBytes: 4096, maxAdmittedInputBytesPerFrame: 4096,
  });
  const ledgers: ByteLedger[] = [];
  const tracked = (): IpcTransport => {
    const transport = h.transport();
    return { ...transport, start(hello, receiver) {
      assert(hasLedger(receiver));
      ledgers.push(receiver.byteLedger);
      return transport.start(hello, receiver);
    } };
  };
  const started = deferred<void>();
  const blocked = deferred<typeof item>();
  const first = h.runtime.createConnection(schema, tracked(), { setup(connection) {
    connection.handle(label, () => { started.resolve(); return blocked.promise; });
  } });
  h.activate('1');
  await turn();
  h.reply(0);
  const old = await first;
  t.after(() => { blocked.resolve(item); old.close(); });
  h.deliver({
    ...invocation(1n, Kind.REQUEST, label.id),
    body: { $case: 'payload', value: itemCodec.encode({ ...item, data: new Uint8Array(3000) }) },
  });
  await started.promise;
  assert.equal(ledgers[0]!.input, 4096);
  h.emit('pagehide', true);
  assert.equal((await old.closed).code, 'navigated');
  const next = h.runtime.createConnection(schema, tracked());
  h.activate('2');
  await turn();
  h.reply(1);
  const current = await next;
  t.after(() => current.close());
  assert.equal(ledgers[0]!.input, 4096);
  assert.equal(ledgers[1]!.input, 4096);
  assert.throws(() => ledgers[1]!.reserveInput(1), { code: 'overloaded' });
  blocked.resolve(item);
  await turn();
  assert.equal(ledgers[0]!.input, 0);
  assert.equal(ledgers[1]!.input, 0);
  assert.equal(h.posts.length, 0, 'retired renderer result is not sent into the restored session');
  assert.equal(await current.call(save, item), undefined);
});

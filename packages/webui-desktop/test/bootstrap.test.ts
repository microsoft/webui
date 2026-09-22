// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import test from 'node:test';
import assert from 'node:assert/strict';
import { createNativeBootstrap, installNativeBootstrap, type NativeChannel } from '../src/bootstrap.js';
import { defaultLimits, type NativeControl, type NativeIpcBootstrap } from '../src/index.js';
import { deferred, schema, turn } from './helpers.js';

function channel(replyCapable: boolean) {
  let receive: ((value: unknown) => void) | undefined;
  const reply = deferred<unknown>();
  const sent: Readonly<Record<string, unknown>>[] = [];
  const channel: NativeChannel = {
    subscribe(callback) { receive = callback; return { close() { receive = undefined; } }; },
    postMessage(message) { sent.push(message); return replyCapable ? reply.promise : undefined; },
  };
  return { channel, reply, sent, push(value: unknown) { receive?.(value); }, listening: () => !!receive };
}
const result = { kind: 'helloResult', callId: '1', generation: '1', token: 'a'.repeat(32), limits: defaultLimits };
const proof = (bootstrap: NativeIpcBootstrap) => ({ navigation: '1', documentNonce: bootstrap.documentNonce, challenge: 'b'.repeat(32) });

test('bootstrap snapshots only Hello fields before appending the five native proof/control fields', async () => {
  const native = channel(false);
  const bootstrap = createNativeBootstrap(native.channel);
  const local = { ...schema, localCodec: () => {}, extra: 'not native data' };
  Object.defineProperty(local, 'metadata', { enumerable: true, get() { assert.fail('local metadata was read'); } });
  const hello = bootstrap.hello(local);
  bootstrap.activate(proof(bootstrap));
  await turn();
  assert.deepEqual(Reflect.ownKeys(native.sent[0]!).sort(), [
    'kind', 'callId', 'wireVersion', 'contractName', 'contractMajor', 'schemaHash',
    'navigation', 'documentNonce', 'challenge',
  ].sort());
  native.push({ ...result, ...proof(bootstrap) });
  await hello;
  bootstrap.disconnect('1', result.token);
});

test('WK/GTK reply and WebView2 correlated controls normalize to one Promise shape', async () => {
  for (const replyCapable of [true, false]) {
    const native = channel(replyCapable);
    const bootstrap = createNativeBootstrap(native.channel);
    const controls: NativeControl[] = [];
    bootstrap.subscribeControl(control => controls.push(control));
    const hello = bootstrap.hello(schema);
    assert.equal(native.sent.length, 0);
    bootstrap.activate(proof(bootstrap));
    await turn();
    assert.equal(native.sent[0]!.kind, 'hello');
    native.push({ kind: 'ready', generation: '1' });
    native.push({ ...result, callId: 'unrelated' });
    if (replyCapable) native.reply.resolve({ ...result, ...proof(bootstrap) }); else native.push({ ...result, ...proof(bootstrap) });
    assert.equal((await hello).generation, '1');
    assert.deepEqual(controls, [{ kind: 'ready', generation: '1' }]);
    await assert.rejects(bootstrap.hello(schema), { code: 'not-ready' });
    bootstrap.disconnect('1', result.token); bootstrap.disconnect('1', result.token);
    assert.equal(native.sent.length, 2);
    assert.equal(native.sent[1]!.kind, 'disconnect');
    assert.equal(native.sent[1]!.token, result.token);
    assert.equal(native.listening(), false);
  }
});

test('seam is frozen/non-replaceable, rejects conflict and supports one disposable listener', () => {
  const native = channel(false);
  const target = {};
  const bootstrap = installNativeBootstrap(target, native.channel);
  assert(Object.isFrozen(bootstrap));
  const property = Object.getOwnPropertyDescriptor(target, '__webuiDesktopIpcV2')!;
  assert.equal(property.writable, false);
  assert.equal(property.configurable, false);
  assert.throws(() => installNativeBootstrap(target, native.channel), { code: 'invalid-frame' });
  const first = bootstrap.subscribeControl(() => {});
  assert.throws(() => bootstrap.subscribeControl(() => {}), { code: 'invalid-frame' });
  first.close(); first.close();
  bootstrap.subscribeControl(() => {}).close();
  bootstrap.disconnect('0', '');
  assert.equal(native.listening(), false);
});

test('handshake deadline includes admission wait, removes native listener and ignores late reply', async t => {
  t.mock.timers.enable({ apis: ['setTimeout'] });
  const native = channel(false);
  const bootstrap = createNativeBootstrap(native.channel);
  const failed = assert.rejects(bootstrap.hello(schema), { code: 'deadline-exceeded' });
  t.mock.timers.tick(5000);
  await failed;
  assert.equal(bootstrap.activate(proof(bootstrap)), false);
  native.push(result);
  await turn();
  assert.equal(native.sent.length, 0);
  assert.equal(native.listening(), false);
});

test('native schema mismatch, rejection and malformed session fail explicitly', async () => {
  for (const reply of [
    { kind: 'helloResult', callId: '1', error: { code: 'schema-mismatch' } },
    { ...result, token: 'invalid' },
    { ...result, generation: '01' },
  ]) {
    const native = channel(true);
    const bootstrap = createNativeBootstrap(native.channel);
    const failed = assert.rejects(bootstrap.hello(schema));
    bootstrap.activate(proof(bootstrap));
    native.reply.resolve({ ...reply, ...proof(bootstrap) });
    await failed;
    assert.equal(native.listening(), false);
  }
  const native = channel(true);
  native.channel.postMessage = () => { throw new Error('native unavailable'); };
  const bootstrap = createNativeBootstrap(native.channel);
  const failed = assert.rejects(bootstrap.hello(schema), { code: 'transport' });
  bootstrap.activate(proof(bootstrap));
  await failed;
  assert.equal(native.listening(), false);
});

test('activation is nonce-bound; duplicate proof is harmless and stale helloResult cannot resolve', async () => {
  const native = channel(false);
  const bootstrap = createNativeBootstrap(native.channel);
  const other = createNativeBootstrap(channel(false).channel);
  assert.notEqual(bootstrap.documentNonce, other.documentNonce);
  assert.equal(bootstrap.documentNonce.length, 32);
  let settled = false;
  const hello = bootstrap.hello(schema).then(value => { settled = true; return value; });
  assert.equal(bootstrap.activate(proof(other)), false);
  await turn();
  assert.equal(native.sent.length, 0);
  assert.equal(bootstrap.activate(proof(bootstrap)), true);
  assert.equal(bootstrap.activate(proof(bootstrap)), true);
  assert.equal(bootstrap.activate({ ...proof(bootstrap), challenge: 'c'.repeat(32) }), false);
  await turn();
  assert.equal(native.sent.length, 1);
  native.push({ ...result, ...proof(other) });
  native.push({ ...result, ...proof(bootstrap), navigation: '2' });
  native.push({ ...result, ...proof(bootstrap), challenge: 'd'.repeat(32), error: { code: 'schema-mismatch' } });
  await turn();
  assert.equal(settled, false);
  native.push({ ...result, ...proof(bootstrap) });
  await hello;
  bootstrap.disconnect('1', result.token);
  other.disconnect('0', '');
});

test('disconnect during handshake rejects once and closes native subscription', async () => {
  const native = channel(false);
  const bootstrap = createNativeBootstrap(native.channel);
  const failed = assert.rejects(bootstrap.hello(schema), { code: 'closed' });
  bootstrap.disconnect('0', '');
  await failed;
  assert.equal(native.listening(), false);
});

test('generation alone, forged tokens and stale credentials cannot disconnect an admitted document', async () => {
  const native = channel(false);
  const bootstrap = createNativeBootstrap(native.channel);
  const hello = bootstrap.hello(schema);
  bootstrap.activate(proof(bootstrap));
  await turn();
  native.push({ ...result, ...proof(bootstrap) });
  await hello;
  bootstrap.disconnect('1', '');
  bootstrap.disconnect('1', 'd'.repeat(32));
  bootstrap.disconnect('2', result.token);
  bootstrap.disconnect('0', '');
  assert.equal(native.listening(), true);
  assert.equal(native.sent.length, 1);
  bootstrap.disconnect('1', result.token);
  assert.equal(native.listening(), false);
  assert.deepEqual(native.sent[1], { kind: 'disconnect', generation: '1', token: result.token });
});

test('concurrent duplicate hello cannot replace the pending proof or create a second native admission', async () => {
  const native = channel(false);
  const bootstrap = createNativeBootstrap(native.channel);
  const first = bootstrap.hello(schema);
  await assert.rejects(bootstrap.hello(schema), { code: 'not-ready' });
  bootstrap.activate(proof(bootstrap));
  await turn();
  assert.equal(native.sent.length, 1);
  native.push({ ...result, ...proof(bootstrap) });
  await first;
  bootstrap.disconnect('1', result.token);
});

test('cross-window proofs and credentials stay isolated even with equal generations and call IDs', async () => {
  const nativeA = channel(false);
  const nativeB = channel(false);
  const a = createNativeBootstrap(nativeA.channel);
  const b = createNativeBootstrap(nativeB.channel);
  const proofA = proof(a);
  const proofB = { ...proof(b), challenge: 'c'.repeat(32) };
  const tokenB = 'd'.repeat(32);
  const first = a.hello(schema);
  let settledB = false;
  const second = b.hello(schema).then(value => { settledB = true; return value; });
  a.activate(proofA);
  assert.equal(b.activate(proofA), false);
  b.activate(proofB);
  await turn();
  nativeA.push({ ...result, ...proofA });
  await first;
  nativeB.push({ ...result, ...proofA });
  await turn();
  assert.equal(settledB, false);
  nativeB.push({ ...result, ...proofB, token: tokenB });
  await second;
  b.disconnect('1', result.token);
  a.disconnect('1', tokenB);
  assert.equal(nativeA.listening(), true);
  assert.equal(nativeB.listening(), true);
  a.disconnect('1', result.token);
  assert.deepEqual(nativeA.sent[1], { kind: 'disconnect', generation: '1', token: result.token });
  assert.equal(nativeB.sent.length, 1);
  b.disconnect('1', tokenB);
  assert.deepEqual(nativeB.sent[1], { kind: 'disconnect', generation: '1', token: tokenB });
});

test('failed hello result validation retires only the matching authenticated session', async () => {
  const native = channel(false);
  const bootstrap = createNativeBootstrap(native.channel);
  const failed = assert.rejects(bootstrap.hello(schema), { code: 'invalid-frame' });
  bootstrap.activate(proof(bootstrap));
  await turn();
  native.push({ ...result, ...proof(bootstrap), generation: '7', limits: { ...defaultLimits, maxFrameBytes: 0 } });
  await failed;
  assert.deepEqual(native.sent[1], { kind: 'disconnect', generation: '7', token: result.token });
  native.push({ ...result, ...proof(bootstrap), generation: '8', token: 'e'.repeat(32) });
  assert.equal(native.sent.length, 2);
});

test('a reply after local hello cancellation retires only its captured proof and credentials', async () => {
  for (const matchingProof of [true, false]) {
    const native = channel(true);
    const bootstrap = createNativeBootstrap(native.channel);
    const activation = proof(bootstrap);
    const failed = assert.rejects(bootstrap.hello(schema), { code: 'closed' });
    bootstrap.activate(activation);
    await turn();
    bootstrap.disconnect('0', '');
    await failed;
    native.reply.resolve({
      ...result, ...activation, generation: '7',
      documentNonce: matchingProof ? activation.documentNonce : 'f'.repeat(32),
    });
    await turn();
    assert.equal(native.sent.length, matchingProof ? 2 : 1);
    if (matchingProof) assert.deepEqual(native.sent[1], { kind: 'disconnect', generation: '7', token: result.token });
    assert.equal(native.listening(), false);
  }
});

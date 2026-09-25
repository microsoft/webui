// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import test from 'node:test';
import { runInNewContext } from 'node:vm';

// Exercise the exact adapter validators, not a separately maintained JS copy.
const source = readFileSync(new URL('./ipc_control.rs', import.meta.url), 'utf8');
const marker = 'const BOUNDED_CONTROL: &str = r#"';
const start = source.indexOf(marker) + marker.length;
assert.ok(start >= marker.length);
const validate = runInNewContext(`(${source.slice(start, source.indexOf('"#;', start))})`);
const shared = readFileSync(new URL('../native_ipc.rs', import.meta.url), 'utf8');
function scriptTemplate(name) {
  const functionStart = shared.indexOf(`pub(crate) fn ${name}`);
  const scriptStart = shared.indexOf('"(()=>', functionStart);
  const scriptEnd = shared.indexOf('"', scriptStart + 1);
  assert.ok(scriptStart > functionStart);
  return JSON.parse(shared.slice(scriptStart, scriptEnd + 1))
    .replaceAll('{{', '{').replaceAll('}}', '}');
}
const template = scriptTemplate('activation_script');
const proof = { navigation: '7', documentNonce: 'a'.repeat(32), challenge: 'b'.repeat(32) };
const activation = template.replace('{proof}', JSON.stringify(proof));
const hello = () => ({
  kind: 'hello', wireVersion: 2, contractName: 'test', contractMajor: 1,
  schemaHash: 'c'.repeat(64), callId: '1', ...proof,
});

test('bounded native control records preserve valid hello and authenticated disconnect', () => {
  assert.deepEqual(JSON.parse(validate(hello())), hello());
  const disconnect = { kind: 'disconnect', generation: '1', token: 'd'.repeat(32) };
  assert.deepEqual(JSON.parse(validate(disconnect)), disconnect);
  assert.equal(validate({ kind: 'disconnect', generation: '1' }), null);
});

test('native control validation rejects oversize, malformed UTF-16 and object coercion', () => {
  for (const name of ['x'.repeat(257), '\ud800', '\udc00', 'a\ud800b']) {
    assert.equal(validate({ ...hello(), contractName: name }), null);
  }
  for (const value of [null, [], 'hello', { ...hello(), extra: true },
    { ...hello(), wireVersion: 2.5 }, { ...hello(), contractMajor: Infinity },
    { ...hello(), contractName: { toString() { throw new Error('must not coerce'); } } }]) {
    assert.equal(validate(value), null);
  }
  assert.notEqual(validate({ ...hello(), contractName: '😀' }), null);
});

test('a stale same-URL activation never invokes a replacement fake bootstrap', () => {
  let calls = 0;
  const bootstrap = { documentNonce: 'e'.repeat(32) };
  Object.defineProperty(bootstrap, 'activate', { get() { calls++; throw new Error('proof leaked'); } });
  assert.equal(runInNewContext(activation, { window: { __webuiDesktopIpcV2: bootstrap } }), false);
  assert.equal(calls, 0);
  assert.equal(runInNewContext(activation, { window: {} }), false);
});

test('matching activation delivers proof once and strict wrapper blocks caller inspection', () => {
  let received;
  const bootstrap = { documentNonce: proof.documentNonce, activate(value) { received = value; return true; } };
  assert.equal(runInNewContext(activation, { window: { __webuiDesktopIpcV2: bootstrap } }), true);
  assert.equal(JSON.stringify(received), JSON.stringify(proof));
  const hostile = runInNewContext(`({
    get documentNonce() {
      try {
        const caller = Object.getOwnPropertyDescriptor(this, 'documentNonce').get.caller;
        if (caller !== null) throw new Error('strict caller exposed');
      } catch (error) {
        if (!(error instanceof TypeError)) throw error;
      }
      return 'mismatch';
    },
    activate() { throw new Error('must not activate'); }
  })`);
  assert.equal(runInNewContext(activation, { window: { __webuiDesktopIpcV2: hostile } }), false);
});

test('delayed native navigation retirement reaches only its outgoing document epoch', () => {
  const control = { kind: 'closed', generation: '7', code: 'navigated' };
  const script = scriptTemplate('control_script')
    .replace('{nonce}', proof.documentNonce).replace('{push}', JSON.stringify(control));
  const received = [];
  const window = {
    __webuiDesktopIpcV2: { documentNonce: proof.documentNonce },
    __webuiDesktopIpcReceiveV2: value => received.push(value),
  };
  runInNewContext(script, { window });
  assert.equal(JSON.stringify(received), JSON.stringify([control]));
  window.__webuiDesktopIpcV2.documentNonce = 'e'.repeat(32);
  runInNewContext(script, { window });
  assert.equal(received.length, 1);
});

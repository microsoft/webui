// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import assert from 'node:assert/strict';
import { createRequire } from 'node:module';
import { test } from 'node:test';
import { fileURLToPath } from 'node:url';
import path from 'node:path';

const fixture = path.dirname(fileURLToPath(import.meta.url));
const require = createRequire(path.resolve(fixture, '../../../../../packages/webui-desktop/package.json'));
const { build } = require('esbuild');
const compiled = await build({
  entryPoints: [path.join(fixture, 'history-navigation.ts')],
  bundle: true,
  format: 'esm',
  platform: 'node',
  target: 'es2022',
  write: false,
});
const { HistoryNavigationHold, confirmHistoryNavigation } = await import(
  `data:text/javascript;base64,${Buffer.from(compiled.outputFiles[0].contents).toString('base64')}`
);

function harness(phase = 'history-back') {
  const entries = new Map();
  const store = {
    getItem: key => entries.get(key) ?? null,
    setItem: (key, value) => entries.set(key, value),
    removeItem: key => entries.delete(key),
  };
  const listeners = new Set();
  const page = {
    addEventListener(name, listener) {
      assert.equal(name, 'pagehide');
      listeners.add(listener);
    },
    removeEventListener(name, listener) {
      assert.equal(name, 'pagehide');
      listeners.delete(listener);
    },
  };
  const hold = new HistoryNavigationHold(phase, store, page);
  return { hold, store, hide: isTrusted => {
    for (const listener of [...listeners]) listener({ isTrusted });
  } };
}

test('WebKit cancellation between history request and trusted pagehide awaits native proof', () => {
  const { hold, store, hide } = harness();
  hold.navigate(() => {});
  assert.equal(hold.recordTransport(), true);
  assert.throws(() => confirmHistoryNavigation(store, 'history-back'), /not verified/);
  hide(false);
  assert.throws(() => confirmHistoryNavigation(store, 'history-back'), /not verified/);
  hide(true);
  confirmHistoryNavigation(store, 'history-back');
  assert.equal(store.getItem('native-ipc-history-cancellation'), null);
  hide(true);
  assert.equal(store.getItem('native-ipc-history-cancellation'), null);
});

test('a transport failure before history traversal cannot be accepted', () => {
  const { hold, store, hide } = harness();
  assert.equal(hold.recordTransport(), false);
  hide(true);
  assert.equal(store.getItem('native-ipc-history-cancellation'), null);
});

test('a different phase or old connection reason cannot reuse cancellation proof', () => {
  const { hold, store, hide } = harness();
  hold.navigate(() => {});
  assert.equal(hold.recordTransport(), true);
  hide(true);
  assert.throws(() => confirmHistoryNavigation(store, 'history-return'), /not verified/);
  assert.throws(() => confirmHistoryNavigation(store, 'history-back', 'navigated'), /not verified/);
  confirmHistoryNavigation(store, 'history-back', 'transport');
});

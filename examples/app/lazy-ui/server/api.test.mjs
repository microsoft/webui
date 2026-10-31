// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.

import assert from 'node:assert/strict';
import { after, before, test } from 'node:test';
import { once } from 'node:events';
import { createStateServer } from './api.mjs';

const server = createStateServer();
let origin;
before(async () => {
  server.listen(0, '127.0.0.1');
  await once(server, 'listening');
  origin = `http://127.0.0.1:${server.address().port}`;
});
after(async () => {
  const closed = new Promise((resolve, reject) => server.close(error => error ? reject(error) : resolve()));
  server.closeAllConnections();
  await closed;
});

test('the initial state contains only the shell and local view preferences', async () => {
  const { state } = await (await fetch(origin)).json();
  assert.equal(state.activityReady, false);
  assert.equal(state.settingsReady, false);
  assert.equal(state.documents.length, 3);
  assert.equal(state.activityItems, undefined);
  assert.equal(state.layoutOptions, undefined);
});

test('each region has fresh scoped state rather than a shared form state cache', async () => {
  const first = await (await fetch(origin + '/api/activity')).json();
  const second = await (await fetch(origin + '/api/activity')).json();
  assert.notEqual(first.state.activityStamp, second.state.activityStamp);
  assert.equal(first.state.activityItems.length, 3);
  assert.equal(first.state.layoutOptions, undefined);
  const settings = await (await fetch(origin + '/api/settings')).json();
  assert.equal(settings.state.layoutOptions.length, 2);
  assert.equal(settings.state.activityItems, undefined);
});

test('an explicitly active SSR body gets its data in the initial state', async () => {
  const { state } = await (await fetch(origin + '/?open=dialog')).json();
  assert.equal(state.settingsReady, true);
  assert.equal(state.layoutOptions.length, 2);
  assert.equal(state.activityReady, false);
});

test('unknown endpoints and writes are rejected', async () => {
  assert.equal((await fetch(origin + '/api/missing')).status, 404);
  assert.equal((await fetch(origin + '/_webui/lazy/state?owner=lazy-popover&id=activity')).status, 404);
  assert.equal((await fetch(origin + '/api/settings', { method: 'POST' })).status, 405);
  assert.equal((await fetch(origin, { method: 'POST' })).status, 405);
});

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
  const activity = '/_webui/deferred/state?owner=deferred-popover&id=activity';
  const first = await (await fetch(origin + activity)).json();
  const second = await (await fetch(origin + activity)).json();
  assert.notEqual(first.state.activityStamp, second.state.activityStamp);
  assert.equal(first.state.activityItems.length, 3);
  assert.equal(first.state.layoutOptions, undefined);
  const settings = await (await fetch(origin + '/_webui/deferred/state?owner=deferred-dialog&id=settings')).json();
  assert.equal(settings.state.layoutOptions.length, 2);
  assert.equal(settings.state.activityItems, undefined);
});

test('an explicitly active SSR body gets its data in the initial state', async () => {
  const { state } = await (await fetch(origin + '/?open=dialog')).json();
  assert.equal(state.settingsReady, true);
  assert.equal(state.layoutOptions.length, 2);
  assert.equal(state.activityReady, false);
});

test('unknown owners, mismatched regions, and writes are rejected', async () => {
  assert.equal((await fetch(origin + '/_webui/deferred/state?owner=deferred-popover&id=missing')).status, 404);
  assert.equal((await fetch(origin + '/_webui/deferred/state?owner=another-owner&id=settings')).status, 404);
  assert.equal((await fetch(origin + '/_webui/deferred/state?owner=deferred-workspace&id=activity')).status, 404);
  assert.equal((await fetch(origin + '/_webui/deferred/state?owner=deferred-popover&id=settings')).status, 404);
  assert.equal((await fetch(origin + '/_webui/deferred/state?owner=deferred-dialog&id=activity')).status, 404);
  assert.equal((await fetch(origin, { method: 'POST' })).status, 405);
});
